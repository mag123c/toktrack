//! Cline parser
//!
//! Cline 4.x (VS Code extension, CLI and SDK share one store) writes one
//! directory per session under `~/.cline/data/sessions/<id>/`:
//!
//! ```text
//! <id>.messages.json   { "messages": [ { id, role, ts, modelInfo, metrics }, ... ] }
//! <id>.json            session record: started_at, provider, model, cwd, ...
//! ```
//!
//! Usage lives on each assistant message as `metrics`:
//! `{inputTokens, outputTokens, cacheReadTokens, cacheWriteTokens, cost}`.
//! One message is one API call, so messages are summed individually; the
//! session record's `metadata.usage` is the same numbers pre-aggregated and is
//! deliberately not read (it would also lose per-day attribution).
//!
//! Token nesting, proven on real sessions: `inputTokens` **includes** both
//! cache reads and cache writes. The session's own non-cached `tokensIn`
//! equals `Σ(inputTokens − cacheReadTokens − cacheWriteTokens)` exactly, so
//! that difference is the v2 `input_tokens`. Cline reports no reasoning count.
//!
//! Compaction strips `metrics` from the messages it folds and sums them into
//! the first message's `metadata.usageBefore` (same five fields, same nesting;
//! repeated compactions add into it). It becomes one extra entry per such
//! message, id `<id>#usageBefore`, dated at the session's `started_at` because
//! the aggregate has no per-day split.
//!
//! Subagent and team-task files are `<root>/<agentId>.messages.json` with no
//! record of their own: they use the root's `<root>/<root>.json` (but not its
//! fork cutoff, as the running session creates them) and a `<root>/<stem>`
//! request id so repeated agent ids cannot collide in dedup.
//!
//! Fork / checkpoint-restore sessions are seeded with the parent's messages and
//! imported sessions carry another tool's usage, both with metrics intact. The
//! session record's `metadata.fork.forkedAt` / `metadata.importedFrom.importedAt`
//! mark where new usage starts; earlier messages are skipped.
//!
//! The legacy VS Code `globalStorage/saoudrizwan.claude-dev/tasks/` layout is
//! not read.

use crate::types::{Result, ToktrackError, UsageEntry};
use chrono::{DateTime, TimeZone, Utc};
use serde::Deserialize;
use std::borrow::Cow;
use std::path::{Path, PathBuf};

use super::CLIParser;

const MESSAGES_SUFFIX: &str = ".messages.json";

/// `<id>.messages.json`. Only `messages` is read; `system_prompt` and every
/// message's `content` are skipped by serde without being materialized.
#[derive(Deserialize)]
struct ClineMessagesFile<'a> {
    #[serde(default, deserialize_with = "lenient_u64")]
    version: Option<u64>,
    #[serde(default, borrow)]
    messages: Vec<ClineMessage<'a>>,
}

#[derive(Deserialize)]
struct ClineMessage<'a> {
    #[serde(default, borrow)]
    id: Option<Cow<'a, str>>,
    /// Epoch milliseconds.
    #[serde(default, deserialize_with = "lenient_i64")]
    ts: Option<i64>,
    #[serde(default, borrow, rename = "modelInfo")]
    model_info: Option<ClineModelInfo<'a>>,
    #[serde(default)]
    metrics: Option<ClineMetrics>,
    #[serde(default, deserialize_with = "lenient_message_metadata")]
    metadata: Option<ClineMessageMetadata>,
}

/// Only `usageBefore` is read from a message's `metadata`.
#[derive(Deserialize)]
struct ClineMessageMetadata {
    #[serde(default, rename = "usageBefore", deserialize_with = "lenient_metrics")]
    usage_before: Option<ClineMetrics>,
}

#[derive(Deserialize)]
struct ClineModelInfo<'a> {
    #[serde(default, borrow)]
    id: Option<Cow<'a, str>>,
    #[serde(default, borrow)]
    provider: Option<Cow<'a, str>>,
}

/// A number-ish JSON value of any shape; anything else is ignored.
#[derive(Deserialize)]
#[serde(untagged)]
enum Lenient {
    Int(i64),
    Uint(u64),
    Float(f64),
    Other(serde::de::IgnoredAny),
}

impl Lenient {
    fn as_f64(&self) -> Option<f64> {
        match *self {
            Lenient::Int(v) => Some(v as f64),
            Lenient::Uint(v) => Some(v as f64),
            Lenient::Float(v) if v.is_finite() => Some(v),
            _ => None,
        }
    }
}

/// Token counts: integers pass through; floats truncate; negatives, strings
/// and other shapes become `None` instead of failing the whole file.
fn lenient_u64<'de, D: serde::Deserializer<'de>>(
    d: D,
) -> std::result::Result<Option<u64>, D::Error> {
    Ok(match Option::<Lenient>::deserialize(d)? {
        Some(Lenient::Uint(v)) => Some(v),
        Some(Lenient::Int(v)) => u64::try_from(v).ok(),
        Some(l @ Lenient::Float(_)) => l.as_f64().filter(|f| *f >= 0.0).map(|f| f as u64),
        _ => None,
    })
}

/// Timestamps: a non-numeric `ts` falls back to the session start.
fn lenient_i64<'de, D: serde::Deserializer<'de>>(
    d: D,
) -> std::result::Result<Option<i64>, D::Error> {
    Ok(match Option::<Lenient>::deserialize(d)? {
        Some(Lenient::Int(v)) => Some(v),
        Some(Lenient::Uint(v)) => i64::try_from(v).ok(),
        Some(l @ Lenient::Float(_)) => l.as_f64().map(|f| f as i64),
        _ => None,
    })
}

/// Cost: any finite number; everything else stays unknown.
fn lenient_f64<'de, D: serde::Deserializer<'de>>(
    d: D,
) -> std::result::Result<Option<f64>, D::Error> {
    Ok(Option::<Lenient>::deserialize(d)?.and_then(|l| l.as_f64()))
}

/// A message `metadata` object, or `None` for any other shape.
fn lenient_message_metadata<'de, D: serde::Deserializer<'de>>(
    d: D,
) -> std::result::Result<Option<ClineMessageMetadata>, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Shape {
        Meta(ClineMessageMetadata),
        Other(serde::de::IgnoredAny),
    }
    Ok(match Option::<Shape>::deserialize(d)? {
        Some(Shape::Meta(m)) => Some(m),
        _ => None,
    })
}

/// A metrics object, or `None` for any other shape instead of failing the file.
fn lenient_metrics<'de, D: serde::Deserializer<'de>>(
    d: D,
) -> std::result::Result<Option<ClineMetrics>, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Shape {
        Metrics(ClineMetrics),
        Other(serde::de::IgnoredAny),
    }
    Ok(match Option::<Shape>::deserialize(d)? {
        Some(Shape::Metrics(m)) => Some(m),
        _ => None,
    })
}

/// Fields are optional and lenient: a `null`, float, negative or string value
/// drops only that field, never the whole file.
#[derive(Deserialize, Default, Clone, Copy)]
#[serde(rename_all = "camelCase")]
struct ClineMetrics {
    #[serde(default, deserialize_with = "lenient_u64")]
    input_tokens: Option<u64>,
    #[serde(default, deserialize_with = "lenient_u64")]
    output_tokens: Option<u64>,
    #[serde(default, deserialize_with = "lenient_u64")]
    cache_read_tokens: Option<u64>,
    #[serde(default, deserialize_with = "lenient_u64")]
    cache_write_tokens: Option<u64>,
    /// `None` when absent: a missing figure must stay unknown rather than
    /// become a confident $0.00 that suppresses the estimated-cost fallback.
    #[serde(default, deserialize_with = "lenient_f64")]
    cost: Option<f64>,
}

/// `<id>.json`, read once per session for fallbacks and project attribution.
#[derive(Deserialize, Default)]
struct ClineSessionRecord {
    #[serde(default)]
    started_at: Option<String>,
    #[serde(default)]
    provider: Option<String>,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    cwd: Option<String>,
    #[serde(default)]
    workspace_root: Option<String>,
    #[serde(default)]
    metadata: Option<ClineSessionMetadata>,
}

/// Markers of sessions seeded with another session's messages.
#[derive(Deserialize, Default)]
struct ClineSessionMetadata {
    #[serde(default)]
    fork: Option<ClineSeedMarker>,
    #[serde(default, rename = "importedFrom")]
    imported_from: Option<ClineSeedMarker>,
}

/// `fork.forkedAt` / `importedFrom.importedAt` (ISO strings).
#[derive(Deserialize, Default)]
struct ClineSeedMarker {
    #[serde(default, rename = "forkedAt")]
    forked_at: Option<String>,
    #[serde(default, rename = "importedAt")]
    imported_at: Option<String>,
}

impl ClineSessionRecord {
    /// Messages stamped before this instant were copied in from a parent
    /// session (fork / checkpoint restore) or another tool (import). Their
    /// usage is already counted under the original session or source.
    fn seed_cutoff(&self) -> Option<DateTime<Utc>> {
        let meta = self.metadata.as_ref()?;
        let parse = |s: &Option<String>| {
            s.as_deref()
                .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
                .map(|dt| dt.with_timezone(&Utc))
        };
        let forked = meta.fork.as_ref().and_then(|m| parse(&m.forked_at));
        let imported = meta
            .imported_from
            .as_ref()
            .and_then(|m| parse(&m.imported_at));
        forked.max(imported)
    }
}

/// v2 token fields derived from one message's `metrics`.
struct MappedTokens {
    input_tokens: u64,
    reported_total_tokens: Option<u64>,
    /// The cache counts exceeded `inputTokens`, contradicting the nesting.
    drifted: bool,
}

/// Map Cline's nested input onto the v2 contract (see module docs).
///
/// When cache reads + writes exceed `inputTokens` the nesting assumption is
/// contradicted. Rather than clamp input to zero (silently under-counting),
/// treat `inputTokens` as already non-cached and withdraw the reconciliation
/// total, which exists only to be compared against.
fn map_tokens(m: &ClineMetrics) -> MappedTokens {
    let input = m.input_tokens.unwrap_or(0);
    let output = m.output_tokens.unwrap_or(0);
    // Saturating: numbers from disk are untrusted, and a panic inside the
    // rayon pool takes every other source down with it.
    let cached = m
        .cache_read_tokens
        .unwrap_or(0)
        .saturating_add(m.cache_write_tokens.unwrap_or(0));
    if cached <= input {
        MappedTokens {
            input_tokens: input - cached,
            reported_total_tokens: Some(input.saturating_add(output)),
            drifted: false,
        }
    } else {
        MappedTokens {
            input_tokens: input,
            reported_total_tokens: None,
            drifted: true,
        }
    }
}

/// `anthropic/claude-sonnet-5.5` → `claude-sonnet-5.5`. Gateways such as
/// OpenRouter prefix the upstream vendor; pricing lookups skip `/`-keys and
/// the bare id rolls up with the same model from other sources.
fn strip_provider_prefix(model: &str) -> &str {
    model.rsplit('/').next().unwrap_or(model)
}

fn non_empty(s: Option<&str>) -> Option<&str> {
    s.map(str::trim).filter(|s| !s.is_empty())
}

fn has_usage(m: &ClineMetrics) -> bool {
    m.input_tokens.unwrap_or(0) > 0
        || m.output_tokens.unwrap_or(0) > 0
        || m.cache_read_tokens.unwrap_or(0) > 0
        || m.cache_write_tokens.unwrap_or(0) > 0
        || m.cost.unwrap_or(0.0) != 0.0
}

/// Parser for Cline usage data
pub struct ClineParser {
    /// The sessions directory: `CLINE_SESSION_DATA_DIR`, else `<data>/sessions`.
    sessions_dir: PathBuf,
}

impl ClineParser {
    /// Create a parser with the default data directory.
    ///
    /// Honors `CLINE_SESSION_DATA_DIR` (the sessions directory itself), then
    /// `CLINE_DATA_DIR` (replaces `~/.cline/data`), then `CLINE_DIR`
    /// (replaces `~/.cline`, data under `data/`) — the upstream variables.
    pub fn new() -> Self {
        let sessions_dir = super::discovery::first_env_dir(&["CLINE_SESSION_DATA_DIR"])
            .or_else(|| {
                super::discovery::first_env_dir(&["CLINE_DATA_DIR"])
                    .or_else(|| {
                        super::discovery::first_env_dir(&["CLINE_DIR"]).map(|d| d.join("data"))
                    })
                    .or_else(|| {
                        directories::BaseDirs::new()
                            .map(|d| d.home_dir().join(".cline").join("data"))
                    })
                    .map(|d| d.join("sessions"))
            })
            .unwrap_or_else(|| {
                eprintln!("[toktrack] Warning: Could not determine home directory");
                PathBuf::from(".")
            });
        Self { sessions_dir }
    }

    /// Create a parser rooted at a custom data directory (for testing).
    #[allow(dead_code)] // Used in tests
    pub fn with_data_dir(data_dir: PathBuf) -> Self {
        Self {
            sessions_dir: data_dir.join("sessions"),
        }
    }

    /// Session id from `<id>.messages.json`.
    fn session_id(path: &Path) -> Option<String> {
        let name = path.file_name()?.to_str()?;
        name.strip_suffix(MESSAGES_SUFFIX).map(String::from)
    }

    /// Name of the session directory holding `path`: the root session id.
    fn root_id(path: &Path) -> Option<&str> {
        path.parent()?.file_name()?.to_str()
    }

    /// The session record `<id>.json` for the root session id `id`, or
    /// defaults when absent or unreadable — it only supplies fallbacks, so
    /// losing it must not drop the usage.
    fn session_record(path: &Path, id: Option<&str>) -> ClineSessionRecord {
        let Some(id) = id else {
            return ClineSessionRecord::default();
        };
        let Ok(mut bytes) = std::fs::read(path.with_file_name(format!("{id}.json"))) else {
            return ClineSessionRecord::default();
        };
        simd_json::from_slice(&mut bytes).unwrap_or_default()
    }

    /// `parse_file`'s body, additionally returning how many usage-bearing
    /// messages were dropped (no resolvable timestamp), so it is testable.
    fn parse_entries(&self, path: &Path) -> Result<(Vec<UsageEntry>, usize)> {
        let mut bytes = std::fs::read(path).map_err(ToktrackError::Io)?;
        let parsed: ClineMessagesFile = simd_json::from_slice(&mut bytes)
            .map_err(|e| ToktrackError::Parse(format!("Cline messages {:?}: {}", path, e)))?;

        if let Some(v) = parsed.version.filter(|v| *v != 1) {
            eprintln!(
                "[toktrack] Warning: Cline messages {:?} use format version {} (expected 1); \
                 token fields may have changed",
                path, v
            );
        }

        let session_id = Self::session_id(path);
        // Subagent / team-task files sit beside the root's as
        // `<root>/<agentId>.messages.json` and have no record of their own, so
        // they take the root's. They are created by the running session, never
        // seeded from a parent, so the root's fork/import cutoff is not theirs.
        // A file with its own `<stem>.json` (e.g. a renamed copy of a root) is
        // a session in its own right; a child needs the root's record to exist.
        let root_id = Self::root_id(path);
        let is_child = match (root_id, session_id.as_deref()) {
            (Some(r), Some(s)) if r != s => {
                !path.with_file_name(format!("{s}.json")).exists()
                    && path.with_file_name(format!("{r}.json")).exists()
            }
            _ => false,
        };
        let root_id = if is_child { root_id } else { None };
        let record = Self::session_record(path, root_id.or(session_id.as_deref()));
        let request_id = match (is_child, root_id, session_id.as_deref()) {
            // Agent ids can repeat across roots; keep the dedup key unique.
            (true, Some(root), Some(stem)) => Some(format!("{root}/{stem}")),
            _ => session_id.clone(),
        };
        let project = non_empty(record.cwd.as_deref())
            .or_else(|| non_empty(record.workspace_root.as_deref()))
            .map(String::from);
        let fallback_ts = record
            .started_at
            .as_deref()
            .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
            .map(|dt| dt.with_timezone(&Utc));

        let seed_cutoff = if is_child { None } else { record.seed_cutoff() };

        let mut entries = Vec::new();
        let mut dropped = 0usize;
        let mut drifted = 0usize;

        for (index, msg) in parsed.messages.iter().enumerate() {
            let own_ts = msg.ts.and_then(|ms| Utc.timestamp_millis_opt(ms).single());
            if matches!((own_ts, seed_cutoff), (Some(ts), Some(cutoff)) if ts < cutoff) {
                continue;
            }

            // Compaction strips the folded messages' `metrics` and sums them
            // into the first message's `metadata.usageBefore` (repeated
            // compactions add into the same field), so each call is counted in
            // exactly one of the two places. The aggregate has no per-day
            // split; it is dated at the session start, which is stable across
            // re-compactions (see .dev/DECISIONS.md).
            let own = msg.metrics.filter(has_usage);
            let aggregate = msg
                .metadata
                .as_ref()
                .and_then(|m| m.usage_before)
                .filter(has_usage);
            for (metrics, ts, is_aggregate) in [
                (own, own_ts, false),
                (aggregate, fallback_ts.or(own_ts), true),
            ] {
                let Some(metrics) = metrics else {
                    continue;
                };
                let Some(timestamp) = ts.or(fallback_ts) else {
                    dropped += 1;
                    continue;
                };

                let info = msg.model_info.as_ref();
                let model = non_empty(info.and_then(|i| i.id.as_deref()))
                    .or_else(|| non_empty(record.model.as_deref()))
                    .map(|m| strip_provider_prefix(m).to_string());
                let provider = non_empty(info.and_then(|i| i.provider.as_deref()))
                    .or_else(|| non_empty(record.provider.as_deref()))
                    .map(String::from);

                let mapped = map_tokens(&metrics);
                if mapped.drifted {
                    drifted += 1;
                }

                entries.push(UsageEntry {
                    fast_speed: false,
                    timestamp,
                    model,
                    input_tokens: mapped.input_tokens,
                    output_tokens: metrics.output_tokens.unwrap_or(0),
                    cache_read_tokens: metrics.cache_read_tokens.unwrap_or(0),
                    cache_creation_tokens: metrics.cache_write_tokens.unwrap_or(0),
                    reasoning_tokens: 0,
                    cache_creation_5m_tokens: 0,
                    cache_creation_1h_tokens: 0,
                    web_search_requests: 0,
                    web_fetch_requests: 0,
                    reported_total_tokens: mapped.reported_total_tokens,
                    cost_usd: metrics.cost,
                    // Message ids are unique within a session; pairing with the
                    // session id keeps the dedup hash unique across sessions.
                    message_id: match (is_aggregate, msg.id.as_deref()) {
                        (false, id) => id.map(String::from),
                        (true, Some(id)) => Some(format!("{id}#usageBefore")),
                        (true, None) => Some(format!("@{index}#usageBefore")),
                    },
                    request_id: request_id.clone(),
                    source: Some("cline".into()),
                    provider,
                    project: project.clone(),
                });
            }
        }

        if drifted > 0 {
            eprintln!(
                "[toktrack] Warning: {} Cline message(s) in {:?} report cache tokens above \
                 inputTokens, contradicting the documented nesting; their input is counted \
                 as non-cached and the reconciliation total is dropped",
                drifted, path
            );
        }

        Ok((entries, dropped))
    }
}

impl Default for ClineParser {
    fn default() -> Self {
        Self::new()
    }
}

impl CLIParser for ClineParser {
    fn name(&self) -> &str {
        "cline"
    }

    fn data_dir(&self) -> &Path {
        &self.sessions_dir
    }

    fn file_pattern(&self) -> &str {
        "*/*.messages.json"
    }

    fn parse_file(&self, path: &Path) -> Result<Vec<UsageEntry>> {
        let (entries, dropped) = self.parse_entries(path)?;
        // Dropped messages are spent usage missing from the totals; say so
        // once per file rather than losing them silently.
        if dropped > 0 {
            eprintln!(
                "[toktrack] Warning: {} Cline message(s) in {:?} have no timestamp and no \
                 session start time; their tokens and cost are missing from the totals",
                dropped, path
            );
        }
        Ok(entries)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Utc};
    use serde::Deserialize;

    fn root() -> PathBuf {
        PathBuf::from("tests/fixtures/cline")
    }

    fn parser() -> ClineParser {
        ClineParser::with_data_dir(root())
    }

    fn session_file(id: &str) -> PathBuf {
        root()
            .join("sessions")
            .join(id)
            .join(format!("{id}.messages.json"))
    }

    fn by_id<'a>(entries: &'a [UsageEntry], id: &str) -> &'a UsageEntry {
        entries
            .iter()
            .find(|e| e.message_id.as_deref() == Some(id))
            .unwrap_or_else(|| panic!("no entry for message {id}"))
    }

    #[test]
    fn test_parser_name_and_pattern() {
        let p = parser();
        assert_eq!(p.name(), "cline");
        assert_eq!(p.file_pattern(), "*/*.messages.json");
    }

    #[test]
    fn test_collect_files_finds_every_session_messages_file() {
        let mut files = parser().collect_files();
        files.sort();
        let names: Vec<String> = files
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            names,
            vec![
                "1792996779409_aaaaa.messages.json",
                "1793069169145_bbbbb.messages.json",
                "1793100000000_ccccc.messages.json",
                "1793200000000_ddddd.messages.json",
                "1793300000000_eeeee.messages.json",
                "1793400000000_fffff.messages.json",
                "1793500000000_ggggg.messages.json",
                "1793600000000_hhhhh.messages.json",
                "1793700000000_iiiii.messages.json",
                "1793800000000_jjjjj.messages.json",
                "1793900000000_kkkkk.messages.json",
                "agent_abc.messages.json",
            ],
            "session records (<id>.json) must not be collected as message files"
        );
    }

    /// Token values are verbatim from a real session: Cline's `inputTokens`
    /// includes cache reads and writes, so the non-cached remainder must sum
    /// to the session's own `tokensIn` (6453).
    #[test]
    fn test_real_session_maps_nested_input_to_non_cached() {
        let entries = parser()
            .parse_file(&session_file("1792996779409_aaaaa"))
            .unwrap();
        assert_eq!(entries.len(), 3);

        let first = by_id(&entries, "msg_real0001");
        assert_eq!(first.input_tokens, 4254);
        assert_eq!(first.output_tokens, 1423);
        assert_eq!(first.cache_read_tokens, 0);
        assert_eq!(first.reasoning_tokens, 0);

        let second = by_id(&entries, "msg_real0002");
        assert_eq!(second.input_tokens, 1441);
        assert_eq!(second.cache_read_tokens, 4250);

        let non_cached: u64 = entries.iter().map(|e| e.input_tokens).sum();
        assert_eq!(non_cached, 6453, "must equal the session's tokensIn");
        let cache_read: u64 = entries.iter().map(|e| e.cache_read_tokens).sum();
        assert_eq!(cache_read, 9937);
    }

    #[test]
    fn test_entry_metadata_fields() {
        let entries = parser()
            .parse_file(&session_file("1792996779409_aaaaa"))
            .unwrap();
        let e = by_id(&entries, "msg_real0001");
        assert_eq!(
            e.timestamp,
            Utc.timestamp_millis_opt(1792996792728).unwrap()
        );
        assert_eq!(e.model.as_deref(), Some("claude-sonnet-5.5"));
        assert_eq!(e.provider.as_deref(), Some("openrouter"));
        assert_eq!(e.source.as_deref(), Some("cline"));
        assert_eq!(e.request_id.as_deref(), Some("1792996779409_aaaaa"));
        assert_eq!(e.project.as_deref(), Some("/home/me/proj-alpha"));
        assert_eq!(e.cost_usd, Some(0.023558000000000003));
    }

    #[test]
    fn test_cost_sums_to_session_total() {
        let entries = parser()
            .parse_file(&session_file("1792996779409_aaaaa"))
            .unwrap();
        let total: f64 = entries.iter().filter_map(|e| e.cost_usd).sum();
        assert!((total - 0.0474297).abs() < 1e-9, "got {total}");
    }

    #[test]
    fn test_cache_write_is_carved_out_of_input() {
        let entries = parser()
            .parse_file(&session_file("1793069169145_bbbbb"))
            .unwrap();
        let e = by_id(&entries, "msg_cachewrite");
        assert_eq!(e.input_tokens, 4);
        assert_eq!(e.cache_creation_tokens, 4257);
        assert_eq!(e.reported_total_tokens, Some(4261 + 953));
    }

    /// A missing upstream cost must stay unknown so pricing falls back to an
    /// estimate, rather than being reported as an exact $0.00.
    #[test]
    fn test_missing_cost_stays_none() {
        let entries = parser()
            .parse_file(&session_file("1793069169145_bbbbb"))
            .unwrap();
        let e = by_id(&entries, "msg_nocost");
        assert_eq!(e.cost_usd, None);
        assert_eq!(e.input_tokens, 100);
    }

    #[test]
    fn test_missing_model_info_and_ts_fall_back_to_session_record() {
        let entries = parser()
            .parse_file(&session_file("1793069169145_bbbbb"))
            .unwrap();
        let e = by_id(&entries, "msg_nomodelinfo");
        assert_eq!(e.model.as_deref(), Some("claude-opus-5.5"));
        assert_eq!(e.provider.as_deref(), Some("anthropic"));
        assert_eq!(
            e.timestamp,
            chrono::DateTime::parse_from_rfc3339("2026-10-27T02:46:09.211Z")
                .unwrap()
                .with_timezone(&Utc)
        );
    }

    #[test]
    fn test_empty_cwd_falls_back_to_workspace_root() {
        let entries = parser()
            .parse_file(&session_file("1793069169145_bbbbb"))
            .unwrap();
        assert!(entries
            .iter()
            .all(|e| e.project.as_deref() == Some("/home/me/proj-beta")));
    }

    #[test]
    fn test_skips_messages_without_metrics_or_with_all_zero_usage() {
        let entries = parser()
            .parse_file(&session_file("1793069169145_bbbbb"))
            .unwrap();
        assert_eq!(entries.len(), 3);
        assert!(entries.iter().all(|e| !matches!(
            e.message_id.as_deref(),
            Some("msg_nometrics" | "msg_allzero")
        )));
    }

    #[test]
    fn test_session_without_record_has_no_project_and_strips_nested_prefix() {
        let (entries, dropped) = parser()
            .parse_entries(&session_file("1793100000000_ccccc"))
            .unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(
            dropped, 1,
            "a message with no timestamp anywhere is dropped"
        );
        let e = by_id(&entries, "msg_nosessionfile");
        assert_eq!(e.project, None);
        assert_eq!(e.model.as_deref(), Some("claude-opus-5.5"));
        assert_eq!(e.input_tokens, 200);
    }

    /// If cache reads exceed `inputTokens` the nesting assumption is broken:
    /// keep the counts truthful (input treated as already non-cached) and
    /// withdraw the reconciliation total instead of clamping input to zero.
    #[test]
    fn test_non_nested_cache_keeps_input_and_drops_reported_total() {
        let entries = parser()
            .parse_file(&session_file("1793100000000_ccccc"))
            .unwrap();
        let e = by_id(&entries, "msg_notnested");
        assert_eq!(e.input_tokens, 500);
        assert_eq!(e.cache_read_tokens, 800);
        assert_eq!(e.reported_total_tokens, None);
    }

    #[test]
    fn test_malformed_file_errors_but_parse_all_continues() {
        assert!(parser()
            .parse_file(&session_file("1793200000000_ddddd"))
            .is_err());
        let entries = parser().parse_all().unwrap();
        assert_eq!(entries.len(), 17);
    }

    #[test]
    fn test_empty_messages_yields_nothing() {
        let entries = parser()
            .parse_file(&session_file("1793300000000_eeeee"))
            .unwrap();
        assert!(entries.is_empty());
    }

    #[test]
    fn test_nonexistent_file_errors() {
        assert!(parser()
            .parse_file(Path::new("/nonexistent/x.messages.json"))
            .is_err());
    }

    #[test]
    fn test_reported_total_reconciles_across_all_fixtures() {
        let entries = parser().parse_all().unwrap();
        assert!(entries.iter().any(|e| e.reported_total_tokens.is_some()));
        for e in &entries {
            if let Some(reported) = e.reported_total_tokens {
                assert_eq!(reported, e.total_tokens(), "message {:?}", e.message_id);
            }
        }
    }

    #[test]
    fn test_entries_are_dedupable_and_reparse_is_idempotent() {
        let hashes: Vec<String> = parser()
            .parse_all()
            .unwrap()
            .iter()
            .map(|e| e.dedup_hash().expect("every entry needs a dedup hash"))
            .collect();
        let mut unique = hashes.clone();
        unique.sort();
        unique.dedup();
        assert_eq!(unique.len(), hashes.len());

        let files = parser().collect_files();
        let doubled: Vec<PathBuf> = files.iter().chain(files.iter()).cloned().collect();
        assert_eq!(parser().parse_and_dedup(&doubled).unwrap().len(), 17);
    }

    /// Numbers from disk are untrusted: extreme values must not panic inside
    /// the rayon pool, which would take every other source down with it.
    #[test]
    fn test_extreme_values_do_not_panic() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path().join("sessions").join("s1");
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("s1.messages.json");
        let max = u64::MAX;
        std::fs::write(
            &file,
            format!(
                r#"{{"messages":[{{"id":"m","role":"assistant","ts":1793000000000,"metrics":{{"inputTokens":{max},"outputTokens":{max},"cacheReadTokens":{max},"cacheWriteTokens":{max}}}}}]}}"#
            ),
        )
        .unwrap();
        let entries = ClineParser::with_data_dir(temp.path().to_path_buf())
            .parse_file(&file)
            .unwrap();
        assert_eq!(entries.len(), 1);
    }

    #[test]
    fn test_collect_files_escapes_glob_metacharacters_in_data_dir() {
        let temp = tempfile::tempdir().unwrap();
        let data_dir = temp.path().join("da[ta]");
        let dir = data_dir.join("sessions").join("s1");
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("s1.messages.json");
        std::fs::write(&file, r#"{"messages":[]}"#).unwrap();
        assert_eq!(
            ClineParser::with_data_dir(data_dir).collect_files(),
            vec![file]
        );
    }

    #[test]
    fn test_data_dir_env_precedence() {
        let saved_data = std::env::var("CLINE_DATA_DIR").ok();
        let saved_dir = std::env::var("CLINE_DIR").ok();
        let saved_sessions = std::env::var("CLINE_SESSION_DATA_DIR").ok();

        std::env::remove_var("CLINE_SESSION_DATA_DIR");
        std::env::remove_var("CLINE_DATA_DIR");
        std::env::set_var("CLINE_DIR", "/tmp/toktrack-cline-root");
        let from_dir = ClineParser::new().data_dir().to_path_buf();

        std::env::set_var("CLINE_DATA_DIR", "/tmp/toktrack-cline-data");
        let from_data = ClineParser::new().data_dir().to_path_buf();

        std::env::set_var("CLINE_SESSION_DATA_DIR", "/tmp/toktrack-cline-sessions");
        let from_sessions = ClineParser::new().data_dir().to_path_buf();

        match saved_sessions {
            Some(v) => std::env::set_var("CLINE_SESSION_DATA_DIR", v),
            None => std::env::remove_var("CLINE_SESSION_DATA_DIR"),
        }
        match saved_data {
            Some(v) => std::env::set_var("CLINE_DATA_DIR", v),
            None => std::env::remove_var("CLINE_DATA_DIR"),
        }
        match saved_dir {
            Some(v) => std::env::set_var("CLINE_DIR", v),
            None => std::env::remove_var("CLINE_DIR"),
        }

        assert_eq!(
            from_dir,
            PathBuf::from("/tmp/toktrack-cline-root/data/sessions")
        );
        assert_eq!(
            from_data,
            PathBuf::from("/tmp/toktrack-cline-data/sessions")
        );
        assert_eq!(from_sessions, PathBuf::from("/tmp/toktrack-cline-sessions"));
    }

    /// A fork copies the parent's messages (metrics intact); only messages
    /// stamped at/after `fork.forkedAt` are new usage.
    #[test]
    fn test_forked_session_skips_messages_inherited_from_parent() {
        let entries = parser()
            .parse_file(&session_file("1793400000000_fffff"))
            .unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].message_id.as_deref(), Some("msg_forknew01"));
        assert_eq!(entries[0].input_tokens, 1000);
    }

    /// Imported sessions carry another tool's usage (with `cost: 0` from
    /// Claude/Codex); toktrack already reads those tools directly.
    #[test]
    fn test_imported_session_skips_messages_before_import() {
        let entries = parser()
            .parse_file(&session_file("1793500000000_ggggg"))
            .unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].message_id.as_deref(), Some("msg_impnew01"));
    }

    fn parse_inline(json: &str) -> (Vec<UsageEntry>, tempfile::TempDir) {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path().join("sessions").join("s1");
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("s1.messages.json");
        std::fs::write(&file, json).unwrap();
        let entries = ClineParser::with_data_dir(temp.path().to_path_buf())
            .parse_file(&file)
            .unwrap();
        (entries, temp)
    }

    /// Compaction strips every message's `metrics` and keeps the totals in the
    /// first message's `metadata.usageBefore`; that aggregate is the usage.
    #[test]
    fn test_compaction_usage_before_becomes_one_entry() {
        let entries = parser()
            .parse_file(&session_file("1793600000000_hhhhh"))
            .unwrap();
        assert_eq!(entries.len(), 1);
        let e = &entries[0];
        // inputTokens nests cache: 1000 - 400 - 100.
        assert_eq!(e.input_tokens, 500);
        assert_eq!(e.output_tokens, 100);
        assert_eq!(e.cache_read_tokens, 400);
        assert_eq!(e.cache_creation_tokens, 100);
        assert_eq!(e.reported_total_tokens, Some(1100));
        assert_eq!(e.cost_usd, Some(0.5));
        assert_eq!(e.model.as_deref(), Some("claude-opus-4.8"));
        assert_eq!(e.provider.as_deref(), Some("cline"));
        assert_eq!(e.project.as_deref(), Some("/home/me/proj-compacted"));
        assert_eq!(e.message_id.as_deref(), Some("msg_compact01#usageBefore"));
        assert_eq!(e.request_id.as_deref(), Some("1793600000000_hhhhh"));
        // One aggregate has no per-day split: it is dated at the session start.
        assert_eq!(
            e.timestamp,
            DateTime::parse_from_rfc3339("2026-11-02T06:13:20.000Z")
                .unwrap()
                .with_timezone(&Utc)
        );
    }

    /// Without a session record the compaction message's own `ts` is used.
    #[test]
    fn test_usage_before_without_record_uses_message_ts() {
        let (entries, _t) = parse_inline(
            r#"{"messages":[{"id":"c","ts":1793000000000,"metadata":{"kind":"compaction","usageBefore":{"inputTokens":10,"outputTokens":1}}}]}"#,
        );
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].timestamp.timestamp_millis(), 1793000000000);
        assert_eq!(entries[0].message_id.as_deref(), Some("c#usageBefore"));
    }

    #[test]
    fn test_usage_before_all_zero_or_missing_is_ignored() {
        let (entries, _t) = parse_inline(
            r#"{"messages":[
{"id":"a","ts":1793000000000,"metadata":{"kind":"compaction","usageBefore":{"inputTokens":0,"cost":0}}},
{"id":"b","ts":1793000000000,"metadata":{"kind":"compaction"}},
{"id":"c","ts":1793000000000,"metadata":{"usageBefore":"oops"}}]}"#,
        );
        assert!(entries.is_empty());
    }

    /// A partially compacted session keeps metrics on surviving messages;
    /// upstream strips what it folds into `usageBefore`, so both are added
    /// and neither is counted twice.
    #[test]
    fn test_partial_compaction_counts_aggregate_and_survivors_once() {
        let entries = parser()
            .parse_file(&session_file("1793700000000_iiiii"))
            .unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(
            by_id(&entries, "msg_partial_c#usageBefore").input_tokens,
            2000
        );
        assert_eq!(by_id(&entries, "msg_partial_a").input_tokens, 300);
        let hashes: std::collections::HashSet<_> =
            entries.iter().filter_map(|e| e.dedup_hash()).collect();
        assert_eq!(hashes.len(), 2);
    }

    /// A fork copies the parent's compaction message along with its
    /// `usageBefore`; stamped before `forkedAt`, it is the parent's usage.
    #[test]
    fn test_forked_session_skips_copied_compaction_aggregate() {
        let entries = parser()
            .parse_file(&session_file("1793800000000_jjjjj"))
            .unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].message_id.as_deref(), Some("msg_jnew01"));
    }

    fn subagent_file() -> PathBuf {
        root()
            .join("sessions")
            .join("1793900000000_kkkkk")
            .join("agent_abc.messages.json")
    }

    /// Subagents write `<root>/<agentId>.messages.json` with no manifest of
    /// their own, so project/model/provider/start come from the root record.
    #[test]
    fn test_subagent_file_inherits_root_session_record() {
        let entries = parser().parse_file(&subagent_file()).unwrap();
        assert_eq!(entries.len(), 2);
        for e in &entries {
            assert_eq!(e.project.as_deref(), Some("/home/me/proj-subagents"));
        }
        let e = by_id(&entries, "msg_subnots");
        assert_eq!(e.model.as_deref(), Some("claude-sonnet-5.5"));
        assert_eq!(e.provider.as_deref(), Some("anthropic"));
        assert_eq!(
            e.timestamp,
            DateTime::parse_from_rfc3339("2026-11-05T17:33:20.000Z")
                .unwrap()
                .with_timezone(&Utc)
        );
    }

    /// Subagent message ids may repeat the root's; the dedup key must differ.
    #[test]
    fn test_subagent_ids_do_not_collide_with_root_under_dedup() {
        let root_entries = parser()
            .parse_file(&session_file("1793900000000_kkkkk"))
            .unwrap();
        let sub_entries = parser().parse_file(&subagent_file()).unwrap();
        // The root's pre-fork message is skipped; its new one shares an id
        // with a subagent message.
        assert_eq!(root_entries.len(), 1);
        assert_eq!(
            root_entries[0].request_id.as_deref(),
            Some("1793900000000_kkkkk")
        );
        let sub = by_id(&sub_entries, "msg_same0001");
        assert_eq!(
            sub.request_id.as_deref(),
            Some("1793900000000_kkkkk/agent_abc")
        );
        assert_ne!(root_entries[0].dedup_hash(), sub.dedup_hash());

        let files = [session_file("1793900000000_kkkkk"), subagent_file()];
        let deduped = parser().parse_and_dedup(&files).unwrap();
        assert_eq!(deduped.len(), 3, "root 1 + subagent 2, none merged");
        let input: u64 = deduped.iter().map(|e| e.input_tokens).sum();
        assert_eq!(input, 100 + 200 + 40);
    }

    /// The root's fork cutoff is not applied to subagent files: they are
    /// created by the forked session, never seeded from the parent.
    #[test]
    fn test_subagent_is_not_filtered_by_root_fork_cutoff() {
        let (entries, _t) = {
            let temp = tempfile::tempdir().unwrap();
            let dir = temp.path().join("sessions").join("r1");
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(
                dir.join("r1.json"),
                r#"{"started_at":"2026-11-05T00:00:00.000Z","metadata":{"fork":{"forkedAt":"2026-11-05T00:00:00.000Z"}}}"#,
            )
            .unwrap();
            let file = dir.join("a1.messages.json");
            std::fs::write(
                &file,
                r#"{"messages":[{"id":"m","ts":1700000000000,"metrics":{"inputTokens":7}}]}"#,
            )
            .unwrap();
            let entries = ClineParser::with_data_dir(temp.path().to_path_buf())
                .parse_file(&file)
                .unwrap();
            (entries, temp)
        };
        assert_eq!(entries.len(), 1);
    }

    /// A message `metadata` that is not an object must not fail the file.
    #[test]
    fn test_non_object_message_metadata_does_not_drop_the_file() {
        let (entries, _t) = parse_inline(
            r#"{"messages":[
{"id":"a","ts":1793000000000,"metrics":{"inputTokens":5},"metadata":"x"},
{"id":"b","ts":1793000000000,"metrics":{"inputTokens":6},"metadata":[1]},
{"id":"c","ts":1793000000000,"metrics":{"inputTokens":7},"metadata":null}]}"#,
        );
        assert_eq!(entries.len(), 3);
    }

    /// A root session file copied or renamed within its own directory still
    /// has its own record: it is not a subagent and keeps its fork cutoff.
    #[test]
    fn test_renamed_root_file_with_own_record_is_not_a_child() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path().join("sessions").join("backup");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("123.json"),
            r#"{"cwd":"/p","started_at":"2026-11-05T00:00:00.000Z","metadata":{"fork":{"forkedAt":"2026-11-05T00:00:00.000Z"}}}"#,
        )
        .unwrap();
        let file = dir.join("123.messages.json");
        std::fs::write(
            &file,
            r#"{"messages":[
{"id":"seed","ts":1700000000000,"metrics":{"inputTokens":1}},
{"id":"new","ts":1794000000000,"metrics":{"inputTokens":2}}]}"#,
        )
        .unwrap();
        let entries = ClineParser::with_data_dir(temp.path().to_path_buf())
            .parse_file(&file)
            .unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].message_id.as_deref(), Some("new"));
        assert_eq!(entries[0].project.as_deref(), Some("/p"));
        assert_eq!(entries[0].request_id.as_deref(), Some("123"));
    }

    /// No id on the compaction message: the index keeps the entry's id stable.
    #[test]
    fn test_usage_before_without_message_id_uses_index() {
        let (entries, _t) = parse_inline(
            r#"{"messages":[{"ts":1793000000000,"metadata":{"usageBefore":{"inputTokens":10}}}]}"#,
        );
        assert_eq!(entries[0].message_id.as_deref(), Some("@0#usageBefore"));
    }

    /// One bad value must not drop the whole file.
    #[test]
    fn test_bad_values_degrade_per_field_not_per_file() {
        let (entries, _t) = parse_inline(
            r#"{"version":"x","messages":[
{"id":"a","ts":"yesterday","metrics":{"inputTokens":10.0,"outputTokens":-5,"cacheReadTokens":"7","cost":"free"}},
{"id":"b","ts":1793000000000,"metrics":{"inputTokens":20,"outputTokens":2}}]}"#,
        );
        // "a": ts unusable and no session record -> dropped; "b" survives.
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].message_id.as_deref(), Some("b"));

        let (entries, _t) = parse_inline(
            r#"{"messages":[{"id":"a","ts":1793000000000,"metrics":{"inputTokens":10.0,"outputTokens":-5,"cost":"free"}}]}"#,
        );
        assert_eq!(entries[0].input_tokens, 10);
        assert_eq!(entries[0].output_tokens, 0);
        assert_eq!(entries[0].cost_usd, None);
    }

    #[test]
    fn test_unexpected_version_still_parses() {
        let (entries, _t) = parse_inline(
            r#"{"version":2,"messages":[{"id":"a","ts":1793000000000,"metrics":{"inputTokens":10}}]}"#,
        );
        assert_eq!(entries.len(), 1);
    }

    #[test]
    fn test_strip_provider_prefix() {
        assert_eq!(
            strip_provider_prefix("anthropic/claude-sonnet-5.5"),
            "claude-sonnet-5.5"
        );
        assert_eq!(strip_provider_prefix("openrouter/anthropic/x"), "x");
        assert_eq!(strip_provider_prefix("gpt-5.5"), "gpt-5.5");
    }

    // ---- schema drift canary ----

    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    #[allow(dead_code)] // Fields exist to declare the accepted schema, not to be read
    struct KnownMetrics {
        #[serde(rename = "inputTokens")]
        input_tokens: Option<u64>,
        #[serde(rename = "outputTokens")]
        output_tokens: Option<u64>,
        #[serde(rename = "cacheReadTokens")]
        cache_read_tokens: Option<u64>,
        #[serde(rename = "cacheWriteTokens")]
        cache_write_tokens: Option<u64>,
        cost: Option<f64>,
    }

    /// Newest `count` real Cline session files, empty when Cline never ran
    /// here (CI).
    fn recent_local_sessions(count: usize) -> Vec<PathBuf> {
        let mut files: Vec<(std::time::SystemTime, PathBuf)> = ClineParser::new()
            .collect_files()
            .into_iter()
            .filter_map(|p| Some((p.metadata().and_then(|m| m.modified()).ok()?, p)))
            .collect();
        files.sort_by_key(|(mtime, _)| std::cmp::Reverse(*mtime));
        files.into_iter().take(count).map(|(_, p)| p).collect()
    }

    fn assert_metrics_shape_known(path: &Path) {
        // A live session can be mid-write; only a parsable file can show drift.
        let Ok(raw) = std::fs::read_to_string(path) else {
            return;
        };
        let Ok(value) = serde_json::from_str::<serde_json::Value>(&raw) else {
            return;
        };
        let Some(messages) = value.get("messages").and_then(|m| m.as_array()) else {
            return;
        };
        for (i, msg) in messages.iter().enumerate() {
            let Some(metrics) = msg.get("metrics") else {
                continue;
            };
            if let Err(e) = serde_json::from_value::<KnownMetrics>(metrics.clone()) {
                panic!(
                    "Cline metrics schema drifted at {} message #{} — {}. Decide whether \
                     the new field affects tokens or cost, then add it here (and to \
                     ClineMetrics if it does).",
                    path.display(),
                    i,
                    e
                );
            }
        }
    }

    #[test]
    fn test_cline_metrics_schema_has_no_unknown_fields() {
        for path in parser().collect_files() {
            assert_metrics_shape_known(&path);
        }
        for path in recent_local_sessions(5) {
            assert_metrics_shape_known(&path);
        }
    }
}
