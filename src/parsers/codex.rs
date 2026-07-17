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

/// Parser for Codex CLI usage data
pub struct CodexParser {
    data_dir: PathBuf,
}

impl CodexParser {
    /// Create a new parser with default data directory.
    ///
    /// Honors `CODEX_HOME` (root; replaces `~/.codex`); sessions live under
    /// `<root>/sessions`.
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
            data_dir: root.join("sessions"),
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

    fn parse_file(&self, path: &Path) -> Result<Vec<UsageEntry>> {
        let file = File::open(path).map_err(ToktrackError::Io)?;
        let reader = BufReader::new(file);
        let mut entries: Vec<UsageEntry> = Vec::new();
        let mut current_model: Option<String> = None;
        let mut session_id: Option<String> = None;
        let mut current_provider: Option<String> = None;
        let mut current_project: Option<String> = None;
        let mut prev_totals = CodexTokenUsage {
            input_tokens: 0,
            output_tokens: 0,
            cached_input_tokens: 0,
        };

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
                }
                ParseResult::TokenCount(data) => {
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
}
