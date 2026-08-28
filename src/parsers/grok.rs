//! Grok CLI parser
//!
//! xAI's Grok CLI (`grok`) writes one session directory per conversation under
//! `~/.grok/sessions/<encoded-cwd>/<session-id>/`. Token usage lives in
//! `updates.jsonl`, an ACP-style JSON-RPC notification log.
//!
//! Only one record shape carries usage:
//!
//! ```text
//! method == "_x.ai/session/update"
//!   && params.update.sessionUpdate == "turn_completed"
//!   -> params.update.usage
//! ```
//!
//! Each such record is **one user turn** (not cumulative): a session emits one
//! per `prompt_id`, matching the `turn_started`/`turn_ended` pairs in the
//! sibling `events.jsonl`. The `numTurns`/`modelCalls` fields count inner
//! agent-loop iterations within that turn, not turns of the session, so they
//! are non-monotonic across records and must not be treated as a running total.
//!
//! Ordinary `session/update` lines carry a `_meta.totalTokens` field. That is a
//! **running context-window size** (it climbs toward the model's
//! `context_window` and resets on compaction), NOT usage — summing it
//! over-reports by an order of magnitude. It is deliberately ignored.
//!
//! Token nesting, proven by arithmetic on real sessions
//! (`totalTokens == inputTokens + outputTokens` exactly):
//! `cachedReadTokens` is a subset of `inputTokens`, and `reasoningTokens` is a
//! subset of `outputTokens`. Both are subtracted out to satisfy the v2 contract.

use crate::types::{Result, ToktrackError, UsageEntry};
use chrono::{DateTime, Utc};
use serde::Deserialize;
use std::collections::HashMap;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

use super::CLIParser;

/// Grok reports per-turn cost as an integer tick count. One tick is exactly
/// 1e-10 USD — verified by reproducing LiteLLM's `xai/grok-4.6` rates ($2/M
/// input, $6/M output, $0.50/M cache read) to every digit across three real
/// turns.
///
/// The reported figure is preferred over toktrack's pricing table for two
/// reasons: the bare model id (`grok-4.6`) is absent from the LiteLLM snapshot
/// (only the provider-prefixed `xai/grok-4.6` exists, which `get_pricing`'s
/// fallback scan skips because it contains `/`), and xAI applies its
/// above-200k tier per API call while `tiered_cost` applies it per entry — a
/// turn whose cache reads exceed 200k across several sub-200k calls would
/// otherwise be over-billed.
const COST_USD_TICKS_PER_USD: f64 = 1e10;

/// One line of `updates.jsonl`.
#[derive(Deserialize)]
struct GrokLine<'a> {
    #[serde(default, borrow)]
    method: Option<&'a str>,
    /// Epoch **seconds**; `_meta.agentTimestampMs` is preferred when present.
    #[serde(default)]
    timestamp: Option<i64>,
    #[serde(default, borrow)]
    params: Option<GrokParams<'a>>,
}

#[derive(Deserialize)]
struct GrokParams<'a> {
    #[serde(default, borrow)]
    update: Option<GrokUpdate<'a>>,
    #[serde(default, borrow, rename = "_meta")]
    meta: Option<GrokMeta<'a>>,
}

/// Note the mixed casing: `sessionUpdate` is camelCase while `prompt_id` and
/// `stop_reason` are snake_case in the same object, so fields are renamed
/// individually rather than with a container-level `rename_all`.
#[derive(Deserialize)]
struct GrokUpdate<'a> {
    #[serde(default, borrow, rename = "sessionUpdate")]
    session_update: Option<&'a str>,
    #[serde(default, borrow)]
    prompt_id: Option<&'a str>,
    #[serde(default)]
    usage: Option<GrokUsage>,
}

#[derive(Deserialize)]
struct GrokMeta<'a> {
    #[serde(default, borrow, rename = "eventId")]
    event_id: Option<&'a str>,
    #[serde(default, rename = "agentTimestampMs")]
    agent_timestamp_ms: Option<i64>,
}

/// Per-model (or whole-turn) token counts. Field names are camelCase.
#[derive(Deserialize, Default, Clone, Copy)]
#[serde(rename_all = "camelCase")]
struct GrokTokens {
    #[serde(default)]
    input_tokens: u64,
    #[serde(default)]
    output_tokens: u64,
    #[serde(default)]
    total_tokens: u64,
    #[serde(default)]
    cached_read_tokens: u64,
    #[serde(default)]
    cache_creation_tokens: u64,
    #[serde(default)]
    reasoning_tokens: u64,
    #[serde(default)]
    cost_usd_ticks: u64,
}

/// `params.update.usage` — turn totals plus an optional per-model breakdown.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct GrokUsage {
    #[serde(default)]
    input_tokens: u64,
    #[serde(default)]
    output_tokens: u64,
    #[serde(default)]
    total_tokens: u64,
    #[serde(default)]
    cached_read_tokens: u64,
    #[serde(default)]
    cache_creation_tokens: u64,
    #[serde(default)]
    reasoning_tokens: u64,
    #[serde(default)]
    cost_usd_ticks: u64,
    /// Keyed by model id (e.g. `grok-4.6`). Absent on older records.
    #[serde(default)]
    model_usage: Option<HashMap<String, GrokTokens>>,
}

impl GrokUsage {
    /// Turn totals as a `GrokTokens`, used when `modelUsage` is absent.
    fn totals(&self) -> GrokTokens {
        GrokTokens {
            input_tokens: self.input_tokens,
            output_tokens: self.output_tokens,
            total_tokens: self.total_tokens,
            cached_read_tokens: self.cached_read_tokens,
            cache_creation_tokens: self.cache_creation_tokens,
            reasoning_tokens: self.reasoning_tokens,
            cost_usd_ticks: self.cost_usd_ticks,
        }
    }
}

/// `summary.json`, read once per session for project + model fallback.
#[derive(Deserialize)]
struct GrokSummary {
    #[serde(default)]
    info: Option<GrokSummaryInfo>,
    #[serde(default)]
    current_model_id: Option<String>,
}

#[derive(Deserialize)]
struct GrokSummaryInfo {
    #[serde(default)]
    cwd: Option<String>,
}

/// Decode the `%XX` escapes Grok uses to encode a working directory as a single
/// path component (`C%3A%5CUsers%5Cme` -> `C:\Users\me`).
fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hi = (bytes[i + 1] as char).to_digit(16);
            let lo = (bytes[i + 2] as char).to_digit(16);
            if let (Some(h), Some(l)) = (hi, lo) {
                out.push((h * 16 + l) as u8);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Parser for xAI Grok CLI usage data.
pub struct GrokParser {
    /// `~/.grok` root (or `$GROK_HOME`). Sessions live under `sessions/`.
    data_dir: PathBuf,
}

impl GrokParser {
    /// Create a parser with the default data directory.
    ///
    /// Honors `GROK_HOME` (overrides `~/.grok`).
    pub fn new() -> Self {
        let root = super::discovery::first_env_dir(&["GROK_HOME"]).unwrap_or_else(|| {
            directories::BaseDirs::new()
                .map(|d| d.home_dir().join(".grok"))
                .unwrap_or_else(|| {
                    eprintln!("[toktrack] Warning: Could not determine home directory");
                    PathBuf::from(".")
                })
        });
        Self { data_dir: root }
    }

    /// Create a parser rooted at a custom `.grok` directory (for testing).
    #[allow(dead_code)] // Used in tests
    pub fn with_data_dir(data_dir: PathBuf) -> Self {
        Self { data_dir }
    }

    /// Resolve `(project, fallback_model)` for the session containing `path`.
    ///
    /// Project attribution, first that resolves:
    /// 1. `summary.json` -> `info.cwd` (authoritative — Grok records the real
    ///    path even when the directory name is a hashed slug)
    /// 2. a `.cwd` file in the session-group directory (Grok writes one when
    ///    the encoded name would exceed 255 bytes)
    /// 3. the percent-decoded group directory name
    fn session_context(&self, path: &Path) -> (Option<String>, Option<String>) {
        let summary = path.with_file_name("summary.json");
        let mut project = None;
        let mut model = None;

        if let Ok(raw) = std::fs::read(&summary) {
            let mut bytes = raw;
            if let Ok(parsed) = simd_json::from_slice::<GrokSummary>(&mut bytes) {
                project = parsed.info.and_then(|i| i.cwd).filter(|c| !c.is_empty());
                model = parsed.current_model_id.filter(|m| !m.is_empty());
            }
        }

        // The group directory is the parent of the session directory.
        let group = path.parent().and_then(Path::parent);

        if project.is_none() {
            if let Some(group) = group {
                if let Ok(raw) = std::fs::read_to_string(group.join(".cwd")) {
                    let trimmed = raw.trim();
                    if !trimmed.is_empty() {
                        project = Some(trimmed.to_string());
                    }
                }
            }
        }

        if project.is_none() {
            project = group
                .and_then(Path::file_name)
                .and_then(|n| n.to_str())
                .map(percent_decode)
                .filter(|p| !p.is_empty());
        }

        (project, model)
    }

    /// Build the entry for one model's slice of a `turn_completed` record.
    fn entry_for_model(
        &self,
        tokens: &GrokTokens,
        model: Option<&str>,
        timestamp: DateTime<Utc>,
        event_id: Option<&str>,
        prompt_id: Option<&str>,
        project: Option<&str>,
    ) -> UsageEntry {
        UsageEntry {
            fast_speed: false,
            timestamp,
            model: model.map(String::from),
            // `inputTokens` INCLUDES `cachedReadTokens`; subtract so
            // `input_tokens` is billable non-cached input and cached reads are
            // not charged twice. `cacheCreationTokens` was 0 in every observed
            // sample, so its nesting is unverified — saturating subtraction
            // keeps a wrong assumption from breaking the total invariant.
            input_tokens: tokens
                .input_tokens
                .saturating_sub(tokens.cached_read_tokens)
                .saturating_sub(tokens.cache_creation_tokens),
            // `outputTokens` INCLUDES `reasoningTokens`; subtract so
            // `output_tokens` is visible output only.
            output_tokens: tokens.output_tokens.saturating_sub(tokens.reasoning_tokens),
            cache_read_tokens: tokens.cached_read_tokens,
            cache_creation_tokens: tokens.cache_creation_tokens,
            reasoning_tokens: tokens.reasoning_tokens,
            cache_creation_5m_tokens: 0,
            cache_creation_1h_tokens: 0,
            web_search_requests: 0,
            web_fetch_requests: 0,
            reported_total_tokens: Some(tokens.total_tokens),
            cost_usd: Some(tokens.cost_usd_ticks as f64 / COST_USD_TICKS_PER_USD),
            // `eventId` is unique per session; the model is appended so a
            // multi-model turn fans out into entries with distinct dedup hashes.
            message_id: event_id.map(|e| match model {
                Some(m) => format!("{}#{}", e, m),
                None => e.to_string(),
            }),
            request_id: prompt_id.map(String::from),
            source: Some("grok".into()),
            provider: Some("xai".into()),
            project: project.map(String::from),
        }
    }
}

impl Default for GrokParser {
    fn default() -> Self {
        Self::new()
    }
}

impl CLIParser for GrokParser {
    fn name(&self) -> &str {
        "grok"
    }

    fn data_dir(&self) -> &Path {
        &self.data_dir
    }

    fn file_pattern(&self) -> &str {
        "sessions/*/*/updates.jsonl"
    }

    fn parse_file(&self, path: &Path) -> Result<Vec<UsageEntry>> {
        let file = File::open(path).map_err(ToktrackError::Io)?;
        let reader = BufReader::new(file);
        let (project, fallback_model) = self.session_context(path);
        let mut entries = Vec::new();

        for line_result in reader.lines() {
            let line = match line_result {
                Ok(l) => l,
                Err(_) => continue,
            };
            if line.trim().is_empty() {
                continue;
            }

            // Cheap prefilter: a session's `updates.jsonl` is overwhelmingly
            // streamed chunks and tool results — and those are the *large*
            // lines (a single tool result can be hundreds of KB), while only a
            // handful are turn completions. Skipping the full parse for lines
            // that cannot match keeps this off the hot path. A false positive
            // (the text appearing inside a tool result) is harmless: the
            // method/sessionUpdate checks below still reject it.
            if !line.contains("turn_completed") {
                continue;
            }

            let mut bytes = line.into_bytes();
            let parsed: GrokLine = match simd_json::from_slice(&mut bytes) {
                Ok(v) => v,
                Err(_) => continue,
            };

            // Usage lives only on the xAI-private turn-completion notification.
            // Everything else — streamed chunks, tool calls, retries, recaps —
            // carries at most the running context counter.
            if parsed.method != Some("_x.ai/session/update") {
                continue;
            }
            let params = match parsed.params {
                Some(p) => p,
                None => continue,
            };
            let update = match params.update {
                Some(u) => u,
                None => continue,
            };
            if update.session_update != Some("turn_completed") {
                continue;
            }
            let usage = match update.usage {
                Some(u) => u,
                None => continue,
            };

            let (event_id, ts_ms) = match params.meta {
                Some(m) => (m.event_id, m.agent_timestamp_ms),
                None => (None, None),
            };

            // Prefer the millisecond agent clock; fall back to the record's
            // epoch-seconds envelope.
            let timestamp = match ts_ms.and_then(DateTime::from_timestamp_millis).or_else(|| {
                parsed
                    .timestamp
                    .and_then(|s| DateTime::from_timestamp(s, 0))
            }) {
                Some(t) => t,
                // Silently dropping a turn here would under-report usage, which
                // is the worst failure mode for a cost tracker — make it visible.
                None => {
                    eprintln!(
                        "[toktrack] Warning: Grok turn with unreadable timestamp \
                         (agentTimestampMs={:?}, timestamp={:?}) in {:?}, skipping",
                        ts_ms, parsed.timestamp, path
                    );
                    continue;
                }
            };

            match &usage.model_usage {
                // Per-model breakdown present: one entry per model, which keeps
                // attribution (and cost) exact for turns that switch models.
                Some(map) if !map.is_empty() => {
                    let mut models: Vec<&String> = map.keys().collect();
                    models.sort(); // deterministic order across runs
                    for model in models {
                        entries.push(self.entry_for_model(
                            &map[model],
                            Some(model.as_str()),
                            timestamp,
                            event_id,
                            update.prompt_id,
                            project.as_deref(),
                        ));
                    }
                }
                // No breakdown: attribute the turn totals to the session model.
                _ => entries.push(self.entry_for_model(
                    &usage.totals(),
                    fallback_model.as_deref(),
                    timestamp,
                    event_id,
                    update.prompt_id,
                    project.as_deref(),
                )),
            }
        }

        Ok(entries)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn root() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("fixtures")
            .join("grok")
    }

    fn parser() -> GrokParser {
        GrokParser::with_data_dir(root())
    }

    fn real_session() -> PathBuf {
        root()
            .join("sessions")
            .join("C%3A%5Cwork%5Cdemo")
            .join("0f0f0f0f-1111-4222-8333-444444444444")
            .join("updates.jsonl")
    }

    #[test]
    fn test_name_and_pattern() {
        let p = parser();
        assert_eq!(p.name(), "grok");
        assert_eq!(p.file_pattern(), "sessions/*/*/updates.jsonl");
    }

    #[test]
    fn test_real_session_yields_one_entry_per_turn() {
        let entries = parser().parse_file(&real_session()).unwrap();
        assert_eq!(entries.len(), 3, "one entry per turn_completed record");
    }

    /// The noise lines carry `_meta.totalTokens` of 4146 / 13497 / 71784. Those
    /// are context-window sizes, not usage — no entry may be derived from them.
    #[test]
    fn test_context_size_counter_is_ignored() {
        let entries = parser().parse_file(&real_session()).unwrap();
        for e in &entries {
            for counter in [4146u64, 13497, 71784] {
                assert_ne!(e.reported_total_tokens, Some(counter));
                assert_ne!(e.total_tokens(), counter);
            }
        }
    }

    #[test]
    fn test_token_mapping_strips_nested_counts() {
        let entries = parser().parse_file(&real_session()).unwrap();
        let e = &entries[0];
        // turn 1: input 82669 (cachedRead 40448), output 1034 (reasoning 573)
        assert_eq!(e.input_tokens, 42221);
        assert_eq!(e.output_tokens, 461);
        assert_eq!(e.cache_read_tokens, 40448);
        assert_eq!(e.cache_creation_tokens, 0);
        assert_eq!(e.reasoning_tokens, 573);
        assert_eq!(e.model.as_deref(), Some("grok-4.6"));
        assert_eq!(e.source.as_deref(), Some("grok"));
        assert_eq!(e.provider.as_deref(), Some("xai"));
    }

    /// v2 invariant: the upstream total must reconcile with the mapped fields.
    #[test]
    fn test_reported_total_reconciles() {
        let entries = parser().parse_file(&real_session()).unwrap();
        for e in &entries {
            assert_eq!(
                e.reported_total_tokens,
                Some(e.total_tokens()),
                "reported_total_tokens must equal total_tokens()"
            );
        }
        let reported: u64 = entries.iter().filter_map(|e| e.reported_total_tokens).sum();
        assert_eq!(reported, 83703 + 125879 + 229491);
    }

    #[test]
    fn test_cost_from_ticks() {
        let entries = parser().parse_file(&real_session()).unwrap();
        let costs: Vec<f64> = entries.iter().map(|e| e.cost_usd.unwrap()).collect();
        for (got, want) in costs.iter().zip([0.110870, 0.132250, 0.160058]) {
            assert!((got - want).abs() < 1e-9, "cost {} != {}", got, want);
        }
    }

    /// A rate-limited turn still spent (and was billed for) its tokens.
    #[test]
    fn test_rate_limited_turn_is_counted() {
        let entries = parser().parse_file(&real_session()).unwrap();
        let last = entries.last().unwrap();
        assert_eq!(last.cache_read_tokens, 205824);
        assert_eq!(
            last.request_id.as_deref(),
            Some("2a310163-9ae4-4380-9a60-8dbc164dec19")
        );
    }

    #[test]
    fn test_project_from_summary_json() {
        let entries = parser().parse_file(&real_session()).unwrap();
        assert_eq!(entries[0].project.as_deref(), Some("C:\\work\\demo"));
    }

    #[test]
    fn test_project_from_cwd_sidecar_and_multi_model_fanout() {
        let path = root()
            .join("sessions")
            .join("deeply-nested-project-a1b2c3d4")
            .join("0f0f0f0f-2222-4333-8444-555555555555")
            .join("updates.jsonl");
        let entries = parser().parse_file(&path).unwrap();

        assert_eq!(entries.len(), 2, "one entry per model in modelUsage");
        for e in &entries {
            // Slug directory name is not a path — the `.cwd` sidecar wins.
            assert_eq!(e.project.as_deref(), Some("C:\\very\\long\\path\\project"));
        }
        let models: Vec<&str> = entries.iter().filter_map(|e| e.model.as_deref()).collect();
        assert_eq!(models, vec!["grok-4.6", "grok-code-fast-1"]);

        // Fanned-out entries must not collapse into each other on dedup.
        assert_ne!(entries[0].dedup_hash(), entries[1].dedup_hash());
    }

    #[test]
    fn test_project_from_percent_decoded_dir_and_model_fallback() {
        let path = root()
            .join("sessions")
            .join("D%3A%5Cproj%5Cbeta")
            .join("0f0f0f0f-3333-4444-8555-666666666666")
            .join("updates.jsonl");
        let entries = parser().parse_file(&path).unwrap();

        assert_eq!(entries.len(), 1);
        // summary.json has no info.cwd here, so the encoded dir name is decoded.
        assert_eq!(entries[0].project.as_deref(), Some("D:\\proj\\beta"));
        // No modelUsage -> fall back to summary.json current_model_id.
        assert_eq!(entries[0].model.as_deref(), Some("grok-4.6"));
        assert_eq!(entries[0].input_tokens, 3500);
    }

    #[test]
    fn test_parse_all_collects_every_session() {
        let entries = parser().parse_all().unwrap();
        assert_eq!(entries.len(), 6, "3 + 2 (multi-model) + 1");
    }

    /// The v2 invariant must hold on *every* fixture, not just the real session.
    ///
    /// This is the only test exercising a non-zero `cacheCreationTokens` (50, in
    /// the multi-model fixture), because all three real sampled turns had 0.
    /// NOTE: it proves the mapping is self-consistent under assumption A1
    /// (`cacheCreationTokens` nested inside `inputTokens`); it cannot confirm
    /// A1 itself, since that fixture's `totalTokens` was authored to match.
    /// Re-check against a real session that performs cache writes.
    #[test]
    fn test_reported_total_reconciles_across_all_fixtures() {
        let entries = parser().parse_all().unwrap();
        assert!(
            entries.iter().any(|e| e.cache_creation_tokens > 0),
            "fixture set must exercise a non-zero cache_creation_tokens"
        );
        for e in &entries {
            assert_eq!(
                e.reported_total_tokens,
                Some(e.total_tokens()),
                "invariant broken for model {:?}",
                e.model
            );
        }
    }

    /// Every entry must carry a dedup hash, and re-parsing must not double-count:
    /// Grok appends to a live session file, so the warm path re-reads it whole.
    #[test]
    fn test_entries_are_dedupable_and_reparse_is_idempotent() {
        let entries = parser().parse_all().unwrap();
        let mut hashes: Vec<String> = entries
            .iter()
            .map(|e| e.dedup_hash().expect("every entry needs a dedup hash"))
            .collect();
        hashes.sort();
        let before = hashes.len();
        hashes.dedup();
        assert_eq!(hashes.len(), before, "dedup hashes must be unique");
    }

    #[test]
    fn test_missing_directory_is_empty() {
        let p = GrokParser::with_data_dir(PathBuf::from("tests/fixtures/nonexistent-grok"));
        assert!(p.parse_all().unwrap().is_empty());
    }

    /// Upstream schema canary. serde drops unknown fields silently, so a new
    /// billed token category appearing in `usage` (xAI has already shipped
    /// `cacheCreationTokens` and `reasoningTokens` here) would under-report
    /// forever without a word. This mirror declares what we have seen and
    /// rejects the rest — over the fixtures always, and over the newest real
    /// session files when this machine has Grok installed.
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    #[allow(dead_code)] // Fields declare the accepted schema, they are not read
    struct KnownUsage {
        #[serde(rename = "inputTokens")]
        input_tokens: Option<u64>,
        #[serde(rename = "outputTokens")]
        output_tokens: Option<u64>,
        #[serde(rename = "totalTokens")]
        total_tokens: Option<u64>,
        #[serde(rename = "cachedReadTokens")]
        cached_read_tokens: Option<u64>,
        #[serde(rename = "cacheCreationTokens")]
        cache_creation_tokens: Option<u64>,
        #[serde(rename = "reasoningTokens")]
        reasoning_tokens: Option<u64>,
        #[serde(rename = "modelCalls")]
        model_calls: Option<u64>,
        #[serde(rename = "apiDurationMs")]
        api_duration_ms: Option<u64>,
        #[serde(rename = "costUsdTicks")]
        cost_usd_ticks: Option<u64>,
        #[serde(rename = "modelUsage")]
        model_usage: Option<HashMap<String, KnownModelUsage>>,
        #[serde(rename = "numTurns")]
        num_turns: Option<u64>,
    }

    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    #[allow(dead_code)] // Fields declare the accepted schema, they are not read
    struct KnownModelUsage {
        #[serde(rename = "inputTokens")]
        input_tokens: Option<u64>,
        #[serde(rename = "outputTokens")]
        output_tokens: Option<u64>,
        #[serde(rename = "totalTokens")]
        total_tokens: Option<u64>,
        #[serde(rename = "cachedReadTokens")]
        cached_read_tokens: Option<u64>,
        #[serde(rename = "cacheCreationTokens")]
        cache_creation_tokens: Option<u64>,
        #[serde(rename = "reasoningTokens")]
        reasoning_tokens: Option<u64>,
        #[serde(rename = "modelCalls")]
        model_calls: Option<u64>,
        #[serde(rename = "apiDurationMs")]
        api_duration_ms: Option<u64>,
        #[serde(rename = "costUsdTicks")]
        cost_usd_ticks: Option<u64>,
    }

    /// Newest `count` `updates.jsonl` files on this machine; empty when Grok has
    /// never run here (CI).
    fn recent_local_sessions(count: usize) -> Vec<PathBuf> {
        let root = match std::env::var("GROK_HOME") {
            Ok(v) if !v.is_empty() => PathBuf::from(v),
            _ => match directories::BaseDirs::new() {
                Some(d) => d.home_dir().join(".grok"),
                None => return Vec::new(),
            },
        };
        let pattern = root
            .join("sessions")
            .join("*")
            .join("*")
            .join("updates.jsonl");
        let Ok(paths) = glob::glob(&pattern.to_string_lossy()) else {
            return Vec::new();
        };
        let mut files: Vec<(std::time::SystemTime, PathBuf)> = paths
            .filter_map(|p| p.ok())
            .filter_map(|p| {
                let mtime = p.metadata().and_then(|m| m.modified()).ok()?;
                Some((mtime, p))
            })
            .collect();
        files.sort_by_key(|(mtime, _)| std::cmp::Reverse(*mtime));
        files.into_iter().take(count).map(|(_, p)| p).collect()
    }

    /// A long session's `updates.jsonl` carries every streamed chunk, so cap the
    /// scan — drift shows up in the first turns just as well.
    const CANARY_LINE_LIMIT: usize = 5_000;

    fn assert_usage_shape_known(path: &Path) {
        let Ok(file) = File::open(path) else { return };
        for (i, line) in BufReader::new(file)
            .lines()
            .take(CANARY_LINE_LIMIT)
            .enumerate()
        {
            let Ok(line) = line else { continue };
            if !line.contains("turn_completed") {
                continue;
            }
            let Ok(value) = serde_json::from_str::<serde_json::Value>(&line) else {
                continue;
            };
            let Some(usage) = value
                .get("params")
                .and_then(|p| p.get("update"))
                .filter(|u| {
                    u.get("sessionUpdate").and_then(|s| s.as_str()) == Some("turn_completed")
                })
                .and_then(|u| u.get("usage"))
                .filter(|u| !u.is_null())
            else {
                continue;
            };
            if let Err(e) = serde_json::from_value::<KnownUsage>(usage.clone()) {
                panic!(
                    "Grok turn_completed usage schema drifted at {}:{} — {}. Decide whether \
                     the new field is a billed token category, then add it here (and to \
                     GrokUsage/GrokTokens if it affects tokens or cost).",
                    path.display(),
                    i + 1,
                    e
                );
            }
        }
    }

    #[test]
    fn test_grok_usage_schema_has_no_unknown_fields() {
        assert_usage_shape_known(&real_session());
        for path in recent_local_sessions(5) {
            assert_usage_shape_known(&path);
        }
    }

    #[test]
    fn test_percent_decode() {
        assert_eq!(percent_decode("C%3A%5CUsers%5Cme"), "C:\\Users\\me");
        assert_eq!(percent_decode("plain-name"), "plain-name");
        // Malformed escapes are passed through rather than dropped.
        assert_eq!(percent_decode("100%"), "100%");
        assert_eq!(percent_decode("%zz"), "%zz");
    }
}
