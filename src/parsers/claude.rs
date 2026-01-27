//! Claude Code JSONL parser

use crate::types::{Result, ToktrackError, UsageEntry};
use chrono::{DateTime, Utc};
use serde::Deserialize;
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
}

#[derive(Deserialize)]
struct ClaudeMessage<'a> {
    model: Option<&'a str>,
    id: Option<&'a str>,
    usage: Option<ClaudeUsage>,
}

#[derive(Deserialize)]
struct ClaudeUsage {
    input_tokens: u64,
    output_tokens: u64,
    cache_creation_input_tokens: Option<u64>,
    cache_read_input_tokens: Option<u64>,
}

/// Parser for Claude Code usage data
pub struct ClaudeCodeParser {
    data_dir: PathBuf,
}

impl ClaudeCodeParser {
    /// Create a new parser with default data directory (~/.claude/projects/)
    pub fn new() -> Self {
        let home = directories::BaseDirs::new()
            .map(|d| d.home_dir().to_path_buf())
            .unwrap_or_else(|| PathBuf::from("."));
        Self {
            data_dir: home.join(".claude").join("projects"),
        }
    }

    /// Create a parser with a custom data directory (for testing)
    pub fn with_data_dir(data_dir: PathBuf) -> Self {
        Self { data_dir }
    }

    /// Parse a single JSONL line (zero-copy with borrowed strings)
    fn parse_line(&self, line: &mut [u8]) -> Option<UsageEntry> {
        if line.is_empty() {
            return None;
        }

        let data: ClaudeJsonLine = simd_json::from_slice(line).ok()?;

        // Only process lines with message and usage data
        let message = data.message.as_ref()?;
        let usage = message.usage.as_ref()?;

        let timestamp = DateTime::parse_from_rfc3339(data.timestamp)
            .map(|dt| dt.with_timezone(&Utc))
            .unwrap_or_else(|_| Utc::now());

        Some(UsageEntry {
            timestamp,
            model: message.model.map(String::from),
            input_tokens: usage.input_tokens,
            output_tokens: usage.output_tokens,
            cache_read_tokens: usage.cache_read_input_tokens.unwrap_or(0),
            cache_creation_tokens: usage.cache_creation_input_tokens.unwrap_or(0),
            cost_usd: data.cost_usd,
            message_id: message.id.map(String::from),
            request_id: data.request_id.map(String::from),
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

    fn data_dir(&self) -> PathBuf {
        self.data_dir.clone()
    }

    fn file_pattern(&self) -> &str {
        "**/*.jsonl"
    }

    fn parse_file(&self, path: &Path) -> Result<Vec<UsageEntry>> {
        let mut content = std::fs::read(path).map_err(ToktrackError::Io)?;
        let mut entries = Vec::new();
        let mut start = 0;

        for i in 0..content.len() {
            if content[i] == b'\n' {
                if start < i {
                    if let Some(entry) = self.parse_line(&mut content[start..i]) {
                        entries.push(entry);
                    }
                }
                start = i + 1;
            }
        }
        // Handle last line without trailing newline
        if start < content.len() {
            if let Some(entry) = self.parse_line(&mut content[start..]) {
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

    #[test]
    fn test_parse_claude_jsonl() {
        let parser = ClaudeCodeParser::with_data_dir(PathBuf::from("tests/fixtures"));
        let entries = parser
            .parse_file(&fixture_path("claude-sample.jsonl"))
            .unwrap();

        // Should parse 3 assistant messages (skipping user message and invalid line)
        assert_eq!(entries.len(), 3);
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

        // Third entry has no cache tokens, message_id, or request_id
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

        // Invalid JSON line should be skipped, not cause an error
        assert_eq!(entries.len(), 3);
    }

    #[test]
    fn test_skip_user_messages() {
        let parser = ClaudeCodeParser::with_data_dir(PathBuf::from("tests/fixtures"));
        let entries = parser
            .parse_file(&fixture_path("claude-sample.jsonl"))
            .unwrap();

        // User message has no usage, should be skipped
        // All entries should have input_tokens > 0
        assert!(entries.iter().all(|e| e.input_tokens > 0));
    }

    #[test]
    fn test_dedup_hash() {
        let parser = ClaudeCodeParser::with_data_dir(PathBuf::from("tests/fixtures"));
        let entries = parser
            .parse_file(&fixture_path("claude-sample.jsonl"))
            .unwrap();

        // First entry has both message_id and request_id
        assert_eq!(entries[0].dedup_hash(), Some("msg-001:req-001".to_string()));

        // Third entry has neither
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
}
