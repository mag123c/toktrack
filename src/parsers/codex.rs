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
    model: Option<String>,
    #[serde(default)]
    info: Option<CodexInfo>,
    #[serde(default)]
    id: Option<String>,
}

#[derive(Deserialize)]
struct CodexInfo {
    total_token_usage: Option<CodexTokenUsage>,
}

#[derive(Deserialize)]
struct CodexTokenUsage {
    input_tokens: u64,
    output_tokens: u64,
    #[serde(default)]
    cached_input_tokens: u64,
}

/// Parser for Codex CLI usage data
pub struct CodexParser {
    data_dir: PathBuf,
}

impl CodexParser {
    /// Create a new parser with default data directory (~/.codex/sessions/)
    pub fn new() -> Self {
        let home = directories::BaseDirs::new()
            .map(|d| d.home_dir().to_path_buf())
            .unwrap_or_else(|| PathBuf::from("."));
        Self {
            data_dir: home.join(".codex").join("sessions"),
        }
    }

    /// Create a parser with a custom data directory (for testing)
    #[allow(dead_code)]
    pub fn with_data_dir(data_dir: PathBuf) -> Self {
        Self { data_dir }
    }

    /// Parse a single JSONL line and return optional UsageEntry with model info
    fn parse_line(
        &self,
        line: &mut [u8],
        current_model: &Option<String>,
        session_id: &Option<String>,
    ) -> ParseResult {
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

        // Handle turn_context lines - extract model info from payload.model
        if data.line_type == "turn_context" {
            if let Some(ref model) = payload.model {
                return ParseResult::Model(model.clone());
            }
            return ParseResult::Skip;
        }

        // Handle session_meta lines - extract session id
        if data.line_type == "session_meta" {
            if let Some(ref id) = payload.id {
                return ParseResult::SessionId(id.clone());
            }
            return ParseResult::Skip;
        }

        // Only process event_msg lines with token_count payload
        if data.line_type != "event_msg" {
            return ParseResult::Skip;
        }

        // Check for token_count type
        let payload_type = match &payload.payload_type {
            Some(t) => t,
            None => return ParseResult::Skip,
        };

        if payload_type != "token_count" {
            return ParseResult::Skip;
        }

        let info = match &payload.info {
            Some(i) => i,
            None => return ParseResult::Skip,
        };

        let usage = match &info.total_token_usage {
            Some(u) => u,
            None => return ParseResult::Skip,
        };

        let timestamp = match DateTime::parse_from_rfc3339(data.timestamp) {
            Ok(dt) => dt.with_timezone(&Utc),
            Err(_) => {
                eprintln!(
                    "[toktrack] Warning: Invalid timestamp '{}', using current time",
                    data.timestamp
                );
                Utc::now()
            }
        };

        let entry = UsageEntry {
            timestamp,
            model: current_model.clone(),
            input_tokens: usage.input_tokens,
            output_tokens: usage.output_tokens,
            cache_read_tokens: usage.cached_input_tokens,
            cache_creation_tokens: 0,
            thinking_tokens: 0,
            cost_usd: None,
            message_id: session_id.clone(),
            request_id: None,
            source: Some("codex".into()),
            provider: None,
        };

        ParseResult::Entry(entry)
    }
}

/// Result of parsing a single line
enum ParseResult {
    Skip,
    Model(String),
    SessionId(String),
    Entry(UsageEntry),
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

    fn parse_file(&self, path: &Path) -> Result<Vec<UsageEntry>> {
        let file = File::open(path).map_err(ToktrackError::Io)?;
        let reader = BufReader::new(file);
        let mut last_entry: Option<UsageEntry> = None;
        let mut current_model: Option<String> = None;
        let mut session_id: Option<String> = None;

        for line_result in reader.lines() {
            let line = match line_result {
                Ok(l) => l,
                Err(_) => continue,
            };

            if line.is_empty() {
                continue;
            }

            let mut line_bytes = line.into_bytes();
            match self.parse_line(&mut line_bytes, &current_model, &session_id) {
                ParseResult::Skip => {}
                ParseResult::Model(m) => current_model = Some(m),
                ParseResult::SessionId(id) => session_id = Some(id),
                // total_token_usage is cumulative; keep only the last
                // token_count event which holds the session total.
                ParseResult::Entry(entry) => last_entry = Some(entry),
            }
        }

        Ok(last_entry.into_iter().collect())
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
            .join("codex")
            .join(name)
    }

    #[test]
    fn test_parse_codex_jsonl_uses_last_cumulative_total() {
        let parser = CodexParser::with_data_dir(PathBuf::from("tests/fixtures/codex"));
        let entries = parser
            .parse_file(&fixture_path("sample-session.jsonl"))
            .unwrap();

        // total_token_usage is cumulative; only the last token_count
        // event should be emitted per session file.
        assert_eq!(entries.len(), 1);
    }

    #[test]
    fn test_parse_last_entry_has_final_totals() {
        let parser = CodexParser::with_data_dir(PathBuf::from("tests/fixtures/codex"));
        let entries = parser
            .parse_file(&fixture_path("sample-session.jsonl"))
            .unwrap();

        // The single entry should carry the last cumulative totals
        // and the model active at that point.
        let entry = &entries[0];
        assert_eq!(entry.model, Some("gpt-4.1".to_string()));
        assert_eq!(entry.input_tokens, 500);
        assert_eq!(entry.output_tokens, 200);
        assert_eq!(entry.cache_read_tokens, 100);
        assert_eq!(entry.cache_creation_tokens, 0);
        assert_eq!(entry.thinking_tokens, 0);
        assert_eq!(entry.source, Some("codex".into()));
        assert_eq!(entry.message_id, Some("session-001".to_string()));
    }

    #[test]
    fn test_skip_invalid_lines() {
        let parser = CodexParser::with_data_dir(PathBuf::from("tests/fixtures/codex"));
        let entries = parser
            .parse_file(&fixture_path("sample-session.jsonl"))
            .unwrap();

        // Invalid JSON line and other event types should be skipped;
        // only one entry (last cumulative total) should remain.
        assert_eq!(entries.len(), 1);
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
    fn test_parse_nonexistent_file() {
        let parser = CodexParser::new();
        let result = parser.parse_file(Path::new("/nonexistent/file.jsonl"));
        assert!(result.is_err());
    }
}
