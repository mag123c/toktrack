//! OpenCode CLI JSON parser

use crate::types::{Result, ToktrackError, UsageEntry};
use chrono::{DateTime, Utc};
use serde::Deserialize;
use std::fs;
use std::path::{Path, PathBuf};

use super::CLIParser;

/// OpenCode message JSON structure
#[derive(Deserialize)]
struct OpenCodeMessage {
    id: String,
    #[serde(rename = "sessionID")]
    session_id: String,
    #[serde(rename = "modelID")]
    model_id: Option<String>,
    #[serde(rename = "providerID")]
    provider_id: Option<String>,
    time: OpenCodeTime,
    tokens: Option<OpenCodeTokens>,
    cost: Option<f64>,
}

#[derive(Deserialize)]
struct OpenCodeTime {
    created: u64, // Unix timestamp in milliseconds
}

#[derive(Deserialize)]
struct OpenCodeTokens {
    input: u64,
    output: u64,
    #[serde(default)]
    reasoning: u64,
    #[serde(default)]
    cache: Option<OpenCodeCache>,
}

#[derive(Deserialize)]
struct OpenCodeCache {
    read: u64,
    write: u64,
}

/// Parser for OpenCode CLI usage data
pub struct OpenCodeParser {
    data_dir: PathBuf,
}

impl OpenCodeParser {
    /// Create a new parser with default data directory (~/.local/share/opencode/storage/message)
    /// OpenCode uses XDG standard, so we use ~/.local/share on all platforms
    pub fn new() -> Self {
        let data_dir = directories::BaseDirs::new()
            .map(|d| d.home_dir().join(".local").join("share"))
            .unwrap_or_else(|| PathBuf::from("."))
            .join("opencode")
            .join("storage")
            .join("message");
        Self { data_dir }
    }

    /// Create a parser with a custom data directory (for testing)
    #[allow(dead_code)]
    pub fn with_data_dir(data_dir: PathBuf) -> Self {
        Self { data_dir }
    }
}

impl Default for OpenCodeParser {
    fn default() -> Self {
        Self::new()
    }
}

impl CLIParser for OpenCodeParser {
    fn name(&self) -> &str {
        "opencode"
    }

    fn data_dir(&self) -> &Path {
        &self.data_dir
    }

    fn file_pattern(&self) -> &str {
        "**/msg_*.json"
    }

    fn parse_file(&self, path: &Path) -> Result<Vec<UsageEntry>> {
        let mut content = fs::read_to_string(path).map_err(ToktrackError::Io)?;
        // SAFETY: simd_json requires mutable access for in-place parsing
        let message: OpenCodeMessage = unsafe {
            simd_json::from_str(&mut content).map_err(|e| ToktrackError::Parse(e.to_string()))?
        };

        // Skip messages without token data
        let tokens = match message.tokens {
            Some(t) => t,
            None => return Ok(Vec::new()),
        };

        let timestamp = i64::try_from(message.time.created)
            .ok()
            .and_then(DateTime::from_timestamp_millis)
            .unwrap_or_else(|| {
                eprintln!(
                    "[toktrack] Warning: Invalid timestamp '{}', using current time",
                    message.time.created
                );
                Utc::now()
            });

        let (cache_read, cache_write) = match tokens.cache {
            Some(c) => (c.read, c.write),
            None => (0, 0),
        };

        let entry = UsageEntry {
            timestamp,
            model: message.model_id,
            input_tokens: tokens.input,
            output_tokens: tokens.output,
            cache_read_tokens: cache_read,
            cache_creation_tokens: cache_write,
            thinking_tokens: tokens.reasoning,
            cost_usd: message.cost,
            message_id: Some(message.id),
            request_id: Some(message.session_id),
            source: Some("opencode".into()),
            provider: message.provider_id,
        };

        Ok(vec![entry])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn fixture_dir() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("fixtures")
            .join("opencode")
            .join("storage")
            .join("message")
    }

    fn fixture_path(filename: &str) -> PathBuf {
        fixture_dir().join("ses_test").join(filename)
    }

    #[test]
    fn test_parse_opencode_message() {
        let parser = OpenCodeParser::with_data_dir(fixture_dir());
        let entries = parser.parse_file(&fixture_path("msg_001.json")).unwrap();

        assert_eq!(entries.len(), 1);
    }

    #[test]
    fn test_parse_first_entry_details() {
        let parser = OpenCodeParser::with_data_dir(fixture_dir());
        let entries = parser.parse_file(&fixture_path("msg_001.json")).unwrap();

        let entry = &entries[0];
        assert_eq!(entry.model, Some("claude-sonnet-4-20250514".to_string()));
        assert_eq!(entry.input_tokens, 1000);
        assert_eq!(entry.output_tokens, 500);
        assert_eq!(entry.cache_read_tokens, 100);
        assert_eq!(entry.cache_creation_tokens, 50);
        assert_eq!(entry.thinking_tokens, 0);
        assert_eq!(entry.cost_usd, Some(0.05));
        assert_eq!(entry.source, Some("opencode".into()));
        assert_eq!(entry.message_id, Some("msg_001".to_string()));
        assert_eq!(entry.request_id, Some("ses_test".to_string()));
    }

    #[test]
    fn test_parse_entry_with_reasoning_tokens() {
        let parser = OpenCodeParser::with_data_dir(fixture_dir());
        let entries = parser.parse_file(&fixture_path("msg_002.json")).unwrap();

        let entry = &entries[0];
        assert_eq!(entry.input_tokens, 2000);
        assert_eq!(entry.output_tokens, 800);
        assert_eq!(entry.cache_read_tokens, 200);
        assert_eq!(entry.cache_creation_tokens, 100);
        assert_eq!(entry.thinking_tokens, 150);
        assert_eq!(entry.cost_usd, Some(0.12));
    }

    #[test]
    fn test_skip_message_without_tokens() {
        let parser = OpenCodeParser::with_data_dir(fixture_dir());
        let entries = parser
            .parse_file(&fixture_path("msg_003_no_tokens.json"))
            .unwrap();

        assert!(entries.is_empty());
    }

    #[test]
    fn test_parser_name() {
        let parser = OpenCodeParser::new();
        assert_eq!(parser.name(), "opencode");
    }

    #[test]
    fn test_parser_file_pattern() {
        let parser = OpenCodeParser::new();
        assert_eq!(parser.file_pattern(), "**/msg_*.json");
    }

    #[test]
    fn test_parse_nonexistent_file() {
        let parser = OpenCodeParser::new();
        let result = parser.parse_file(Path::new("/nonexistent/file.json"));
        assert!(result.is_err());
    }

    #[test]
    fn test_total_tokens_calculation() {
        let parser = OpenCodeParser::with_data_dir(fixture_dir());
        let entries = parser.parse_file(&fixture_path("msg_001.json")).unwrap();

        // 1000 + 500 + 100 + 50 + 0 = 1650
        assert_eq!(entries[0].total_tokens(), 1650);
    }

    #[test]
    fn test_total_tokens_with_reasoning() {
        let parser = OpenCodeParser::with_data_dir(fixture_dir());
        let entries = parser.parse_file(&fixture_path("msg_002.json")).unwrap();

        // 2000 + 800 + 200 + 100 + 150 = 3250
        assert_eq!(entries[0].total_tokens(), 3250);
    }
}
