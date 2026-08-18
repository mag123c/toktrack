//! Claude Code JSONL parser

use crate::types::{Result, ToktrackError, UsageEntry};
use chrono::{DateTime, Utc};
use serde::Deserialize;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

use super::CLIParser;

/// Claude Code JSONL line structure (assistant messages with usage)
#[derive(Deserialize)]
struct ClaudeJsonLine<'a> {
    timestamp: &'a str,
    #[serde(rename = "requestId")]
    request_id: Option<&'a str>,
    message: Option<ClaudeMessage<'a>>,
    #[serde(rename = "costUSD")]
    cost_usd: Option<f64>,
    /// Working directory of the session — used as the project identifier.
    cwd: Option<&'a str>,
}

#[derive(Deserialize)]
struct ClaudeMessage<'a> {
    model: Option<&'a str>,
    id: Option<&'a str>,
    usage: Option<ClaudeUsage<'a>>,
}

#[derive(Deserialize, Default)]
struct CacheCreationDetail {
    ephemeral_5m_input_tokens: Option<u64>,
    ephemeral_1h_input_tokens: Option<u64>,
}

#[derive(Deserialize, Default)]
struct ServerToolUse {
    web_search_requests: Option<u64>,
    web_fetch_requests: Option<u64>,
}

#[derive(Deserialize)]
struct ClaudeUsage<'a> {
    input_tokens: u64,
    output_tokens: u64,
    cache_creation_input_tokens: Option<u64>,
    cache_read_input_tokens: Option<u64>,
    cache_creation: Option<CacheCreationDetail>,
    server_tool_use: Option<ServerToolUse>,
    /// "standard" or "fast" — fast mode bills at the provider's fast multiplier.
    #[serde(borrow)]
    speed: Option<&'a str>,
}

/// Parser for Claude Code usage data
pub struct ClaudeCodeParser {
    data_dir: PathBuf,
}

impl ClaudeCodeParser {
    /// Create a new parser with default data directory.
    ///
    /// Honors `CLAUDE_CONFIG_DIR` (root; replaces `~/.claude`); projects live
    /// under `<root>/projects`.
    pub fn new() -> Self {
        let root = super::discovery::first_env_dir(&["CLAUDE_CONFIG_DIR"]).unwrap_or_else(|| {
            directories::BaseDirs::new()
                .map(|d| d.home_dir().join(".claude"))
                .unwrap_or_else(|| {
                    eprintln!("[toktrack] Warning: Could not determine home directory");
                    PathBuf::from(".")
                })
        });
        Self {
            data_dir: root.join("projects"),
        }
    }

    /// Create a parser with a custom data directory (for testing)
    #[allow(dead_code)] // Used in tests
    pub fn with_data_dir(data_dir: PathBuf) -> Self {
        Self { data_dir }
    }

    /// Parse a single JSONL line (zero-copy with borrowed strings)
    fn parse_line(&self, line: &mut [u8]) -> Option<UsageEntry> {
        if line.is_empty() {
            return None;
        }

        let data: ClaudeJsonLine = simd_json::from_slice(line).ok()?;

        let message = data.message.as_ref()?;
        let usage = message.usage.as_ref()?;

        if message.model == Some("<synthetic>") {
            return None;
        }

        let timestamp = match DateTime::parse_from_rfc3339(data.timestamp) {
            Ok(dt) => dt.with_timezone(&Utc),
            Err(_) => {
                eprintln!(
                    "[toktrack] Warning: Invalid timestamp '{}', skipping entry",
                    data.timestamp
                );
                return None;
            }
        };

        let cache_detail = usage.cache_creation.as_ref();
        let tool_use = usage.server_tool_use.as_ref();

        Some(UsageEntry {
            fast_speed: usage.speed == Some("fast"),
            timestamp,
            model: message.model.map(String::from),
            input_tokens: usage.input_tokens,
            output_tokens: usage.output_tokens,
            cache_read_tokens: usage.cache_read_input_tokens.unwrap_or(0),
            cache_creation_tokens: usage.cache_creation_input_tokens.unwrap_or(0),
            reasoning_tokens: 0,
            cache_creation_5m_tokens: cache_detail
                .and_then(|d| d.ephemeral_5m_input_tokens)
                .unwrap_or(0),
            cache_creation_1h_tokens: cache_detail
                .and_then(|d| d.ephemeral_1h_input_tokens)
                .unwrap_or(0),
            web_search_requests: tool_use.and_then(|t| t.web_search_requests).unwrap_or(0),
            web_fetch_requests: tool_use.and_then(|t| t.web_fetch_requests).unwrap_or(0),
            reported_total_tokens: None,
            cost_usd: data.cost_usd,
            message_id: message.id.map(String::from),
            request_id: data.request_id.map(String::from),
            source: Some("claude".into()),
            provider: None,
            project: data.cwd.map(String::from),
        })
    }
}

impl Default for ClaudeCodeParser {
    fn default() -> Self {
        Self::new()
    }
}

impl CLIParser for ClaudeCodeParser {
    fn name(&self) -> &str {
        "claude-code"
    }

    fn data_dir(&self) -> &Path {
        &self.data_dir
    }

    fn file_pattern(&self) -> &str {
        "**/*.jsonl"
    }

    fn parse_file(&self, path: &Path) -> Result<Vec<UsageEntry>> {
        let file = File::open(path).map_err(ToktrackError::Io)?;
        let reader = BufReader::new(file);
        let mut entries = Vec::new();

        for line_result in reader.lines() {
            let line = match line_result {
                Ok(l) => l,
                Err(_) => continue, // Skip lines with read errors
            };

            if line.is_empty() {
                continue;
            }

            let mut line_bytes = line.into_bytes();
            if let Some(entry) = self.parse_line(&mut line_bytes) {
                entries.push(entry);
            }
        }

        Ok(entries)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn fixture_path(name: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("fixtures")
            .join(name)
    }

    /// Token-semantics contract test pinned to the REAL Claude Code log shape
    /// (sanitized from `~/.claude/projects`). Proves toktrack (a) tolerates fields
    /// it does not read (`web_fetch_requests`, `service_tier`, `iterations`, `speed`)
    /// and (b) extracts each token field per the v2 contract. Modern Claude logs
    /// omit top-level `costUSD`, so cost is LiteLLM-calculated → `cost_usd` is None.
    #[test]
    fn test_real_shape_claude_contract() {
        let parser = ClaudeCodeParser::with_data_dir(PathBuf::from("tests/fixtures"));
        let entries = parser
            .parse_file(&fixture_path("claude/real-shape-session.jsonl"))
            .unwrap();

        assert_eq!(entries.len(), 1, "only the assistant line carries usage");
        let e = &entries[0];
        assert_eq!(e.model.as_deref(), Some("claude-opus-4-1-20250805"));
        assert_eq!(e.input_tokens, 6); // non-cached (Anthropic input excludes cache)
        assert_eq!(e.cache_creation_tokens, 19693);
        assert_eq!(e.cache_read_tokens, 17079);
        assert_eq!(e.output_tokens, 1075);
        assert_eq!(e.cache_creation_5m_tokens, 0);
        assert_eq!(e.cache_creation_1h_tokens, 19693);
        assert_eq!(e.web_search_requests, 2);
        assert_eq!(e.web_fetch_requests, 1); // captured though not auto-priced
        assert_eq!(e.reasoning_tokens, 0); // folded into output by Anthropic
        assert_eq!(e.reported_total_tokens, None);
        assert_eq!(e.cost_usd, None); // modern logs omit costUSD → LiteLLM-calculated
    }

    #[test]
    fn test_parse_claude_jsonl() {
        let parser = ClaudeCodeParser::with_data_dir(PathBuf::from("tests/fixtures"));
        let entries = parser
            .parse_file(&fixture_path("claude-sample.jsonl"))
            .unwrap();

        assert_eq!(entries.len(), 4);
    }

    #[test]
    fn test_parse_first_entry() {
        let parser = ClaudeCodeParser::with_data_dir(PathBuf::from("tests/fixtures"));
        let entries = parser
            .parse_file(&fixture_path("claude-sample.jsonl"))
            .unwrap();

        let first = &entries[0];
        assert_eq!(first.model, Some("claude-sonnet-4-20250514".to_string()));
        assert_eq!(first.input_tokens, 100);
        assert_eq!(first.output_tokens, 50);
        assert_eq!(first.cache_creation_tokens, 10);
        assert_eq!(first.cache_read_tokens, 20);
        assert_eq!(first.message_id, Some("msg-001".to_string()));
        assert_eq!(first.request_id, Some("req-001".to_string()));
    }

    #[test]
    fn test_parse_entry_with_cost() {
        let parser = ClaudeCodeParser::with_data_dir(PathBuf::from("tests/fixtures"));
        let entries = parser
            .parse_file(&fixture_path("claude-sample.jsonl"))
            .unwrap();

        let second = &entries[1];
        assert_eq!(second.model, Some("claude-opus-4-20250514".to_string()));
        assert_eq!(second.cost_usd, Some(0.025));
    }

    #[test]
    fn test_parse_entry_without_optional_fields() {
        let parser = ClaudeCodeParser::with_data_dir(PathBuf::from("tests/fixtures"));
        let entries = parser
            .parse_file(&fixture_path("claude-sample.jsonl"))
            .unwrap();

        let third = &entries[2];
        assert_eq!(third.cache_creation_tokens, 0);
        assert_eq!(third.cache_read_tokens, 0);
        assert_eq!(third.message_id, None);
        assert_eq!(third.request_id, None);
    }

    #[test]
    fn test_skip_invalid_lines() {
        let parser = ClaudeCodeParser::with_data_dir(PathBuf::from("tests/fixtures"));
        let entries = parser
            .parse_file(&fixture_path("claude-sample.jsonl"))
            .unwrap();

        assert_eq!(entries.len(), 4);
    }

    #[test]
    fn test_skip_user_messages() {
        let parser = ClaudeCodeParser::with_data_dir(PathBuf::from("tests/fixtures"));
        let entries = parser
            .parse_file(&fixture_path("claude-sample.jsonl"))
            .unwrap();

        assert!(entries.iter().all(|e| e.input_tokens > 0));
    }

    #[test]
    fn test_dedup_hash() {
        let parser = ClaudeCodeParser::with_data_dir(PathBuf::from("tests/fixtures"));
        let entries = parser
            .parse_file(&fixture_path("claude-sample.jsonl"))
            .unwrap();

        assert_eq!(entries[0].dedup_hash(), Some("msg-001:req-001".to_string()));

        assert_eq!(entries[2].dedup_hash(), None);
    }

    #[test]
    fn test_parser_name() {
        let parser = ClaudeCodeParser::new();
        assert_eq!(parser.name(), "claude-code");
    }

    #[test]
    fn test_parser_file_pattern() {
        let parser = ClaudeCodeParser::new();
        assert_eq!(parser.file_pattern(), "**/*.jsonl");
    }

    #[test]
    fn test_claude_config_dir_env_override() {
        let saved = std::env::var("CLAUDE_CONFIG_DIR").ok();
        std::env::set_var("CLAUDE_CONFIG_DIR", "/tmp/toktrack-claude-cfg");
        assert_eq!(
            ClaudeCodeParser::new().data_dir(),
            Path::new("/tmp/toktrack-claude-cfg/projects")
        );
        match saved {
            Some(v) => std::env::set_var("CLAUDE_CONFIG_DIR", v),
            None => std::env::remove_var("CLAUDE_CONFIG_DIR"),
        }
    }

    #[test]
    fn test_parse_nonexistent_file() {
        let parser = ClaudeCodeParser::new();
        let result = parser.parse_file(Path::new("/nonexistent/file.jsonl"));
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_empty_file() {
        let parser = ClaudeCodeParser::with_data_dir(PathBuf::from("tests/fixtures"));
        let entries = parser.parse_file(&fixture_path("empty.jsonl")).unwrap();
        assert!(entries.is_empty());
    }

    #[test]
    fn test_skip_synthetic_model() {
        let parser = ClaudeCodeParser::with_data_dir(PathBuf::from("tests/fixtures"));
        let entries = parser
            .parse_file(&fixture_path("claude-sample.jsonl"))
            .unwrap();

        assert!(
            entries
                .iter()
                .all(|e| e.model != Some("<synthetic>".to_string())),
            "Synthetic model entries should be filtered out"
        );
    }

    #[test]
    fn test_parse_cache_ttl_tiers() {
        let parser = ClaudeCodeParser::with_data_dir(PathBuf::from("tests/fixtures"));
        let entries = parser
            .parse_file(&fixture_path("claude-sample.jsonl"))
            .unwrap();

        let fourth = &entries[3];
        assert_eq!(fourth.cache_creation_tokens, 5000);
        assert_eq!(fourth.cache_creation_5m_tokens, 3000);
        assert_eq!(fourth.cache_creation_1h_tokens, 2000);
    }

    #[test]
    fn test_parse_web_search_requests() {
        let parser = ClaudeCodeParser::with_data_dir(PathBuf::from("tests/fixtures"));
        let entries = parser
            .parse_file(&fixture_path("claude-sample.jsonl"))
            .unwrap();

        let fourth = &entries[3];
        assert_eq!(fourth.web_search_requests, 3);
    }

    #[test]
    fn test_no_ttl_tiers_defaults_to_zero() {
        let parser = ClaudeCodeParser::with_data_dir(PathBuf::from("tests/fixtures"));
        let entries = parser
            .parse_file(&fixture_path("claude-sample.jsonl"))
            .unwrap();

        let first = &entries[0];
        assert_eq!(first.cache_creation_5m_tokens, 0);
        assert_eq!(first.cache_creation_1h_tokens, 0);
        assert_eq!(first.web_search_requests, 0);
    }

    #[test]
    fn test_parse_line_extracts_cwd_as_project() {
        let parser = ClaudeCodeParser::with_data_dir(PathBuf::from("tests/fixtures"));
        let line = r#"{"type":"assistant","timestamp":"2026-01-15T10:00:01.500Z","cwd":"/work/demo","message":{"model":"claude-sonnet-4","id":"msg-x","usage":{"input_tokens":100,"output_tokens":50}}}"#;
        let mut bytes = line.as_bytes().to_vec();
        let entry = parser.parse_line(&mut bytes).expect("entry parsed");
        assert_eq!(entry.project.as_deref(), Some("/work/demo"));
    }

    #[test]
    fn test_parse_line_without_cwd_has_no_project() {
        let parser = ClaudeCodeParser::with_data_dir(PathBuf::from("tests/fixtures"));
        let line = r#"{"type":"assistant","timestamp":"2026-01-15T10:00:01.500Z","message":{"model":"claude-sonnet-4","id":"msg-y","usage":{"input_tokens":100,"output_tokens":50}}}"#;
        let mut bytes = line.as_bytes().to_vec();
        let entry = parser.parse_line(&mut bytes).expect("entry parsed");
        assert_eq!(entry.project, None);
    }

    /// Upstream schema canary. Claude Code has added fields to `usage` more than
    /// once (`speed`, `output_tokens_details`, `iterations`), and serde drops
    /// what it does not know, so drift is silent. This mirror declares every
    /// field we have seen and rejects anything else, over the fixture always and
    /// over the newest real session files when this machine has them.
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    #[allow(dead_code)] // Fields exist to declare the accepted schema, not to be read
    struct KnownUsage {
        input_tokens: Option<u64>,
        output_tokens: Option<u64>,
        cache_creation_input_tokens: Option<u64>,
        cache_read_input_tokens: Option<u64>,
        cache_creation: Option<KnownCacheCreation>,
        output_tokens_details: Option<KnownOutputTokensDetails>,
        server_tool_use: Option<KnownServerToolUse>,
        service_tier: Option<serde_json::Value>,
        speed: Option<serde_json::Value>,
        inference_geo: Option<serde_json::Value>,
        /// Per-request breakdown of the same counters; drift inside it does not
        /// change what we bill, so it stays opaque.
        iterations: Option<serde_json::Value>,
    }

    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    #[allow(dead_code)] // Fields exist to declare the accepted schema, not to be read
    struct KnownCacheCreation {
        ephemeral_5m_input_tokens: Option<u64>,
        ephemeral_1h_input_tokens: Option<u64>,
    }

    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    #[allow(dead_code)] // Fields exist to declare the accepted schema, not to be read
    struct KnownOutputTokensDetails {
        thinking_tokens: Option<u64>,
    }

    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    #[allow(dead_code)] // Fields exist to declare the accepted schema, not to be read
    struct KnownServerToolUse {
        web_search_requests: Option<u64>,
        web_fetch_requests: Option<u64>,
    }

    /// Newest `count` session files on this machine, empty when Claude Code has
    /// never run here (CI).
    fn recent_local_sessions(count: usize) -> Vec<PathBuf> {
        let Some(dir) = directories::BaseDirs::new().map(|d| d.home_dir().join(".claude")) else {
            return Vec::new();
        };
        // Same recursion as `file_pattern`: subagent transcripts nest deeper.
        let pattern = dir.join("projects").join("**").join("*.jsonl");
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

    /// Session files reach hundreds of MB, so stream and cap instead of reading
    /// the whole file: drift shows up in the first lines just as well.
    const CANARY_LINE_LIMIT: usize = 2_000;

    fn assert_usage_shape_known(path: &Path) {
        let file = File::open(path).unwrap();
        for (i, line) in BufReader::new(file)
            .lines()
            .take(CANARY_LINE_LIMIT)
            .enumerate()
        {
            let Ok(line) = line else { continue };
            if !line.contains("\"usage\"") {
                continue;
            }
            let Ok(value) = serde_json::from_str::<serde_json::Value>(&line) else {
                continue;
            };
            let Some(usage) = value.get("message").and_then(|m| m.get("usage")) else {
                continue;
            };
            if let Err(e) = serde_json::from_value::<KnownUsage>(usage.clone()) {
                panic!(
                    "Claude usage schema drifted at {}:{} — {}. Decide whether the new field \
                     affects cost, then add it here (and to ClaudeUsage if it does).",
                    path.display(),
                    i + 1,
                    e
                );
            }
        }
    }

    #[test]
    fn test_claude_usage_schema_has_no_unknown_fields() {
        assert_usage_shape_known(&fixture_path("claude/real-shape-session.jsonl"));
        for path in recent_local_sessions(5) {
            assert_usage_shape_known(&path);
        }
    }

    #[test]
    fn test_claude_parses_fast_speed() {
        let parser = ClaudeCodeParser::with_data_dir(PathBuf::from("/tmp"));
        let line = r#"{"type":"assistant","timestamp":"2026-08-18T10:00:00.000Z","message":{"model":"claude-opus-5","id":"msg-fast","usage":{"input_tokens":10,"output_tokens":5,"speed":"fast"}}}"#;
        let mut bytes = line.as_bytes().to_vec();

        let entry = parser.parse_line(&mut bytes).unwrap();

        assert!(entry.fast_speed, "speed=fast must mark the entry");
    }

    #[test]
    fn test_claude_standard_speed_is_not_fast() {
        let parser = ClaudeCodeParser::with_data_dir(PathBuf::from("/tmp"));
        let line = r#"{"type":"assistant","timestamp":"2026-08-18T10:00:00.000Z","message":{"model":"claude-opus-5","id":"msg-std","usage":{"input_tokens":10,"output_tokens":5,"speed":"standard"}}}"#;
        let mut bytes = line.as_bytes().to_vec();

        let entry = parser.parse_line(&mut bytes).unwrap();

        assert!(!entry.fast_speed);
    }
}
