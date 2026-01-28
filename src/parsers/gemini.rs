//! Gemini CLI JSON parser

use crate::types::{Result, ToktrackError, UsageEntry};
use chrono::{DateTime, Utc};
use serde::Deserialize;
use std::fs;
use std::path::{Path, PathBuf};

// Using simd_json for consistency with other parsers

use super::CLIParser;

/// Gemini session JSON structure
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

/// Parser for Gemini CLI usage data
pub struct GeminiParser {
    data_dir: PathBuf,
}

impl GeminiParser {
    /// Create a new parser with default data directory (~/.gemini/tmp/)
    pub fn new() -> Self {
        let home = directories::BaseDirs::new()
            .map(|d| d.home_dir().to_path_buf())
            .unwrap_or_else(|| PathBuf::from("."));
        Self {
            data_dir: home.join(".gemini").join("tmp"),
        }
    }

    /// Create a parser with a custom data directory (for testing)
    #[allow(dead_code)]
    pub fn with_data_dir(data_dir: PathBuf) -> Self {
        Self { data_dir }
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

    fn data_dir(&self) -> PathBuf {
        self.data_dir.clone()
    }

    fn file_pattern(&self) -> &str {
        "*/chats/session-*.json"
    }

    fn parse_file(&self, path: &Path) -> Result<Vec<UsageEntry>> {
        let mut content = fs::read_to_string(path).map_err(ToktrackError::Io)?;
        // SAFETY: simd_json requires mutable access for in-place parsing
        let session: GeminiSession = unsafe {
            simd_json::from_str(&mut content).map_err(|e| ToktrackError::Parse(e.to_string()))?
        };

        let mut entries = Vec::new();

        for msg in session.messages {
            // Only process gemini type messages
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
                        "[toktrack] Warning: Invalid timestamp '{}', using current time",
                        msg.timestamp
                    );
                    Utc::now()
                }
            };

            entries.push(UsageEntry {
                timestamp,
                model: session.model.clone(),
                input_tokens: tokens.input,
                output_tokens: tokens.output,
                cache_read_tokens: tokens.cached,
                cache_creation_tokens: 0,
                thinking_tokens: tokens.thoughts,
                cost_usd: None,
                message_id: Some(msg.id),
                request_id: Some(session.session_id.clone()),
                source: Some("gemini".into()),
            });
        }

        Ok(entries)
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

    #[test]
    fn test_parse_gemini_json() {
        let parser = GeminiParser::with_data_dir(PathBuf::from("tests/fixtures/gemini"));
        let entries = parser.parse_file(&fixture_path()).unwrap();

        // Should parse 2 gemini messages (skipping user, error, info)
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

        // All entries should have gemini source
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
        assert_eq!(parser.file_pattern(), "*/chats/session-*.json");
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

        // First entry: 100 + 50 + 20 + 0 + 30 = 200
        assert_eq!(entries[0].total_tokens(), 200);

        // Second entry: 250 + 150 + 50 + 0 + 100 = 550
        assert_eq!(entries[1].total_tokens(), 550);
    }
}
