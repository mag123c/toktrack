//! Gemini CLI JSON / JSONL parser
//!
//! Supports two formats:
//! - Legacy single-object `.json` (gemini-cli ≤ 2026-04-08)
//! - Streaming `.jsonl` (gemini-cli ≥ 2026-04-09, PR #23749)

use crate::types::{Result, ToktrackError, UsageEntry};
use chrono::{DateTime, Utc};
use serde::Deserialize;
use std::collections::HashSet;
use std::fs::{self, File};
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

use super::CLIParser;

/// Legacy Gemini session JSON structure (pre-2026-04-09)
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct GeminiSession {
    session_id: String,
    model: Option<String>,
    messages: Vec<GeminiMessage>,
}

#[derive(Deserialize)]
struct GeminiMessage {
    id: String,
    #[serde(rename = "type")]
    msg_type: String,
    timestamp: String,
    #[serde(default)]
    tokens: Option<GeminiTokens>,
    model: Option<String>,
}

#[derive(Deserialize)]
struct GeminiTokens {
    input: u64,
    output: u64,
    #[serde(default)]
    cached: u64,
    #[serde(default)]
    thoughts: u64,
}

/// JSONL line — combines metadata, message, and dispatch markers.
/// Field presence determines line type at runtime.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct GeminiJsonlLine<'a> {
    // Metadata line carries sessionId
    #[serde(default, borrow)]
    session_id: Option<&'a str>,

    // Message fields
    #[serde(default, borrow)]
    id: Option<&'a str>,
    #[serde(default, rename = "type", borrow)]
    msg_type: Option<&'a str>,
    #[serde(default, borrow)]
    timestamp: Option<&'a str>,
    #[serde(default, borrow)]
    model: Option<&'a str>,
    #[serde(default)]
    tokens: Option<GeminiTokensJsonl>,
}

#[derive(Deserialize)]
struct GeminiTokensJsonl {
    #[serde(default)]
    input: u64,
    #[serde(default)]
    output: u64,
    #[serde(default)]
    cached: Option<u64>,
    #[serde(default)]
    thoughts: Option<u64>,
    // `tool` (toolUsePromptTokenCount, added 2026-04-09) is merged into input_tokens.
    #[serde(default)]
    tool: Option<u64>,
}

/// Parser for Gemini CLI usage data
pub struct GeminiParser {
    data_dir: PathBuf,
}

impl GeminiParser {
    /// Create a new parser with default data directory.
    ///
    /// Honors `GEMINI_CLI_HOME` (matches gemini-cli's `paths.ts` behavior),
    /// falling back to the user's home directory.
    pub fn new() -> Self {
        let home = std::env::var("GEMINI_CLI_HOME")
            .ok()
            .filter(|s| !s.is_empty())
            .map(PathBuf::from)
            .or_else(|| directories::BaseDirs::new().map(|d| d.home_dir().to_path_buf()))
            .unwrap_or_else(|| {
                eprintln!("[toktrack] Warning: Could not determine home directory");
                PathBuf::from(".")
            });
        Self {
            data_dir: home.join(".gemini").join("tmp"),
        }
    }

    /// Create a parser with a custom data directory (for testing)
    #[allow(dead_code)]
    pub fn with_data_dir(data_dir: PathBuf) -> Self {
        Self { data_dir }
    }

    /// Parse a legacy `.json` (single-object) session file.
    fn parse_json_legacy(&self, path: &Path) -> Result<Vec<UsageEntry>> {
        let mut content = fs::read_to_string(path).map_err(ToktrackError::Io)?;
        // SAFETY: `content` is exclusively owned and not aliased; safe for simd_json in-place mutation
        let session: GeminiSession = unsafe {
            simd_json::from_str(&mut content).map_err(|e| ToktrackError::Parse(e.to_string()))?
        };

        let mut entries = Vec::new();

        for msg in session.messages {
            if msg.msg_type != "gemini" {
                continue;
            }

            let tokens = match msg.tokens {
                Some(t) => t,
                None => continue,
            };

            let timestamp = match DateTime::parse_from_rfc3339(&msg.timestamp) {
                Ok(dt) => dt.with_timezone(&Utc),
                Err(_) => {
                    eprintln!(
                        "[toktrack] Warning: Invalid timestamp '{}', skipping entry",
                        msg.timestamp
                    );
                    continue;
                }
            };

            entries.push(UsageEntry {
                timestamp,
                model: msg.model.clone().or_else(|| session.model.clone()),
                input_tokens: tokens.input,
                output_tokens: tokens.output,
                cache_read_tokens: tokens.cached,
                cache_creation_tokens: 0,
                thinking_tokens: tokens.thoughts,
                cache_creation_5m_tokens: 0,
                cache_creation_1h_tokens: 0,
                web_search_requests: 0,
                cost_usd: None,
                message_id: Some(msg.id),
                request_id: Some(session.session_id.clone()),
                source: Some("gemini".into()),
                provider: None,
            });
        }

        Ok(entries)
    }

    /// Parse a `.jsonl` (streaming) session file (gemini-cli ≥ 2026-04-09).
    fn parse_jsonl(&self, path: &Path) -> Result<Vec<UsageEntry>> {
        let file = File::open(path).map_err(ToktrackError::Io)?;
        let reader = BufReader::new(file);

        let fallback_session_id: String = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_string();
        let mut session_id: Option<String> = None;
        let mut entries = Vec::new();

        for line_result in reader.lines() {
            let line = match line_result {
                Ok(l) => l,
                Err(_) => continue,
            };
            if line.trim().is_empty() {
                continue;
            }

            let mut bytes = line.into_bytes();
            let parsed: GeminiJsonlLine = match simd_json::from_slice(&mut bytes) {
                Ok(v) => v,
                Err(_) => continue, // malformed line: skip
            };

            // Metadata line — capture sessionId for subsequent message records.
            if let Some(sid) = parsed.session_id {
                session_id = Some(sid.to_string());
                continue;
            }

            // Only `type: "gemini"` carries usage. Other types (user/info/error/warning)
            // and dispatch markers (`$set`, `$rewindTo`) fall through here.
            if parsed.msg_type != Some("gemini") {
                continue;
            }

            let tokens = match parsed.tokens {
                Some(t) => t,
                None => continue,
            };

            let ts_str = match parsed.timestamp {
                Some(s) => s,
                None => continue,
            };
            let timestamp = match DateTime::parse_from_rfc3339(ts_str) {
                Ok(dt) => dt.with_timezone(&Utc),
                Err(_) => {
                    eprintln!(
                        "[toktrack] Warning: Invalid timestamp '{}', skipping entry",
                        ts_str
                    );
                    continue;
                }
            };

            let request_id = session_id
                .clone()
                .unwrap_or_else(|| fallback_session_id.clone());

            entries.push(UsageEntry {
                timestamp,
                model: parsed.model.map(String::from),
                // `tokens.tool` is gemini-cli's toolUsePromptTokenCount — folded into input
                // so user-visible totals match gemini-cli's own `total` (which sums it in).
                input_tokens: tokens.input + tokens.tool.unwrap_or(0),
                output_tokens: tokens.output,
                cache_read_tokens: tokens.cached.unwrap_or(0),
                cache_creation_tokens: 0,
                thinking_tokens: tokens.thoughts.unwrap_or(0),
                cache_creation_5m_tokens: 0,
                cache_creation_1h_tokens: 0,
                web_search_requests: 0,
                cost_usd: None,
                message_id: parsed.id.map(String::from),
                request_id: Some(request_id),
                source: Some("gemini".into()),
                provider: None,
            });
        }

        Ok(entries)
    }
}

impl Default for GeminiParser {
    fn default() -> Self {
        Self::new()
    }
}

impl CLIParser for GeminiParser {
    fn name(&self) -> &str {
        "gemini"
    }

    fn data_dir(&self) -> &Path {
        &self.data_dir
    }

    /// Representative pattern. Actual collection uses three globs via `collect_files`.
    fn file_pattern(&self) -> &str {
        "*/chats/session-*.jsonl"
    }

    fn collect_files(&self) -> Vec<PathBuf> {
        // 1. main session (new) — session-{ts}-{8-char}.jsonl
        // 2. main session (legacy) — session-*.json
        // 3. subagent — {chatsDir}/{parentSessionId}/{id}.jsonl (no `session-` prefix)
        let patterns = [
            "*/chats/session-*.jsonl",
            "*/chats/session-*.json",
            "*/chats/*/*.jsonl",
        ];
        let mut seen: HashSet<PathBuf> = HashSet::new();
        let mut out = Vec::new();
        for pat in patterns {
            let full = self.data_dir.join(pat);
            if let Ok(paths) = glob::glob(&full.to_string_lossy()) {
                for p in paths.filter_map(std::result::Result::ok) {
                    if seen.insert(p.clone()) {
                        out.push(p);
                    }
                }
            }
        }
        out
    }

    fn parse_file(&self, path: &Path) -> Result<Vec<UsageEntry>> {
        match path.extension().and_then(|s| s.to_str()) {
            Some("jsonl") => self.parse_jsonl(path),
            Some("json") => self.parse_json_legacy(path),
            _ => Ok(Vec::new()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn fixture_path() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("fixtures")
            .join("gemini")
            .join("tmp123")
            .join("chats")
            .join("session-abc123.json")
    }

    fn jsonl_fixture_path() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("fixtures")
            .join("gemini")
            .join("tmp_jsonl")
            .join("chats")
            .join("session-2026-04-20T10-00-00-abc12345.jsonl")
    }

    fn jsonl_subagent_path() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("fixtures")
            .join("gemini")
            .join("tmp_jsonl")
            .join("chats")
            .join("parent-session-xyz")
            .join("sub-abc.jsonl")
    }

    fn jsonl_malformed_path() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("fixtures")
            .join("gemini")
            .join("tmp_jsonl_malformed")
            .join("chats")
            .join("session-bad.jsonl")
    }

    fn jsonl_no_meta_path() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("fixtures")
            .join("gemini")
            .join("tmp_jsonl_no_meta")
            .join("chats")
            .join("session-2026-04-22T11-00-00-no00meta.jsonl")
    }

    // ===== Legacy .json regression tests (preserved from prior version) =====

    #[test]
    fn test_parse_gemini_json() {
        let parser = GeminiParser::with_data_dir(PathBuf::from("tests/fixtures/gemini"));
        let entries = parser.parse_file(&fixture_path()).unwrap();
        assert_eq!(entries.len(), 2);
    }

    #[test]
    fn test_parse_first_entry() {
        let parser = GeminiParser::with_data_dir(PathBuf::from("tests/fixtures/gemini"));
        let entries = parser.parse_file(&fixture_path()).unwrap();

        let first = &entries[0];
        assert_eq!(first.model, Some("gemini-2.5-pro".to_string()));
        assert_eq!(first.input_tokens, 100);
        assert_eq!(first.output_tokens, 50);
        assert_eq!(first.cache_read_tokens, 20);
        assert_eq!(first.cache_creation_tokens, 0);
        assert_eq!(first.thinking_tokens, 30);
        assert_eq!(first.source, Some("gemini".into()));
        assert_eq!(first.message_id, Some("msg-002".to_string()));
        assert_eq!(first.request_id, Some("abc123".to_string()));
    }

    #[test]
    fn test_parse_second_entry_with_thinking() {
        let parser = GeminiParser::with_data_dir(PathBuf::from("tests/fixtures/gemini"));
        let entries = parser.parse_file(&fixture_path()).unwrap();

        let second = &entries[1];
        assert_eq!(second.input_tokens, 250);
        assert_eq!(second.output_tokens, 150);
        assert_eq!(second.cache_read_tokens, 50);
        assert_eq!(second.thinking_tokens, 100);
        assert_eq!(second.message_id, Some("msg-004".to_string()));
    }

    #[test]
    fn test_skip_non_gemini_messages() {
        let parser = GeminiParser::with_data_dir(PathBuf::from("tests/fixtures/gemini"));
        let entries = parser.parse_file(&fixture_path()).unwrap();
        assert!(entries.iter().all(|e| e.source == Some("gemini".into())));
    }

    #[test]
    fn test_parser_name() {
        let parser = GeminiParser::new();
        assert_eq!(parser.name(), "gemini");
    }

    #[test]
    fn test_parser_file_pattern() {
        let parser = GeminiParser::new();
        // Representative — see `collect_files` for the full glob set.
        assert_eq!(parser.file_pattern(), "*/chats/session-*.jsonl");
    }

    #[test]
    fn test_parse_nonexistent_file() {
        let parser = GeminiParser::new();
        let result = parser.parse_file(Path::new("/nonexistent/file.json"));
        assert!(result.is_err());
    }

    #[test]
    fn test_total_tokens_includes_thinking() {
        let parser = GeminiParser::with_data_dir(PathBuf::from("tests/fixtures/gemini"));
        let entries = parser.parse_file(&fixture_path()).unwrap();
        assert_eq!(entries[0].total_tokens(), 200);
        assert_eq!(entries[1].total_tokens(), 550);
    }

    fn fixture_no_session_model_path() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("fixtures")
            .join("gemini")
            .join("tmp456")
            .join("chats")
            .join("session-no-session-model.json")
    }

    #[test]
    fn test_parse_msg_model_fallback_when_session_model_missing() {
        let parser = GeminiParser::with_data_dir(PathBuf::from("tests/fixtures/gemini"));
        let entries = parser.parse_file(&fixture_no_session_model_path()).unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].model, Some("gemini-2.5-pro".to_string()));
        assert_eq!(entries[1].model, Some("gemini-2.5-flash".to_string()));
        assert!(entries
            .iter()
            .all(|e| e.model.is_some() && e.model.as_deref() != Some("unknown")));
    }

    // ===== JSONL tests =====

    #[test]
    fn test_parse_jsonl_main_session() {
        let parser = GeminiParser::with_data_dir(PathBuf::from("tests/fixtures/gemini"));
        let entries = parser.parse_file(&jsonl_fixture_path()).unwrap();

        // Only `type: "gemini"` lines (m2, m4) count; user/error/$set/$rewindTo are skipped.
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].message_id, Some("m2".to_string()));
        assert_eq!(entries[0].model, Some("gemini-2.5-pro".to_string()));
        assert_eq!(entries[1].message_id, Some("m4".to_string()));
        assert_eq!(entries[1].model, Some("gemini-2.5-flash".to_string()));
    }

    #[test]
    fn test_parse_jsonl_tool_tokens_added_to_input() {
        let parser = GeminiParser::with_data_dir(PathBuf::from("tests/fixtures/gemini"));
        let entries = parser.parse_file(&jsonl_fixture_path()).unwrap();

        // m2: tokens.input=100, tokens.tool=10 → input_tokens=110
        assert_eq!(entries[0].input_tokens, 110);
        // m4: tokens.input=250, no `tool` → input_tokens=250
        assert_eq!(entries[1].input_tokens, 250);
    }

    #[test]
    fn test_parse_jsonl_request_id_from_metadata() {
        let parser = GeminiParser::with_data_dir(PathBuf::from("tests/fixtures/gemini"));
        let entries = parser.parse_file(&jsonl_fixture_path()).unwrap();

        let expected = Some("abc12345-9999-4d2e-8a1f-000000000001".to_string());
        assert_eq!(entries[0].request_id, expected);
        assert_eq!(entries[1].request_id, expected);
    }

    #[test]
    fn test_parse_jsonl_skips_set_and_rewind_and_non_gemini() {
        // Implicitly covered by main_session test (len == 2), but make it explicit:
        // m1 (user), $set, m3 (error), $rewindTo are all skipped.
        let parser = GeminiParser::with_data_dir(PathBuf::from("tests/fixtures/gemini"));
        let entries = parser.parse_file(&jsonl_fixture_path()).unwrap();
        let ids: Vec<_> = entries
            .iter()
            .filter_map(|e| e.message_id.clone())
            .collect();
        assert_eq!(ids, vec!["m2".to_string(), "m4".to_string()]);
    }

    #[test]
    fn test_parse_jsonl_subagent() {
        let parser = GeminiParser::with_data_dir(PathBuf::from("tests/fixtures/gemini"));
        let entries = parser.parse_file(&jsonl_subagent_path()).unwrap();

        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].message_id, Some("sm2".to_string()));
        assert_eq!(entries[0].input_tokens, 40);
        assert_eq!(
            entries[0].request_id,
            Some("sub-abc-0000-4d2e-8a1f-000000000002".to_string())
        );
    }

    #[test]
    fn test_parse_jsonl_malformed_lines_skipped() {
        let parser = GeminiParser::with_data_dir(PathBuf::from("tests/fixtures/gemini"));
        let entries = parser.parse_file(&jsonl_malformed_path()).unwrap();

        // Blank line, `{not_json`, and gemini-without-timestamp should all skip.
        // Only `bm1` (well-formed gemini) survives.
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].message_id, Some("bm1".to_string()));
        assert_eq!(entries[0].input_tokens, 7);
    }

    #[test]
    fn test_parse_jsonl_no_metadata_fallback_to_file_stem() {
        let parser = GeminiParser::with_data_dir(PathBuf::from("tests/fixtures/gemini"));
        let entries = parser.parse_file(&jsonl_no_meta_path()).unwrap();

        assert_eq!(entries.len(), 1);
        // No metadata line → request_id falls back to the file stem.
        assert_eq!(
            entries[0].request_id,
            Some("session-2026-04-22T11-00-00-no00meta".to_string())
        );
    }

    fn fixture_root() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("fixtures")
            .join("gemini")
    }

    #[test]
    fn test_collect_files_finds_jsonl_and_json_and_subagent() {
        let parser = GeminiParser::with_data_dir(fixture_root());
        let files = parser.collect_files();

        // Expected matches:
        // - tmp123/chats/session-abc123.json                          (legacy main)
        // - tmp456/chats/session-no-session-model.json                (legacy main)
        // - tmp_jsonl/chats/session-2026-04-20T10-00-00-abc12345.jsonl (main new)
        // - tmp_jsonl/chats/parent-session-xyz/sub-abc.jsonl           (subagent)
        // - tmp_jsonl_malformed/chats/session-bad.jsonl                (main new, bad lines)
        // - tmp_jsonl_no_meta/chats/session-2026-04-22T11-00-00-no00meta.jsonl (main new)
        assert_eq!(files.len(), 6, "got files: {:?}", files);

        // Spot-check both extensions present
        assert!(files
            .iter()
            .any(|f| f.extension().and_then(|s| s.to_str()) == Some("json")));
        assert!(files
            .iter()
            .any(|f| f.extension().and_then(|s| s.to_str()) == Some("jsonl")));
        // Spot-check subagent path nested under a parent dir under chats/
        assert!(files
            .iter()
            .any(|f| f.to_string_lossy().contains("parent-session-xyz")));
    }

    #[test]
    fn test_parse_all_aggregates_both_formats() {
        let parser = GeminiParser::with_data_dir(fixture_root());
        let entries = parser.parse_all().unwrap();

        // Expected entries per file:
        // - tmp123/...session-abc123.json: 2
        // - tmp456/...session-no-session-model.json: 2
        // - tmp_jsonl/...session-2026-04-20....jsonl: 2
        // - tmp_jsonl/...parent-session-xyz/sub-abc.jsonl: 1
        // - tmp_jsonl_malformed/...session-bad.jsonl: 1
        // - tmp_jsonl_no_meta/...session-2026-04-22....jsonl: 1
        // = 9 total, all unique message_id × request_id combos.
        assert_eq!(entries.len(), 9);
    }

    #[test]
    fn test_gemini_cli_home_env_var_overrides_home() {
        // Other tests use `with_data_dir`; only `new()` reads this env var,
        // so concurrent reads don't collide on data_dir state.
        let saved = std::env::var("GEMINI_CLI_HOME").ok();
        std::env::set_var("GEMINI_CLI_HOME", "/tmp/toktrack-gemini-env-test");

        let parser = GeminiParser::new();
        assert_eq!(
            parser.data_dir(),
            Path::new("/tmp/toktrack-gemini-env-test/.gemini/tmp")
        );

        // Restore prior env state.
        match saved {
            Some(v) => std::env::set_var("GEMINI_CLI_HOME", v),
            None => std::env::remove_var("GEMINI_CLI_HOME"),
        }
    }

    #[test]
    fn test_gemini_cli_home_empty_string_falls_through() {
        let saved = std::env::var("GEMINI_CLI_HOME").ok();
        std::env::set_var("GEMINI_CLI_HOME", "");

        let parser = GeminiParser::new();
        // Empty value should be ignored (matches gemini-cli JS falsiness).
        // Resulting data_dir should NOT be just "/.gemini/tmp".
        assert_ne!(parser.data_dir(), Path::new("/.gemini/tmp"));

        match saved {
            Some(v) => std::env::set_var("GEMINI_CLI_HOME", v),
            None => std::env::remove_var("GEMINI_CLI_HOME"),
        }
    }
}
