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
    #[serde(default, borrow)]
    messages: Vec<ClineMessage<'a>>,
}

#[derive(Deserialize)]
struct ClineMessage<'a> {
    #[serde(default, borrow)]
    id: Option<Cow<'a, str>>,
    /// Epoch milliseconds.
    #[serde(default)]
    ts: Option<i64>,
    #[serde(default, borrow, rename = "modelInfo")]
    model_info: Option<ClineModelInfo<'a>>,
    #[serde(default)]
    metrics: Option<ClineMetrics>,
}

#[derive(Deserialize)]
struct ClineModelInfo<'a> {
    #[serde(default, borrow)]
    id: Option<Cow<'a, str>>,
    #[serde(default, borrow)]
    provider: Option<Cow<'a, str>>,
}

/// Fields are optional so a single `null` cannot fail the whole file.
#[derive(Deserialize, Default, Clone, Copy)]
#[serde(rename_all = "camelCase")]
struct ClineMetrics {
    #[serde(default)]
    input_tokens: Option<u64>,
    #[serde(default)]
    output_tokens: Option<u64>,
    #[serde(default)]
    cache_read_tokens: Option<u64>,
    #[serde(default)]
    cache_write_tokens: Option<u64>,
    /// `None` when absent: a missing figure must stay unknown rather than
    /// become a confident $0.00 that suppresses the estimated-cost fallback.
    #[serde(default)]
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
    /// `~/.cline/data` (or an override). Sessions live under `sessions/`.
    data_dir: PathBuf,
}

impl ClineParser {
    /// Create a parser with the default data directory.
    ///
    /// Honors `CLINE_DATA_DIR` (replaces `~/.cline/data`), then `CLINE_DIR`
    /// (replaces `~/.cline`, data under `data/`) — the upstream variables.
    pub fn new() -> Self {
        let data_dir = super::discovery::first_env_dir(&["CLINE_DATA_DIR"])
            .or_else(|| super::discovery::first_env_dir(&["CLINE_DIR"]).map(|d| d.join("data")))
            .or_else(|| {
                directories::BaseDirs::new().map(|d| d.home_dir().join(".cline").join("data"))
            })
            .unwrap_or_else(|| {
                eprintln!("[toktrack] Warning: Could not determine home directory");
                PathBuf::from(".")
            });
        Self { data_dir }
    }

    /// Create a parser rooted at a custom data directory (for testing).
    #[allow(dead_code)] // Used in tests
    pub fn with_data_dir(data_dir: PathBuf) -> Self {
        Self { data_dir }
    }

    /// Session id from `<id>.messages.json`.
    fn session_id(path: &Path) -> Option<String> {
        let name = path.file_name()?.to_str()?;
        name.strip_suffix(MESSAGES_SUFFIX).map(String::from)
    }

    /// The sibling session record, or defaults when absent or unreadable —
    /// it only supplies fallbacks, so losing it must not drop the usage.
    fn session_record(path: &Path, session_id: Option<&str>) -> ClineSessionRecord {
        let Some(id) = session_id else {
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

        let session_id = Self::session_id(path);
        let record = Self::session_record(path, session_id.as_deref());
        let project = non_empty(record.cwd.as_deref())
            .or_else(|| non_empty(record.workspace_root.as_deref()))
            .map(String::from);
        let fallback_ts = record
            .started_at
            .as_deref()
            .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
            .map(|dt| dt.with_timezone(&Utc));

        let mut entries = Vec::new();
        let mut dropped = 0usize;
        let mut drifted = 0usize;

        for msg in &parsed.messages {
            let Some(metrics) = msg.metrics.filter(has_usage) else {
                continue;
            };
            let Some(timestamp) = msg
                .ts
                .and_then(|ms| Utc.timestamp_millis_opt(ms).single())
                .or(fallback_ts)
            else {
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
                message_id: msg.id.as_deref().map(String::from),
                request_id: session_id.clone(),
                source: Some("cline".into()),
                provider,
                project: project.clone(),
            });
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
        &self.data_dir
    }

    fn file_pattern(&self) -> &str {
        "sessions/*/*.messages.json"
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
        assert_eq!(p.file_pattern(), "sessions/*/*.messages.json");
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
        assert_eq!(entries.len(), 8);
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
        assert_eq!(parser().parse_and_dedup(&doubled).unwrap().len(), 8);
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

        std::env::remove_var("CLINE_DATA_DIR");
        std::env::set_var("CLINE_DIR", "/tmp/toktrack-cline-root");
        let from_dir = ClineParser::new().data_dir().to_path_buf();

        std::env::set_var("CLINE_DATA_DIR", "/tmp/toktrack-cline-data");
        let from_data = ClineParser::new().data_dir().to_path_buf();

        match saved_data {
            Some(v) => std::env::set_var("CLINE_DATA_DIR", v),
            None => std::env::remove_var("CLINE_DATA_DIR"),
        }
        match saved_dir {
            Some(v) => std::env::set_var("CLINE_DIR", v),
            None => std::env::remove_var("CLINE_DIR"),
        }

        assert_eq!(from_dir, PathBuf::from("/tmp/toktrack-cline-root/data"));
        assert_eq!(from_data, PathBuf::from("/tmp/toktrack-cline-data"));
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
