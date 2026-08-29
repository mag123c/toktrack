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
    /// `None` when the record carries no `costUsdTicks`. A missing figure must
    /// stay unknown rather than collapse to a confident $0.00: `cost_usd` is
    /// treated downstream as upstream-priced, so a defaulted zero would be
    /// reported as exact spend and would suppress the estimated-cost fallback.
    #[serde(default)]
    cost_usd_ticks: Option<u64>,
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
    cost_usd_ticks: Option<u64>,
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
        ctx: &TurnContext<'_>,
    ) -> UsageEntry {
        let mapped = map_tokens(tokens, model, ctx.path);

        // `eventId` is unique per session. When the record carries no `_meta`,
        // fall back to `prompt_id` so the turn is dedupable at all: entries
        // without a hash bypass the dedup set entirely. One `turn_completed`
        // per `prompt_id` is the documented shape but is assumed here, not
        // enforced — if it ever fails, two turns collide and one is dropped.
        // A record with neither seed stays undedupable.
        let dedup_seed = ctx.event_id.or(ctx.prompt_id);

        UsageEntry {
            fast_speed: false,
            timestamp: ctx.timestamp,
            model: model.map(String::from),
            input_tokens: mapped.input_tokens,
            output_tokens: mapped.output_tokens,
            cache_read_tokens: tokens.cached_read_tokens,
            cache_creation_tokens: tokens.cache_creation_tokens,
            reasoning_tokens: tokens.reasoning_tokens,
            cache_creation_5m_tokens: 0,
            cache_creation_1h_tokens: 0,
            web_search_requests: 0,
            web_fetch_requests: 0,
            reported_total_tokens: mapped.reported_total_tokens,
            cost_usd: tokens
                .cost_usd_ticks
                .map(|t| t as f64 / COST_USD_TICKS_PER_USD),
            // The model is appended so a multi-model turn fans out into entries
            // with distinct dedup hashes.
            message_id: dedup_seed.map(|e| match model {
                Some(m) => format!("{}#{}", e, m),
                None => e.to_string(),
            }),
            request_id: ctx.prompt_id.map(String::from),
            source: Some("grok".into()),
            provider: Some("xai".into()),
            project: ctx.project.map(String::from),
        }
    }
}

/// Everything about a `turn_completed` record that every model slice shares.
struct TurnContext<'a> {
    timestamp: DateTime<Utc>,
    event_id: Option<&'a str>,
    prompt_id: Option<&'a str>,
    project: Option<&'a str>,
    /// Only used to name the file in drift warnings.
    path: &'a Path,
}

/// The v2 fields derived from one `GrokTokens`, plus the upstream total when it
/// still reconciles with them.
struct MappedTokens {
    input_tokens: u64,
    output_tokens: u64,
    reported_total_tokens: Option<u64>,
}

/// Which nesting assumption a record contradicted. Two different upstream
/// shapes reach the same fallback, and naming the wrong one in the warning
/// sends a bug report down the wrong path.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
enum NestingDrift {
    /// `cachedReadTokens` alone exceeds `inputTokens`.
    CachedReadExceedsInput,
    /// Cache writes appear to be additional rather than nested.
    CacheCreationNotNested,
}

/// Map Grok's nested counts onto the v2 contract.
///
/// `inputTokens` includes `cachedReadTokens` and `outputTokens` includes
/// `reasoningTokens`, both proven by `totalTokens == inputTokens +
/// outputTokens` holding exactly on every sampled turn. `cacheCreationTokens`
/// is assumed to nest the same way, which no sampled turn could confirm because
/// all of them reported 0.
///
/// When that assumption fails the counts cannot all be satisfied at once: cache
/// writes would live outside `inputTokens` while `totalTokens` still excludes
/// them, so no mapping reconciles. Rather than clamp to a number that quietly
/// violates the documented invariant, keep the counts truthful and drop the
/// reconciliation total, which exists only to be compared against.
/// Which nesting assumption a record contradicts, if any. Split out so the
/// branch is assertable without capturing stderr.
fn nesting_drift(tokens: &GrokTokens) -> Option<NestingDrift> {
    // Saturating: these are numbers from a file on disk, and a parser panic
    // propagates out of the rayon pool in `parse_and_dedup` and takes every
    // other source down with it.
    let nested = tokens
        .cached_read_tokens
        .saturating_add(tokens.cache_creation_tokens);
    if nested <= tokens.input_tokens {
        return None;
    }
    Some(if tokens.cached_read_tokens > tokens.input_tokens {
        NestingDrift::CachedReadExceedsInput
    } else {
        NestingDrift::CacheCreationNotNested
    })
}

fn map_tokens(tokens: &GrokTokens, model: Option<&str>, path: &Path) -> MappedTokens {
    let visible_output = tokens.output_tokens.saturating_sub(tokens.reasoning_tokens);

    let Some(drift) = nesting_drift(tokens) else {
        let nested = tokens
            .cached_read_tokens
            .saturating_add(tokens.cache_creation_tokens);
        return MappedTokens {
            input_tokens: tokens.input_tokens - nested,
            output_tokens: visible_output,
            reported_total_tokens: Some(tokens.total_tokens),
        };
    };

    match drift {
        NestingDrift::CachedReadExceedsInput => eprintln!(
            "[toktrack] Warning: Grok cachedReadTokens ({}) exceeds inputTokens ({}) \
             for model {:?} in {:?}, which contradicts the documented nesting. \
             Dropping the reconciliation total for this turn.",
            tokens.cached_read_tokens,
            tokens.input_tokens,
            model.unwrap_or("unknown"),
            path
        ),
        NestingDrift::CacheCreationNotNested => eprintln!(
            "[toktrack] Warning: Grok cacheCreationTokens ({}) is not nested inside \
             inputTokens ({}, cachedRead {}) for model {:?} in {:?}. Counting cache \
             writes as additional tokens and dropping the reconciliation total — \
             please report this session shape.",
            tokens.cache_creation_tokens,
            tokens.input_tokens,
            tokens.cached_read_tokens,
            model.unwrap_or("unknown"),
            path
        ),
    }

    MappedTokens {
        input_tokens: tokens
            .input_tokens
            .saturating_sub(tokens.cached_read_tokens),
        output_tokens: visible_output,
        reported_total_tokens: None,
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
        let (entries, dropped) = self.parse_entries(path)?;

        // Every dropped turn is usage the user spent and will never see again,
        // which is the worst failure mode for a cost tracker — the timestamp
        // path warns for the same reason. Reported once per file so a
        // systematically drifted schema is loud without becoming per-line spam.
        if dropped > 0 {
            eprintln!(
                "[toktrack] Warning: {} Grok turn-completion line(s) in {:?} could not be \
                 parsed and were skipped; their tokens and cost are missing from the totals",
                dropped, path
            );
        }

        Ok(entries)
    }
}

impl GrokParser {
    /// `parse_file`'s body, additionally returning how many `turn_completed`
    /// lines were unreadable, so the warning is observable to tests.
    fn parse_entries(&self, path: &Path) -> Result<(Vec<UsageEntry>, usize)> {
        let file = File::open(path).map_err(ToktrackError::Io)?;
        let mut reader = BufReader::new(file);
        let (project, fallback_model) = self.session_context(path);
        let mut entries = Vec::new();
        let mut dropped = 0usize;

        // `read_line` keeps the terminator, so the last line read tells us
        // whether it was complete. Sampling the file's last byte separately
        // would race the CLI appending between the sample and EOF.
        let mut last_line_terminated = true;
        let mut last_dropped_was_final = false;
        let mut raw = String::new();

        loop {
            raw.clear();
            match reader.read_line(&mut raw) {
                Ok(0) => break,
                Ok(_) => {}
                Err(_) => break,
            }
            last_line_terminated = raw.ends_with('\n');
            last_dropped_was_final = false;
            let line = raw.trim_end_matches(['\n', '\r']).to_string();
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
                Err(_) => {
                    dropped += 1;
                    last_dropped_was_final = true;
                    continue;
                }
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

            let ctx = TurnContext {
                timestamp,
                event_id,
                prompt_id: update.prompt_id,
                project: project.as_deref(),
                path,
            };

            match &usage.model_usage {
                // Per-model breakdown present: one entry per model, which keeps
                // attribution (and cost) exact for turns that switch models.
                Some(map) if !map.is_empty() => {
                    let mut models: Vec<&String> = map.keys().collect();
                    models.sort(); // deterministic order across runs
                    for model in models {
                        entries.push(self.entry_for_model(&map[model], Some(model.as_str()), &ctx));
                    }
                }
                // No breakdown: attribute the turn totals to the session model.
                _ => entries.push(self.entry_for_model(
                    &usage.totals(),
                    fallback_model.as_deref(),
                    &ctx,
                )),
            }
        }

        // Grok appends to this file while the CLI runs, so the last line is
        // routinely half-written. An unterminated final line was still being
        // written; a terminated one that will not parse is real lost usage.
        if !last_line_terminated && last_dropped_was_final {
            dropped -= 1;
        }

        Ok((entries, dropped))
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

    fn posix_session() -> PathBuf {
        root()
            .join("sessions")
            .join("%2Fhome%2Fme%2Fproj")
            .join("0f0f0f0f-4444-4555-8666-777777777777")
            .join("updates.jsonl")
    }

    fn edge_case_session() -> PathBuf {
        root()
            .join("sessions")
            .join("edge-cases-e5f6a7b8")
            .join("0f0f0f0f-5555-4666-8777-888888888888")
            .join("updates.jsonl")
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

    /// The upstream totals must survive the mapping verbatim. Asserting
    /// `reported_total_tokens == total_tokens()` here would prove nothing: that
    /// identity holds for any record whose nested subtractions do not clamp.
    #[test]
    fn test_reported_totals_are_carried_verbatim() {
        let entries = parser().parse_file(&real_session()).unwrap();
        let reported: Vec<Option<u64>> = entries.iter().map(|e| e.reported_total_tokens).collect();
        assert_eq!(
            reported,
            vec![Some(83703), Some(125879), Some(229491)],
            "each turn's upstream totalTokens, in file order"
        );
    }

    /// Cache writes nesting inside `inputTokens` is assumed, not proven — every
    /// sampled turn reported 0. If a real session ever contradicts it, the counts
    /// must stay truthful and the reconciliation total must be withdrawn rather
    /// than clamped into a number that violates the documented v2 invariant.
    #[test]
    fn test_additive_cache_creation_drops_reconciliation() {
        let tokens = GrokTokens {
            input_tokens: 100,
            output_tokens: 10,
            total_tokens: 110,
            cached_read_tokens: 60,
            cache_creation_tokens: 80,
            reasoning_tokens: 4,
            cost_usd_ticks: Some(1_000),
        };
        let ctx = TurnContext {
            timestamp: DateTime::from_timestamp(1_756_454_400, 0).unwrap(),
            event_id: Some("evt-additive"),
            prompt_id: Some("prompt-additive"),
            project: None,
            path: Path::new("synthetic"),
        };
        let e = parser().entry_for_model(&tokens, Some("grok-4.6"), &ctx);

        // cachedRead stays nested (proven); cacheCreation is counted on top.
        assert_eq!(e.input_tokens, 40);
        assert_eq!(e.cache_read_tokens, 60);
        assert_eq!(e.cache_creation_tokens, 80);
        assert_eq!(e.output_tokens, 6);
        assert_eq!(e.reasoning_tokens, 4);
        assert_eq!(e.total_tokens(), 190);
        assert_eq!(
            e.reported_total_tokens, None,
            "an unreconcilable upstream total must be withdrawn, not clamped"
        );
        assert_eq!(
            nesting_drift(&tokens),
            Some(NestingDrift::CacheCreationNotNested)
        );
    }

    /// The nested case must keep reporting the total, so the withdrawal above is
    /// a genuine branch rather than the parser giving up everywhere.
    #[test]
    fn test_nested_cache_creation_keeps_reconciliation() {
        let tokens = GrokTokens {
            input_tokens: 100,
            output_tokens: 10,
            total_tokens: 110,
            cached_read_tokens: 60,
            cache_creation_tokens: 30,
            reasoning_tokens: 4,
            cost_usd_ticks: Some(1_000),
        };
        let ctx = TurnContext {
            timestamp: DateTime::from_timestamp(1_756_454_400, 0).unwrap(),
            event_id: Some("evt-nested"),
            prompt_id: Some("prompt-nested"),
            project: None,
            path: Path::new("synthetic"),
        };
        let e = parser().entry_for_model(&tokens, Some("grok-4.6"), &ctx);

        assert_eq!(e.input_tokens, 10);
        assert_eq!(e.reported_total_tokens, Some(110));
        assert_eq!(e.total_tokens(), 110);
        assert_eq!(nesting_drift(&tokens), None);
    }

    /// These counts come off disk, so any arithmetic on them must survive
    /// absurd values. `parse_and_dedup` runs parsers under `par_iter`, so a
    /// panic here would abort the load for every other source too.
    #[test]
    fn test_absurd_token_counts_do_not_panic() {
        let tokens = GrokTokens {
            input_tokens: 100,
            output_tokens: 10,
            total_tokens: 110,
            cached_read_tokens: u64::MAX,
            cache_creation_tokens: u64::MAX,
            reasoning_tokens: u64::MAX,
            cost_usd_ticks: Some(u64::MAX),
        };
        let ctx = TurnContext {
            timestamp: DateTime::from_timestamp(1_756_454_400, 0).unwrap(),
            event_id: Some("evt-absurd"),
            prompt_id: Some("prompt-absurd"),
            project: None,
            path: Path::new("synthetic"),
        };
        let e = parser().entry_for_model(&tokens, Some("grok-4.6"), &ctx);
        assert_eq!(e.input_tokens, 0);
        assert_eq!(e.output_tokens, 0);
        assert_eq!(e.reported_total_tokens, None);
        // The raw counts pass through unclamped, so the aggregation the entry
        // feeds has to survive them too.
        assert_eq!(e.total_tokens(), u64::MAX);
        assert_eq!(
            nesting_drift(&tokens),
            Some(NestingDrift::CachedReadExceedsInput)
        );
    }

    /// A record without `costUsdTicks` is unpriced, not free. Defaulting it to
    /// `Some(0.0)` would be reported as exact upstream spend and would suppress
    /// the estimated-pricing fallback.
    #[test]
    fn test_missing_cost_ticks_is_unknown_not_zero() {
        let tokens = GrokTokens {
            input_tokens: 500,
            output_tokens: 20,
            total_tokens: 520,
            cached_read_tokens: 0,
            cache_creation_tokens: 0,
            reasoning_tokens: 0,
            cost_usd_ticks: None,
        };
        let ctx = TurnContext {
            timestamp: DateTime::from_timestamp(1_756_454_400, 0).unwrap(),
            event_id: Some("evt-nocost"),
            prompt_id: Some("prompt-nocost"),
            project: None,
            path: Path::new("synthetic"),
        };
        let e = parser().entry_for_model(&tokens, Some("grok-4.6"), &ctx);
        assert_eq!(e.cost_usd, None);

        // A genuine zero still reports as zero, so the two stay distinguishable.
        let free = GrokTokens {
            cost_usd_ticks: Some(0),
            ..tokens
        };
        let e = parser().entry_for_model(&free, Some("grok-4.6"), &ctx);
        assert_eq!(e.cost_usd, Some(0.0));
    }

    #[test]
    fn test_cost_from_ticks() {
        let entries = parser().parse_file(&real_session()).unwrap();
        let costs: Vec<f64> = entries.iter().map(|e| e.cost_usd.unwrap()).collect();
        let want = [0.110870, 0.132250, 0.160058];
        assert_eq!(costs.len(), want.len(), "one cost per turn");
        for (got, want) in costs.iter().zip(want) {
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
        assert_eq!(
            entries.len(),
            9,
            "3 + 2 (multi-model) + 1 + 2 (POSIX-encoded) + 1 (corrupt final \
             line); the edge-case and empty sessions contribute none"
        );
    }

    /// Session directories whose name is a POSIX path are the common shape on
    /// macOS and Linux, where the Windows-style fixtures say nothing.
    #[test]
    fn test_project_from_posix_encoded_dir() {
        let entries = parser().parse_file(&posix_session()).unwrap();
        assert_eq!(entries.len(), 2);
        for e in &entries {
            assert_eq!(e.project.as_deref(), Some("/home/me/proj"));
        }
        assert_eq!(entries[0].model.as_deref(), Some("grok-4.6"));
        assert_eq!(entries[0].input_tokens, 3000);
        let cost = entries[0].cost_usd.unwrap();
        assert!((cost - 0.0153).abs() < 1e-9, "cost {} != 0.0153", cost);
    }

    /// The `Option<u64>` on `costUsdTicks` has to survive deserialization, not
    /// just direct construction: an absent field must reach the entry as `None`.
    #[test]
    fn test_absent_cost_ticks_survives_deserialization() {
        let entries = parser().parse_file(&posix_session()).unwrap();
        let unpriced = &entries[1];
        assert_eq!(
            unpriced.request_id.as_deref(),
            Some("1a2b3c4d-5e6f-4a7b-8c9d-0e1f2a3b4c5d")
        );
        assert_eq!(
            unpriced.cost_usd, None,
            "a turn whose usage omits costUsdTicks must stay unpriced"
        );
        // The turn's tokens are still counted.
        assert_eq!(unpriced.input_tokens, 1500);
        assert_eq!(unpriced.cache_read_tokens, 500);
        // No summary.json and no modelUsage, so the model is unknown and the
        // dedup seed carries no model suffix.
        assert_eq!(unpriced.model, None);
        assert_eq!(
            unpriced.message_id.as_deref(),
            Some("1a2b3c4d-5e6f-4a7b-8c9d-0e1f2a3b4c5d")
        );
    }

    /// Corrupt, empty, and half-written sessions must degrade to "no entries"
    /// rather than panicking or aborting the whole load.
    #[test]
    fn test_corrupt_and_empty_sessions_yield_no_entries() {
        assert!(
            parser()
                .parse_file(&edge_case_session())
                .unwrap()
                .is_empty(),
            "no entry may be derived from a session of unusable records"
        );

        let empty = root()
            .join("sessions")
            .join("empty-session-c9d0e1f2")
            .join("0f0f0f0f-6666-4777-8888-999999999999")
            .join("updates.jsonl");
        assert!(parser().parse_file(&empty).unwrap().is_empty());
    }

    /// A half-written final line is normal (Grok appends live), so it must not
    /// be reported as lost usage. The fixture genuinely lacks a trailing
    /// newline, which is what distinguishes a write in progress from a corrupt
    /// record.
    #[test]
    fn test_truncated_final_line_is_not_reported_as_dropped() {
        let (entries, dropped) = parser().parse_entries(&posix_session()).unwrap();
        assert_eq!(entries.len(), 2, "completed turns survive truncation");
        assert_eq!(dropped, 0, "a line still being written is not lost usage");
    }

    /// A malformed line that is not the one in flight is a turn whose tokens and
    /// cost are gone, and the user has to be told.
    #[test]
    fn test_unparsable_turn_is_counted_as_dropped() {
        let (entries, dropped) = parser().parse_entries(&edge_case_session()).unwrap();
        assert!(entries.is_empty());
        assert_eq!(
            dropped, 1,
            "the malformed mid-file turn_completed line must be reported"
        );
    }

    /// The distinction that decides whether a broken final line is reported is
    /// the newline: this fixture's last line is malformed *and* terminated, so
    /// nothing is being written and the turn is genuinely lost.
    #[test]
    fn test_terminated_but_corrupt_final_line_is_counted_as_dropped() {
        let path = root()
            .join("sessions")
            .join("corrupt-final-line-b3c4d5e6")
            .join("0f0f0f0f-7777-4888-8999-aaaabbbbcccc")
            .join("updates.jsonl");
        let (entries, dropped) = parser().parse_entries(&path).unwrap();
        assert_eq!(entries.len(), 1, "the completed turn is still counted");
        assert_eq!(
            dropped, 1,
            "a terminated final line that will not parse is lost usage, not a \
             write in progress"
        );
    }

    /// The v2 invariant must hold on *every* fixture, not just the real session.
    ///
    /// This is the only test exercising a non-zero `cacheCreationTokens` (50, in
    /// the multi-model fixture), because all three real sampled turns had 0.
    /// NOTE: it proves the mapping is self-consistent under the assumption that
    /// `cacheCreationTokens` nests inside `inputTokens`; it cannot confirm that
    /// assumption, since the fixture's `totalTokens` was authored to match.
    /// Re-check against a real session that performs cache writes.
    #[test]
    fn test_reported_total_reconciles_across_all_fixtures() {
        let entries = parser().parse_all().unwrap();
        assert!(
            entries.iter().any(|e| e.cache_creation_tokens > 0),
            "fixture set must exercise a non-zero cache_creation_tokens"
        );
        assert!(
            entries.iter().any(|e| e.reported_total_tokens.is_some()),
            "the check below is vacuous if no fixture reports a total"
        );
        // `None` is legal: `map_tokens` withdraws the total when the upstream
        // counts cannot reconcile, so assert the invariant where it applies
        // rather than forbidding a fixture that exercises the other branch.
        for e in &entries {
            if let Some(reported) = e.reported_total_tokens {
                assert_eq!(
                    reported,
                    e.total_tokens(),
                    "invariant broken for model {:?}",
                    e.model
                );
            }
        }
    }

    /// Every entry must carry a dedup hash, and re-parsing must not double-count:
    /// Grok appends to a live session file, so the warm path re-reads it whole.
    #[test]
    fn test_entries_are_dedupable_and_reparse_is_idempotent() {
        let first: Vec<String> = parser()
            .parse_all()
            .unwrap()
            .iter()
            .map(|e| e.dedup_hash().expect("every entry needs a dedup hash"))
            .collect();

        let mut unique: Vec<String> = first.clone();
        unique.sort();
        unique.dedup();
        assert_eq!(unique.len(), first.len(), "dedup hashes must be unique");

        // Re-read the same files the way the warm path does: the second pass
        // must produce hashes the first pass already claimed, so the loader's
        // `seen` set collapses them instead of counting the turns twice.
        let second: Vec<String> = parser()
            .parse_all()
            .unwrap()
            .iter()
            .map(|e| e.dedup_hash().expect("every entry needs a dedup hash"))
            .collect();
        assert_eq!(first, second, "re-parsing must reproduce the same hashes");
    }

    /// `_meta.eventId` is the usual dedup seed, but it is absent on some records.
    /// Falling through to `None` there would make the turn undedupable, and the
    /// loader keeps unhashed entries — so a re-read would double-count it.
    #[test]
    fn test_dedup_hash_falls_back_to_prompt_id_without_meta() {
        let entries = parser().parse_file(&posix_session()).unwrap();
        let e = &entries[0];
        assert_eq!(
            e.message_id.as_deref(),
            Some("9c1d2e3f-4a5b-4c6d-8e7f-0a1b2c3d4e5f#grok-4.6"),
            "prompt_id seeds the hash when the record carries no _meta"
        );
        assert!(e.dedup_hash().is_some());
    }

    /// `recent_local_sessions` reads `GROK_HOME` too and cargo runs tests in
    /// parallel, so this points the override at the fixture root: a canary that
    /// observes it mid-run still scans real sessions instead of an empty dir.
    #[test]
    fn test_grok_home_env_override() {
        let saved = std::env::var("GROK_HOME").ok();
        std::env::set_var("GROK_HOME", root().as_os_str());
        let got = GrokParser::new().data_dir().to_path_buf();
        match saved {
            Some(v) => std::env::set_var("GROK_HOME", v),
            None => std::env::remove_var("GROK_HOME"),
        }
        assert_eq!(got, root());
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

    /// Returns how many `turn_completed` usage objects were actually inspected,
    /// so a caller can tell "the schema is unchanged" apart from "nothing was
    /// looked at". `required` marks the checked-in fixture, whose absence must
    /// fail: a moved or renamed path would otherwise turn this canary into a
    /// permanently-green no-op. Real machine sessions stay optional (CI has none).
    fn assert_usage_shape_known(path: &Path, required: bool) -> usize {
        let file = match File::open(path) {
            Ok(f) => f,
            Err(e) if required => {
                panic!("canary fixture {:?} could not be opened: {}", path, e)
            }
            Err(_) => return 0,
        };
        let mut inspected = 0usize;
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
            inspected += 1;
        }
        inspected
    }

    #[test]
    fn test_grok_usage_schema_has_no_unknown_fields() {
        let inspected = assert_usage_shape_known(&real_session(), true);
        // The canary reaches records through the same prefilter and
        // `sessionUpdate` value the parser uses. If xAI renames either, the
        // parser silently yields nothing for every Grok session — and without
        // this assertion the canary would iterate nothing and agree.
        assert!(
            inspected > 0,
            "canary inspected no turn_completed usage records in the fixture; \
             the prefilter or the sessionUpdate value has drifted"
        );
        for path in recent_local_sessions(5) {
            assert_usage_shape_known(&path, false);
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
