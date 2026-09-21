//! Codex CLI JSONL parser

use crate::types::{Result, ToktrackError, UsageEntry};
use chrono::{DateTime, Utc};
use serde::Deserialize;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

use super::CLIParser;

/// Codex JSONL line types
#[derive(Deserialize)]
struct CodexJsonLine<'a> {
    #[serde(rename = "type")]
    line_type: &'a str,
    timestamp: &'a str,
    #[serde(default)]
    payload: Option<CodexPayload>,
}

#[derive(Deserialize)]
struct CodexPayload {
    #[serde(rename = "type")]
    payload_type: Option<String>,
    #[serde(default)]
    thread_settings: Option<serde_json::Value>,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    info: Option<CodexInfo>,
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    model_provider: Option<String>,
    /// Working directory — present on `turn_context` (and sometimes
    /// `session_meta`) payloads; used as the project identifier.
    #[serde(default)]
    cwd: Option<String>,
}

#[derive(Deserialize)]
struct CodexInfo {
    total_token_usage: Option<CodexTokenUsage>,
    #[serde(default)]
    last_token_usage: Option<CodexTokenUsage>,
}

#[derive(Deserialize, Clone)]
struct CodexTokenUsage {
    input_tokens: u64,
    output_tokens: u64,
    #[serde(default)]
    cached_input_tokens: u64,
}

/// Raw token data extracted from a token_count event
struct TokenCountData {
    timestamp: DateTime<Utc>,
    total: CodexTokenUsage,
    last: Option<CodexTokenUsage>,
}

/// Directory name of a standard Codex session root. Remote snapshots mirror this
/// name so `collect_files` finds the archive beside it (`crate::services::remote`).
pub const SESSIONS_DIR: &str = "sessions";

/// Sibling directory Codex moves finished sessions into.
pub const ARCHIVED_SESSIONS_DIR: &str = "archived_sessions";

/// Parser for Codex CLI usage data
pub struct CodexParser {
    data_dir: PathBuf,
}

impl CodexParser {
    /// Create a new parser with default data directory.
    ///
    /// Honors `CODEX_HOME` (root; replaces `~/.codex`); sessions live under
    /// `<root>/sessions` and `<root>/archived_sessions`.
    pub fn new() -> Self {
        let root = super::discovery::first_env_dir(&["CODEX_HOME"]).unwrap_or_else(|| {
            directories::BaseDirs::new()
                .map(|d| d.home_dir().join(".codex"))
                .unwrap_or_else(|| {
                    eprintln!("[toktrack] Warning: Could not determine home directory");
                    PathBuf::from(".")
                })
        });
        Self {
            data_dir: root.join(SESSIONS_DIR),
        }
    }

    /// Create a parser with a custom data directory (for testing)
    #[allow(dead_code)]
    pub fn with_data_dir(data_dir: PathBuf) -> Self {
        Self { data_dir }
    }

    /// Parse a single JSONL line
    fn parse_line(&self, line: &mut [u8]) -> ParseResult {
        if line.is_empty() {
            return ParseResult::Skip;
        }

        let data: CodexJsonLine = match simd_json::from_slice(line) {
            Ok(d) => d,
            Err(_) => return ParseResult::Skip,
        };

        let payload = match &data.payload {
            Some(p) => p,
            None => return ParseResult::Skip,
        };

        if data.line_type == "turn_context" {
            if payload.model.is_none() && payload.cwd.is_none() {
                return ParseResult::Skip;
            }
            return ParseResult::TurnContext {
                model: payload.model.clone(),
                cwd: payload.cwd.clone(),
            };
        }

        if data.line_type == "session_meta" {
            if payload.id.is_none() && payload.model_provider.is_none() && payload.cwd.is_none() {
                return ParseResult::Skip;
            }
            return ParseResult::SessionMeta {
                id: payload.id.clone(),
                provider: payload.model_provider.clone(),
                cwd: payload.cwd.clone(),
            };
        }

        if data.line_type != "event_msg" {
            return ParseResult::Skip;
        }

        let payload_type = match &payload.payload_type {
            Some(t) => t,
            None => return ParseResult::Skip,
        };

        if payload_type == "thread_settings_applied" {
            // Settings are snapshots. Missing, null, or unknown tiers must clear
            // an earlier Fast setting rather than silently carrying it forward.
            let fast = payload
                .thread_settings
                .as_ref()
                .and_then(|settings| settings.get("service_tier"))
                .and_then(serde_json::Value::as_str)
                .is_some_and(|tier| matches!(tier, "priority" | "fast"));
            return ParseResult::Speed(fast);
        }

        if payload_type != "token_count" {
            return ParseResult::Skip;
        }

        let info = match &payload.info {
            Some(i) => i,
            None => return ParseResult::Skip,
        };

        let total = match &info.total_token_usage {
            Some(u) => u.clone(),
            None => return ParseResult::Skip,
        };

        let timestamp = match DateTime::parse_from_rfc3339(data.timestamp) {
            Ok(dt) => dt.with_timezone(&Utc),
            Err(_) => {
                eprintln!(
                    "[toktrack] Warning: Invalid timestamp '{}', skipping entry",
                    data.timestamp
                );
                return ParseResult::Skip;
            }
        };

        ParseResult::TokenCount(TokenCountData {
            timestamp,
            total,
            last: info.last_token_usage.clone(),
        })
    }
}

/// Result of parsing a single line
enum ParseResult {
    Skip,
    Speed(bool),
    TurnContext {
        model: Option<String>,
        cwd: Option<String>,
    },
    SessionMeta {
        id: Option<String>,
        provider: Option<String>,
        cwd: Option<String>,
    },
    TokenCount(TokenCountData),
}

impl Default for CodexParser {
    fn default() -> Self {
        Self::new()
    }
}

impl CLIParser for CodexParser {
    fn name(&self) -> &str {
        "codex"
    }

    fn data_dir(&self) -> &Path {
        &self.data_dir
    }

    fn file_pattern(&self) -> &str {
        "**/*.jsonl"
    }

    // `retroactive_reconciliation` is deliberately left at the default: resumed
    // sessions make old-dated entries in a recent file the norm here, so opting
    // in would force a full reparse on nearly every run. See
    // `test_codex_does_not_reconcile_retroactively`.

    fn collect_files(&self) -> Vec<PathBuf> {
        let collect = |dir: &Path| super::glob_patterns_under(dir, &[self.file_pattern()]);
        let mut files = collect(&self.data_dir);
        // Only standard session roots have a sibling archive directory.
        if self.data_dir.file_name() == Some(std::ffi::OsStr::new(SESSIONS_DIR)) {
            if let Some(root) = self.data_dir.parent() {
                // Archiving flattens date directories; rollout filenames remain stable.
                let active_names: std::collections::HashSet<_> = files
                    .iter()
                    .filter_map(|path| path.file_name().map(|name| name.to_os_string()))
                    .collect();
                files.extend(
                    collect(&root.join(ARCHIVED_SESSIONS_DIR))
                        .into_iter()
                        .filter(|path| {
                            !path
                                .file_name()
                                .is_some_and(|name| active_names.contains(name))
                        }),
                );
            }
        }
        files
    }

    fn parse_file(&self, path: &Path) -> Result<Vec<UsageEntry>> {
        let file = File::open(path).map_err(ToktrackError::Io)?;
        let reader = BufReader::new(file);
        let mut entries: Vec<UsageEntry> = Vec::new();
        let mut current_model: Option<String> = None;
        let mut fast_speed = false;
        let mut session_id: Option<String> = None;
        let mut current_provider: Option<String> = None;
        let mut current_project: Option<String> = None;
        let mut prev_totals = CodexTokenUsage {
            input_tokens: 0,
            output_tokens: 0,
            cached_input_tokens: 0,
        };
        // Codex re-emits the same token_count event (byte-identical total AND
        // last usage) within one response, so consecutive identical snapshots
        // must collapse. Both fields key the identity: `last` alone would also
        // drop a turn whose `total` genuinely advanced while `last` repeated.
        let mut prev_key: Option<(u64, u64, u64, u64, u64, u64)> = None;

        for line_result in reader.lines() {
            let line = match line_result {
                Ok(l) => l,
                Err(_) => continue,
            };

            if line.is_empty() {
                continue;
            }

            let mut line_bytes = line.into_bytes();
            match self.parse_line(&mut line_bytes) {
                ParseResult::Skip => {}
                ParseResult::Speed(fast) => fast_speed = fast,
                ParseResult::TurnContext { model, cwd } => {
                    if model.is_some() {
                        current_model = model;
                    }
                    if cwd.is_some() {
                        current_project = cwd;
                    }
                }
                ParseResult::SessionMeta { id, provider, cwd } => {
                    if let Some(id) = id {
                        session_id = Some(id);
                    }
                    // Replace unconditionally: if a later session_meta lacks
                    // model_provider_id, the previous provider must NOT stick —
                    // sticking would silently misattribute billing.
                    current_provider = provider;
                    if cwd.is_some() {
                        current_project = cwd;
                    }
                    // Re-emission identity is scoped to one session.
                    prev_key = None;
                }
                ParseResult::TokenCount(data) => {
                    // Collapse byte-identical re-emissions before anything else.
                    // Identity is (total, last); a repeat of `last` with an
                    // advancing `total` is a distinct turn and must survive.
                    let key = data.last.as_ref().map(|l| {
                        (
                            data.total.input_tokens,
                            data.total.output_tokens,
                            data.total.cached_input_tokens,
                            l.input_tokens,
                            l.output_tokens,
                            l.cached_input_tokens,
                        )
                    });
                    if key.is_some() && prev_key == key {
                        prev_totals = data.total;
                        continue;
                    }
                    // An event without `last` has no identity; letting it
                    // overwrite the key would let the next re-emission through.
                    if key.is_some() {
                        prev_key = key;
                    }

                    // Compute delta: prefer last_token_usage, fallback to diff
                    let (delta_input, delta_output, delta_cached) =
                        if let Some(ref last) = data.last {
                            (
                                last.input_tokens,
                                last.output_tokens,
                                last.cached_input_tokens,
                            )
                        } else {
                            (
                                data.total
                                    .input_tokens
                                    .saturating_sub(prev_totals.input_tokens),
                                data.total
                                    .output_tokens
                                    .saturating_sub(prev_totals.output_tokens),
                                data.total
                                    .cached_input_tokens
                                    .saturating_sub(prev_totals.cached_input_tokens),
                            )
                        };

                    prev_totals = data.total;

                    if delta_input == 0 && delta_output == 0 && delta_cached == 0 {
                        continue;
                    }

                    // Normalize: input_tokens = non-cached only (Claude convention)
                    let non_cached_input = delta_input.saturating_sub(delta_cached);

                    entries.push(UsageEntry {
                        fast_speed,
                        timestamp: data.timestamp,
                        model: current_model.clone(),
                        input_tokens: non_cached_input,
                        output_tokens: delta_output,
                        cache_read_tokens: delta_cached,
                        cache_creation_tokens: 0,
                        reasoning_tokens: 0,
                        cache_creation_5m_tokens: 0,
                        cache_creation_1h_tokens: 0,
                        web_search_requests: 0,
                        web_fetch_requests: 0,
                        reported_total_tokens: None,
                        cost_usd: None,
                        message_id: session_id.clone(),
                        request_id: None,
                        source: Some("codex".into()),
                        provider: current_provider.clone(),
                        project: current_project.clone(),
                    });
                }
            }
        }

        Ok(entries)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn test_codex_fast_tier_transitions() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("session.jsonl");
        let mut lines = vec![];
        for (i, tier) in [
            None,
            Some("priority"),
            Some("default"),
            Some("fast"),
            Some("future"),
            Some("priority"),
            None,
        ]
        .iter()
        .enumerate()
        {
            if i > 0 {
                lines.push(serde_json::json!({"type":"event_msg", "timestamp":"2026-09-20T00:00:00Z", "payload":{"type":"thread_settings_applied", "thread_settings":{"service_tier":tier}}}).to_string());
            }
            lines.push(serde_json::json!({"type":"event_msg", "timestamp":"2026-09-20T00:00:01Z", "payload":{"type":"token_count", "info":{"total_token_usage":{"input_tokens":(i+1)*100,"output_tokens":(i+1)*10}}}}).to_string());
        }
        std::fs::write(&path, lines.join("\n")).unwrap();
        let entries = CodexParser::with_data_dir(temp.path().into())
            .parse_file(&path)
            .unwrap();
        assert_eq!(
            entries.iter().map(|e| e.fast_speed).collect::<Vec<_>>(),
            vec![false, true, false, true, false, true, false]
        );
        assert!(entries
            .iter()
            .all(|e| e.input_tokens == 100 && e.output_tokens == 10));
    }

    #[test]
    fn test_codex_fast_unknown_settings_clear_previous_tier() {
        use serde_json::json;
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("session.jsonl");
        for payload in [
            json!({"type":"thread_settings_applied"}),
            json!({"type":"thread_settings_applied", "thread_settings":null}),
            json!({"type":"thread_settings_applied", "thread_settings":[]}),
            json!({"type":"thread_settings_applied", "thread_settings":true}),
            json!({"type":"thread_settings_applied", "thread_settings":{}}),
            json!({"type":"thread_settings_applied", "thread_settings":{"service_tier":42}}),
            json!({"type":"thread_settings_applied", "thread_settings":{"service_tier":false}}),
        ] {
            let lines = [
                json!({"type":"event_msg", "timestamp":"2026-09-20T00:00:00Z", "payload":{"type":"thread_settings_applied", "thread_settings":{"service_tier":"priority"}}}),
                json!({"type":"event_msg", "timestamp":"2026-09-20T00:00:00Z", "payload":payload}),
                json!({"type":"event_msg", "timestamp":"2026-09-20T00:00:01Z", "payload":{"type":"token_count", "info":{"total_token_usage":{"input_tokens":100,"output_tokens":10}}}}),
            ];
            std::fs::write(
                &path,
                lines
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join("\n"),
            )
            .unwrap();
            let entries = CodexParser::with_data_dir(temp.path().into())
                .parse_file(&path)
                .unwrap();
            assert_eq!(entries.len(), 1, "{payload}");
            assert!(!entries[0].fast_speed, "{payload}");
        }
    }

    fn fixture_path(name: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("fixtures")
            .join("codex")
            .join(name)
    }

    #[test]
    fn test_parse_delta_sum_produces_per_turn_entries() {
        let parser = CodexParser::with_data_dir(PathBuf::from("tests/fixtures/codex"));
        let entries = parser
            .parse_file(&fixture_path("sample-session.jsonl"))
            .unwrap();

        assert_eq!(entries.len(), 2);
    }

    #[test]
    fn test_input_tokens_normalized_for_codex() {
        let parser = CodexParser::with_data_dir(PathBuf::from("tests/fixtures/codex"));
        let entries = parser
            .parse_file(&fixture_path("sample-session.jsonl"))
            .unwrap();

        let e1 = &entries[0];
        assert_eq!(e1.model, Some("o4-mini".to_string()));
        assert_eq!(e1.input_tokens, 125);
        assert_eq!(e1.output_tokens, 75);
        assert_eq!(e1.cache_read_tokens, 25);

        let e2 = &entries[1];
        assert_eq!(e2.model, Some("gpt-4.1".to_string()));
        assert_eq!(e2.input_tokens, 275);
        assert_eq!(e2.output_tokens, 125);
        assert_eq!(e2.cache_read_tokens, 75);
    }

    #[test]
    fn test_delta_sum_matches_expected() {
        let parser = CodexParser::with_data_dir(PathBuf::from("tests/fixtures/codex"));
        let entries = parser
            .parse_file(&fixture_path("multi-turn-session.jsonl"))
            .unwrap();

        assert_eq!(entries.len(), 3);

        assert_eq!(entries[0].input_tokens, 80);
        assert_eq!(entries[0].output_tokens, 50);
        assert_eq!(entries[0].cache_read_tokens, 20);

        assert_eq!(entries[1].input_tokens, 160);
        assert_eq!(entries[1].output_tokens, 70);
        assert_eq!(entries[1].cache_read_tokens, 40);

        assert_eq!(entries[2].input_tokens, 160);
        assert_eq!(entries[2].output_tokens, 80);
        assert_eq!(entries[2].cache_read_tokens, 40);
    }

    #[test]
    fn test_zero_delta_skipped() {
        let parser = CodexParser::with_data_dir(PathBuf::from("tests/fixtures/codex"));
        let entries = parser
            .parse_file(&fixture_path("multi-turn-session.jsonl"))
            .unwrap();

        assert_eq!(entries.len(), 3);
    }

    #[test]
    fn test_byte_identical_reemissions_collapse() {
        // The same token_count event duplicated (identical total AND last):
        // one turn, counted once.
        let parser = CodexParser::with_data_dir(PathBuf::from("tests/fixtures/codex"));
        let entries = parser
            .parse_file(&fixture_path("reemitted-session.jsonl"))
            .unwrap();
        assert_eq!(entries.len(), 3);
        let reemitted_turn: Vec<_> = entries.iter().filter(|e| e.output_tokens == 70).collect();
        assert_eq!(
            reemitted_turn.len(),
            1,
            "re-emitted turn counted more than once"
        );
    }

    #[test]
    fn test_repeated_last_with_advancing_total_survives() {
        // `last` repeating while `total` advances is a distinct turn: the
        // guard must not collapse it (the undercount direction).
        let parser = CodexParser::with_data_dir(PathBuf::from("tests/fixtures/codex"));
        let entries = parser
            .parse_file(&fixture_path("repeated-last-advancing-total.jsonl"))
            .unwrap();
        // Prefer-last path: turn 2's last repeats turn 1's, so its delta is
        // (100 in, 50 out, 20 cached) again -> non-cached input 80, not 0 or
        // a cumulative 100; the point is both turns survive the guard.
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].input_tokens, 80);
        assert_eq!(entries[0].output_tokens, 50);
        assert_eq!(entries[1].input_tokens, 80);
        assert_eq!(entries[1].output_tokens, 50);
    }

    #[test]
    fn test_reemission_after_event_without_last_still_collapses() {
        // A token_count without `last_token_usage` between a turn and its
        // byte-identical re-emission must not clear the dedup memory,
        // otherwise the re-emission is counted as a second turn.
        let parser = CodexParser::with_data_dir(PathBuf::from("tests/fixtures/codex"));
        let entries = parser
            .parse_file(&fixture_path("reemission-around-missing-last.jsonl"))
            .unwrap();

        assert_eq!(entries.len(), 1, "re-emission counted as a second turn");
        assert_eq!(entries[0].input_tokens, 80);
        assert_eq!(entries[0].output_tokens, 50);
        assert_eq!(entries[0].cache_read_tokens, 20);
    }

    #[test]
    fn test_reemission_key_does_not_cross_session_meta() {
        // Dedup identity is scoped to one session: the first turn of a new
        // session_meta must count even if its numbers match the last turn
        // of the previous session.
        let parser = CodexParser::with_data_dir(PathBuf::from("tests/fixtures/codex"));
        let entries = parser
            .parse_file(&fixture_path("reemission-key-across-session-meta.jsonl"))
            .unwrap();

        assert_eq!(entries.len(), 2, "new session's first turn was collapsed");
        for (entry, session) in entries.iter().zip(["session-a", "session-b"]) {
            assert_eq!(entry.message_id, Some(session.to_string()));
            assert_eq!(entry.input_tokens, 80);
            assert_eq!(entry.output_tokens, 50);
            assert_eq!(entry.cache_read_tokens, 20);
        }
    }

    #[test]
    fn test_skip_invalid_lines() {
        let parser = CodexParser::with_data_dir(PathBuf::from("tests/fixtures/codex"));
        let entries = parser
            .parse_file(&fixture_path("sample-session.jsonl"))
            .unwrap();

        assert_eq!(entries.len(), 2);
    }

    #[test]
    fn test_session_id_and_source() {
        let parser = CodexParser::with_data_dir(PathBuf::from("tests/fixtures/codex"));
        let entries = parser
            .parse_file(&fixture_path("sample-session.jsonl"))
            .unwrap();

        for entry in &entries {
            assert_eq!(entry.source, Some("codex".into()));
            assert_eq!(entry.message_id, Some("session-001".to_string()));
        }
    }

    #[test]
    fn test_project_extracted_from_turn_context_cwd() {
        // Real Codex `turn_context.payload` carries `cwd` (and workspace_roots);
        // it must be attached to that session's usage as the project.
        let parser = CodexParser::with_data_dir(PathBuf::from("tests/fixtures/codex"));
        let entries = parser
            .parse_file(&fixture_path("cwd-session.jsonl"))
            .unwrap();

        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].project.as_deref(), Some("/work/myrepo"));
        assert_eq!(entries[0].model, Some("gpt-5.5".to_string()));
    }

    #[test]
    fn test_project_none_when_no_cwd() {
        let parser = CodexParser::with_data_dir(PathBuf::from("tests/fixtures/codex"));
        let entries = parser
            .parse_file(&fixture_path("sample-session.jsonl"))
            .unwrap();
        assert!(!entries.is_empty());
        for entry in &entries {
            assert_eq!(entry.project, None);
        }
    }

    #[test]
    fn test_archived_sessions_discovery_and_parsing() {
        let home = tempfile::tempdir().unwrap();
        let sessions = home.path().join("sessions");
        let archived = home.path().join("archived_sessions");
        std::fs::create_dir_all(sessions.join("2026/06/19")).unwrap();
        std::fs::create_dir_all(&archived).unwrap();
        let fixture = std::fs::read_to_string(fixture_path("cwd-session.jsonl")).unwrap();
        let active = sessions.join("2026/06/19/active.jsonl");
        std::fs::write(&active, &fixture).unwrap();
        // Archive moves flatten the date directories. Prefer the active copy.
        std::fs::write(archived.join("active.jsonl"), "invalid archive copy").unwrap();
        let archive_only = archived.join("archived.jsonl");
        std::fs::write(
            &archive_only,
            fixture.replace("sess-cwd-001", "archived-session"),
        )
        .unwrap();
        let parser = CodexParser::with_data_dir(sessions);
        let mut files = parser.collect_files();
        files.sort();
        let mut expected = vec![active, archive_only];
        expected.sort();
        assert_eq!(files, expected);
        assert_eq!(parser.parse_all().unwrap().len(), 2);
        assert_eq!(
            parser
                .parse_recent_files(std::time::UNIX_EPOCH)
                .unwrap()
                .len(),
            2
        );
    }

    #[test]
    fn test_archived_sessions_without_active_directory() {
        let home = tempfile::tempdir().unwrap();
        let archived = home.path().join("archived_sessions");
        std::fs::create_dir_all(&archived).unwrap();
        std::fs::copy(
            fixture_path("cwd-session.jsonl"),
            archived.join("only.jsonl"),
        )
        .unwrap();
        let parser = CodexParser::with_data_dir(home.path().join("sessions"));
        assert_eq!(parser.parse_all().unwrap().len(), 1);
    }

    #[test]
    fn test_archived_sessions_do_not_leak_into_custom_directory() {
        let home = tempfile::tempdir().unwrap();
        let custom = home.path().join("custom");
        let archived = home.path().join("archived_sessions");
        std::fs::create_dir_all(&custom).unwrap();
        std::fs::create_dir_all(&archived).unwrap();
        std::fs::copy(
            fixture_path("cwd-session.jsonl"),
            archived.join("only.jsonl"),
        )
        .unwrap();
        assert!(CodexParser::with_data_dir(custom)
            .collect_files()
            .is_empty());
    }

    #[test]
    fn test_parser_name() {
        let parser = CodexParser::new();
        assert_eq!(parser.name(), "codex");
    }

    #[test]
    fn test_parser_file_pattern() {
        let parser = CodexParser::new();
        assert_eq!(parser.file_pattern(), "**/*.jsonl");
    }

    #[test]
    fn test_codex_home_env_override() {
        let saved = std::env::var("CODEX_HOME").ok();
        std::env::set_var("CODEX_HOME", "/tmp/toktrack-codex-home");
        assert_eq!(
            CodexParser::new().data_dir(),
            Path::new("/tmp/toktrack-codex-home/sessions")
        );
        match saved {
            Some(v) => std::env::set_var("CODEX_HOME", v),
            None => std::env::remove_var("CODEX_HOME"),
        }
    }

    #[test]
    fn test_parse_nonexistent_file() {
        let parser = CodexParser::new();
        let result = parser.parse_file(Path::new("/nonexistent/file.jsonl"));
        assert!(result.is_err());
    }

    #[test]
    fn test_provider_extracted_from_session_meta() {
        // Real Codex CLI v0.116+ writes `session_meta.payload.model_provider`
        // as a lowercase string (e.g. "openai"); verified against live data.
        let parser = CodexParser::with_data_dir(PathBuf::from("tests/fixtures/codex"));
        let entries = parser
            .parse_file(&fixture_path("openai-session.jsonl"))
            .unwrap();

        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].provider, Some("openai".to_string()));
        assert_eq!(entries[0].model, Some("o4-mini".to_string()));
        assert_eq!(
            entries[0].message_id,
            Some("019d2e4c-e19a-7662-b2eb-0629d5ddd78b".to_string())
        );
    }

    #[test]
    fn test_provider_resets_when_later_session_meta_omits_it() {
        // A later session_meta without model_provider must clear the previous
        // value; sticking would silently misattribute billing.
        let parser = CodexParser::with_data_dir(PathBuf::from("tests/fixtures/codex"));
        let entries = parser
            .parse_file(&fixture_path("multi-session-meta.jsonl"))
            .unwrap();

        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].provider, Some("openai".to_string()));
        assert_eq!(
            entries[0].message_id,
            Some("session-with-provider".to_string())
        );
        assert_eq!(entries[1].provider, None);
        assert_eq!(
            entries[1].message_id,
            Some("session-without-provider".to_string())
        );
    }

    #[test]
    fn test_provider_none_when_session_meta_lacks_provider() {
        let parser = CodexParser::with_data_dir(PathBuf::from("tests/fixtures/codex"));
        let entries = parser
            .parse_file(&fixture_path("sample-session.jsonl"))
            .unwrap();

        assert!(!entries.is_empty());
        for entry in &entries {
            assert_eq!(entry.provider, None);
        }
    }

    /// Upstream schema canary. Codex has grown `reasoning_output_tokens` and
    /// `cache_write_input_tokens` inside `token_count` since this parser was
    /// written, and serde drops unknown fields silently. This mirror declares
    /// what we have seen and rejects the rest, over the fixture always and over
    /// the newest real session files when this machine has them.
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    #[allow(dead_code)] // Fields exist to declare the accepted schema, not to be read
    struct KnownInfo {
        total_token_usage: Option<KnownTokenUsage>,
        last_token_usage: Option<KnownTokenUsage>,
        model_context_window: Option<serde_json::Value>,
    }

    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    #[allow(dead_code)] // Fields exist to declare the accepted schema, not to be read
    struct KnownTokenUsage {
        input_tokens: Option<u64>,
        output_tokens: Option<u64>,
        cached_input_tokens: Option<u64>,
        cache_write_input_tokens: Option<u64>,
        reasoning_output_tokens: Option<u64>,
        total_tokens: Option<u64>,
    }

    /// Newest `count` rollout files on this machine, empty when Codex has never
    /// run here (CI).
    fn recent_local_sessions(count: usize) -> Vec<PathBuf> {
        let Some(dir) = directories::BaseDirs::new().map(|d| d.home_dir().join(".codex")) else {
            return Vec::new();
        };
        let pattern = dir.join("sessions").join("**").join("*.jsonl");
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

    /// Rollout files reach hundreds of MB, so stream and cap instead of reading
    /// the whole file: drift shows up in the first lines just as well.
    const CANARY_LINE_LIMIT: usize = 2_000;

    fn assert_token_count_shape_known(path: &Path) {
        let file = File::open(path).unwrap();
        for (i, line) in BufReader::new(file)
            .lines()
            .take(CANARY_LINE_LIMIT)
            .enumerate()
        {
            let Ok(line) = line else { continue };
            if !line.contains("token_count") {
                continue;
            }
            let Ok(value) = serde_json::from_str::<serde_json::Value>(&line) else {
                continue;
            };
            let Some(info) = value
                .get("payload")
                .filter(|p| p.get("type").and_then(|t| t.as_str()) == Some("token_count"))
                .and_then(|p| p.get("info"))
                .filter(|info| !info.is_null())
            else {
                continue;
            };
            if let Err(e) = serde_json::from_value::<KnownInfo>(info.clone()) {
                panic!(
                    "Codex token_count schema drifted at {}:{} — {}. Decide whether the new field \
                     affects cost, then add it here (and to CodexInfo/CodexTokenUsage if it does).",
                    path.display(),
                    i + 1,
                    e
                );
            }
        }
    }

    #[test]
    fn test_codex_token_usage_schema_has_no_unknown_fields() {
        assert_token_count_shape_known(&fixture_path("real-shape-token-count.jsonl"));
        for path in recent_local_sessions(5) {
            assert_token_count_shape_known(&path);
        }
    }

    /// Tiers we know how to price. A tier outside this set bills at Standard,
    /// so silent vocabulary growth upstream is an undercount, not an error.
    const KNOWN_SERVICE_TIERS: [&str; 3] = ["default", "priority", "fast"];

    /// Fast pricing rests on one field the parser does not own. This asserts
    /// where it lives and what it may say, so a rename, a move out of
    /// `thread_settings`, or a new tier id fails loudly instead of quietly
    /// pricing every Fast request at Standard.
    fn assert_service_tier_shape_known(path: &Path) {
        let file = File::open(path).unwrap();
        for (i, line) in BufReader::new(file)
            .lines()
            .take(CANARY_LINE_LIMIT)
            .enumerate()
        {
            let Ok(line) = line else { continue };
            if !line.contains("thread_settings_applied") {
                continue;
            }
            let Ok(value) = serde_json::from_str::<serde_json::Value>(&line) else {
                continue;
            };
            let Some(payload) = value.get("payload").filter(|p| {
                p.get("type").and_then(|t| t.as_str()) == Some("thread_settings_applied")
            }) else {
                continue;
            };
            let settings = &payload["thread_settings"];
            assert!(
                settings.is_object() || settings.is_null(),
                "Codex thread_settings is no longer an object at {}:{} — the service_tier lookup \
                 in parse_line reads it as one.",
                path.display(),
                i + 1
            );
            // Older rollouts omit the tier entirely; only a present one is checked.
            let Some(tier) = settings.get("service_tier").filter(|t| !t.is_null()) else {
                continue;
            };
            let known = tier
                .as_str()
                .is_some_and(|t| KNOWN_SERVICE_TIERS.contains(&t));
            assert!(
                known,
                "Codex service_tier drifted to {tier} at {}:{} — decide whether it bills at a Fast \
                 rate, then add it to KNOWN_SERVICE_TIERS (and to parse_line if it does).",
                path.display(),
                i + 1
            );
        }
    }

    #[test]
    fn test_codex_service_tier_schema_is_known() {
        assert_service_tier_shape_known(&fixture_path("real-shape-thread-settings.jsonl"));
        for path in recent_local_sessions(5) {
            assert_service_tier_shape_known(&path);
        }
    }
}
