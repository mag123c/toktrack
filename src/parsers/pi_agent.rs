//! PI Agent JSONL parser
//!
//! # Branch counting policy
//!
//! PI Agent v2/v3 sessions form a parent/child tree via `parentId`. The
//! `/tree` command lets multiple assistant messages share the same parent
//! within a single file (divergent branches). Each such message represents
//! a real, billed API call, so we count **all** assistant messages found
//! in a file — including those on abandoned branches.
//!
//! The `/fork` command produces a new session file that copies its parent's
//! history; the duplicated entries share their original `message_id`s and
//! are deduplicated by `ParserRegistry::parse_and_dedup` (see `mod.rs`).

use crate::types::{Result, ToktrackError, UsageEntry};
use chrono::{DateTime, TimeZone, Utc};
use serde::Deserialize;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

use super::CLIParser;

/// Event line in PI Agent JSONL sessions
#[derive(Deserialize)]
struct PiAgentLine<'a> {
    #[serde(rename = "type")]
    line_type: &'a str,
    id: Option<&'a str>,
    timestamp: Option<&'a str>,
    message: Option<PiAgentMessage<'a>>,
    /// Working directory of the session (present on the leading `session` line)
    /// — used as the project identifier. Verified against live PI Agent v3
    /// session files (top-level `"cwd"` on the `session` line).
    cwd: Option<&'a str>,
}

#[derive(Deserialize)]
struct PiAgentMessage<'a> {
    role: Option<&'a str>,
    model: Option<&'a str>,
    provider: Option<&'a str>,
    usage: Option<PiAgentUsage>,
}

#[derive(Deserialize)]
struct PiAgentUsage {
    #[serde(default)]
    input: u64,
    #[serde(default)]
    output: u64,
    #[serde(rename = "cacheRead", default)]
    cache_read: u64,
    #[serde(rename = "cacheWrite", default)]
    cache_write: u64,
    cost: Option<PiAgentCost>,
}

#[derive(Deserialize)]
struct PiAgentCost {
    total: Option<f64>,
}

/// Parsed parser state for one line
struct PiAgentEvent {
    timestamp: DateTime<Utc>,
    message_id: Option<String>,
    model: Option<String>,
    provider: Option<String>,
    usage: PiAgentUsage,
}

enum ParseResult {
    Skip,
    Session {
        id: Option<String>,
        cwd: Option<String>,
    },
    Usage(PiAgentEvent),
}

/// Parser for PI Agent usage data
pub struct PiAgentParser {
    data_dir: PathBuf,
}

impl PiAgentParser {
    /// Create a new parser with default data directory (~/.pi/agent/sessions/).
    ///
    /// Honors `PI_CODING_AGENT_SESSION_DIR` (== `--session-dir`, canonical since
    /// v0.71.0) with precedence over the legacy `PI_AGENT_DIR`.
    pub fn new() -> Self {
        let data_dir =
            super::discovery::first_env_dir(&["PI_CODING_AGENT_SESSION_DIR", "PI_AGENT_DIR"])
                .or_else(|| {
                    directories::BaseDirs::new()
                        .map(|d| d.home_dir().join(".pi").join("agent").join("sessions"))
                })
                .unwrap_or_else(|| {
                    eprintln!("[toktrack] Warning: Could not determine home directory");
                    PathBuf::from(".")
                });

        Self { data_dir }
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

        let line_data: PiAgentLine = match simd_json::from_slice(line) {
            Ok(data) => data,
            Err(_) => return ParseResult::Skip,
        };

        match line_data.line_type {
            "session" => {
                // Capture the session id (used as request_id) and the cwd (used
                // as the project). Emit even if id is absent so the project still
                // gets recorded for this file's entries.
                if line_data.id.is_none() && line_data.cwd.is_none() {
                    return ParseResult::Skip;
                }
                ParseResult::Session {
                    id: line_data.id.map(String::from),
                    cwd: line_data.cwd.map(String::from),
                }
            }
            "message" => {
                let message = match line_data.message {
                    Some(message) => message,
                    None => return ParseResult::Skip,
                };

                let usage = match message.usage {
                    Some(usage) => usage,
                    None => return ParseResult::Skip,
                };

                if message.role != Some("assistant") {
                    return ParseResult::Skip;
                }

                let timestamp = match line_data.timestamp.and_then(parse_timestamp) {
                    Some(ts) => ts,
                    None => return ParseResult::Skip,
                };

                ParseResult::Usage(PiAgentEvent {
                    timestamp,
                    message_id: line_data.id.map(String::from),
                    model: message.model.map(String::from),
                    provider: message.provider.map(String::from),
                    usage,
                })
            }
            _ => ParseResult::Skip,
        }
    }
}

fn parse_timestamp(value: &str) -> Option<DateTime<Utc>> {
    if let Ok(ts) = DateTime::parse_from_rfc3339(value) {
        return Some(ts.with_timezone(&Utc));
    }

    value
        .parse::<i64>()
        .ok()
        .and_then(|ms| Utc.timestamp_millis_opt(ms).single())
}

impl Default for PiAgentParser {
    fn default() -> Self {
        Self::new()
    }
}

impl CLIParser for PiAgentParser {
    fn name(&self) -> &str {
        "pi-agent"
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
        let mut current_session: Option<String> = None;
        let mut current_project: Option<String> = None;

        for line_result in reader.lines() {
            let line = match line_result {
                Ok(l) => l,
                Err(_) => continue,
            };

            let mut line_bytes = line.into_bytes();
            match self.parse_line(&mut line_bytes) {
                ParseResult::Skip => {}
                ParseResult::Session { id, cwd } => {
                    if id.is_some() {
                        current_session = id;
                    }
                    if cwd.is_some() {
                        current_project = cwd;
                    }
                }
                ParseResult::Usage(event) => {
                    entries.push(UsageEntry {
                        timestamp: event.timestamp,
                        model: event.model,
                        input_tokens: event.usage.input,
                        output_tokens: event.usage.output,
                        cache_read_tokens: event.usage.cache_read,
                        cache_creation_tokens: event.usage.cache_write,
                        reasoning_tokens: 0,
                        cache_creation_5m_tokens: 0,
                        cache_creation_1h_tokens: 0,
                        web_search_requests: 0,
                        web_fetch_requests: 0,
                        reported_total_tokens: None,
                        cost_usd: event.usage.cost.and_then(|c| c.total),
                        message_id: event.message_id,
                        request_id: current_session.clone(),
                        source: Some("pi-agent".into()),
                        provider: event.provider,
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

    fn fixture_path(name: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("fixtures")
            .join("pi_agent")
            .join(name)
    }

    #[test]
    fn test_parse_pi_agent_jsonl() {
        let parser = PiAgentParser::with_data_dir(PathBuf::from("tests/fixtures"));
        let entries = parser
            .parse_file(&fixture_path("sample-session.jsonl"))
            .unwrap();

        assert_eq!(entries.len(), 1);
    }

    #[test]
    fn test_parse_pi_agent_assistant_fields() {
        let parser = PiAgentParser::with_data_dir(PathBuf::from("tests/fixtures"));
        let entries = parser
            .parse_file(&fixture_path("sample-session.jsonl"))
            .unwrap();

        let entry = &entries[0];
        assert_eq!(entry.model, Some("gpt-5.3-codex".to_string()));
        assert_eq!(entry.provider, Some("openai-codex".to_string()));
        assert_eq!(entry.input_tokens, 1224);
        assert_eq!(entry.output_tokens, 189);
        assert_eq!(entry.message_id, Some("9687b1d5".to_string()));
        assert_eq!(
            entry.request_id,
            Some("00000000-0000-0000-0000-000000000001".to_string())
        );
        assert_eq!(entry.cost_usd, Some(0.004788));
    }

    #[test]
    fn test_parse_pi_agent_extracts_project_from_session_cwd() {
        let parser = PiAgentParser::with_data_dir(PathBuf::from("tests/fixtures"));
        let entries = parser
            .parse_file(&fixture_path("sample-session.jsonl"))
            .unwrap();

        assert_eq!(entries[0].project.as_deref(), Some("/tmp/example-project"));
    }

    #[test]
    fn test_parse_pi_agent_skips_non_assistant_lines() {
        let parser = PiAgentParser::with_data_dir(PathBuf::from("tests/fixtures"));
        let entries = parser
            .parse_file(&fixture_path("sample-session.jsonl"))
            .unwrap();

        assert_eq!(entries.len(), 1);
    }

    #[test]
    fn test_parser_name() {
        let parser = PiAgentParser::new();
        assert_eq!(parser.name(), "pi-agent");
    }

    #[test]
    fn test_parser_file_pattern() {
        let parser = PiAgentParser::new();
        assert_eq!(parser.file_pattern(), "**/*.jsonl");
    }

    #[test]
    fn test_pi_session_dir_env_takes_precedence() {
        let saved_s = std::env::var("PI_CODING_AGENT_SESSION_DIR").ok();
        let saved_a = std::env::var("PI_AGENT_DIR").ok();
        std::env::set_var("PI_AGENT_DIR", "/tmp/toktrack-pi-legacy");
        std::env::set_var("PI_CODING_AGENT_SESSION_DIR", "/tmp/toktrack-pi-session");
        assert_eq!(
            PiAgentParser::new().data_dir(),
            Path::new("/tmp/toktrack-pi-session")
        );
        match saved_s {
            Some(v) => std::env::set_var("PI_CODING_AGENT_SESSION_DIR", v),
            None => std::env::remove_var("PI_CODING_AGENT_SESSION_DIR"),
        }
        match saved_a {
            Some(v) => std::env::set_var("PI_AGENT_DIR", v),
            None => std::env::remove_var("PI_AGENT_DIR"),
        }
    }

    #[test]
    fn test_parse_nonexistent_file() {
        let parser = PiAgentParser::new();
        let result = parser.parse_file(Path::new("/nonexistent/file.jsonl"));
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_line_cost_extracted_from_total() {
        let parser = PiAgentParser::with_data_dir(PathBuf::from("tests/fixtures"));
        let entries = parser
            .parse_file(&fixture_path("sample-session.jsonl"))
            .unwrap();

        assert_eq!(entries[0].cost_usd, Some(0.004788));
    }

    #[test]
    fn test_parse_timestamp_from_ms_or_rfc3339() {
        assert!(parse_timestamp("2026-02-20T10:00:00Z").is_some());
        assert!(parse_timestamp("1708425600000").is_some());
        assert!(parse_timestamp("invalid").is_none());
    }
}
