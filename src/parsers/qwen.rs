//! Qwen Code parser
//!
//! Qwen Code ≥ 0.18 writes a Claude-Code-style session log to
//! `~/.qwen/projects/<slug>/chats/<uuid>.jsonl`. Each line is a typed record
//! (`user` / `system` / `assistant` / `tool_result`); only `assistant` lines
//! carry token usage, in a top-level `usageMetadata` object. The same counts
//! are also emitted on a `system` / `ui_telemetry` `qwen-code.api_response`
//! line — we read only the `assistant` line so each turn is counted once.
//!
//! Older Qwen Code (≤ 0.17) used the Gemini-CLI fork format under
//! `~/.qwen/tmp/<hash>/chats/`. That layout is still collected here and parsed
//! via the Gemini-format reader (`source = "qwen"`) so historical data survives
//! the migration.

use crate::types::{Result, ToktrackError, UsageEntry};
use chrono::{DateTime, Utc};
use serde::Deserialize;
use std::collections::HashSet;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

use super::{CLIParser, GeminiParser};

/// A single line of the new-format session log. Field presence determines the
/// record type at runtime; only `assistant` + `usageMetadata` lines are usage.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct QwenLine<'a> {
    #[serde(default, borrow)]
    uuid: Option<&'a str>,
    #[serde(default, borrow)]
    session_id: Option<&'a str>,
    #[serde(default, borrow)]
    timestamp: Option<&'a str>,
    #[serde(default, rename = "type", borrow)]
    line_type: Option<&'a str>,
    #[serde(default, borrow)]
    cwd: Option<&'a str>,
    #[serde(default, borrow)]
    model: Option<&'a str>,
    #[serde(default)]
    usage_metadata: Option<QwenUsageMetadata>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct QwenUsageMetadata {
    #[serde(default)]
    prompt_token_count: u64,
    #[serde(default)]
    candidates_token_count: u64,
    #[serde(default)]
    cached_content_token_count: u64,
    #[serde(default)]
    thoughts_token_count: u64,
    #[serde(default)]
    total_token_count: Option<u64>,
}

/// Parser for Qwen Code usage data.
pub struct QwenParser {
    /// `~/.qwen` root (or `$QWEN_HOME`). New-format logs live under
    /// `projects/*/chats/`, legacy logs under `tmp/*/chats/`.
    data_dir: PathBuf,
    source: &'static str,
}

impl QwenParser {
    /// Create a parser with the default data directory.
    ///
    /// Honors `QWEN_HOME` (overrides `~/.qwen`).
    pub fn new() -> Self {
        let root = super::discovery::first_env_dir(&["QWEN_HOME"]).unwrap_or_else(|| {
            directories::BaseDirs::new()
                .map(|d| d.home_dir().join(".qwen"))
                .unwrap_or_else(|| {
                    eprintln!("[toktrack] Warning: Could not determine home directory");
                    PathBuf::from(".")
                })
        });
        Self {
            data_dir: root,
            source: "qwen",
        }
    }

    /// Create a parser rooted at a custom `.qwen` directory (for testing).
    #[allow(dead_code)] // Used in tests
    pub fn with_data_dir(data_dir: PathBuf) -> Self {
        Self {
            data_dir,
            source: "qwen",
        }
    }

    /// True when `path` is a legacy Gemini-fork log (`tmp/*/chats/...`) rather
    /// than a new-format `projects/*/chats/...` log.
    fn is_legacy_path(&self, path: &Path) -> bool {
        // Scan only the portion below `data_dir` so a home dir that itself
        // contains a "projects" component can't misroute a legacy `tmp/` file.
        let rel = path.strip_prefix(&self.data_dir).unwrap_or(path);
        !rel.components()
            .any(|c| c.as_os_str().eq_ignore_ascii_case("projects"))
    }

    /// Parse a new-format `projects/*/chats/<uuid>.jsonl` session file.
    fn parse_new_jsonl(&self, path: &Path) -> Result<Vec<UsageEntry>> {
        let file = File::open(path).map_err(ToktrackError::Io)?;
        let reader = BufReader::new(file);
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
            let parsed: QwenLine = match simd_json::from_slice(&mut bytes) {
                Ok(v) => v,
                Err(_) => continue,
            };

            // Only `assistant` lines carry usage. The same counts also appear on
            // a `ui_telemetry` `api_response` line; reading just `assistant`
            // avoids double-counting each turn.
            if parsed.line_type != Some("assistant") {
                continue;
            }
            let usage = match parsed.usage_metadata {
                Some(u) => u,
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

            entries.push(UsageEntry {
                timestamp,
                model: parsed.model.map(String::from),
                // `promptTokenCount` (Gemini semantics) INCLUDES cached tokens;
                // subtract so input_tokens is billable non-cached input and
                // cached isn't double-charged.
                input_tokens: usage
                    .prompt_token_count
                    .saturating_sub(usage.cached_content_token_count),
                // `candidatesTokenCount` (Gemini usageMetadata semantics) INCLUDES
                // `thoughtsTokenCount`; subtract so reasoning isn't double-counted
                // and total_tokens() reconciles with `totalTokenCount`. (This is the
                // opposite of the legacy Gemini JSONL format, where `tokens.thoughts`
                // is a separate, additive field.)
                output_tokens: usage
                    .candidates_token_count
                    .saturating_sub(usage.thoughts_token_count),
                cache_read_tokens: usage.cached_content_token_count,
                cache_creation_tokens: 0,
                reasoning_tokens: usage.thoughts_token_count,
                cache_creation_5m_tokens: 0,
                cache_creation_1h_tokens: 0,
                web_search_requests: 0,
                web_fetch_requests: 0,
                reported_total_tokens: usage.total_token_count,
                cost_usd: None,
                message_id: parsed.uuid.map(String::from),
                request_id: parsed.session_id.map(String::from),
                source: Some(self.source.into()),
                provider: None,
                // The session's working directory is on every line — use it as
                // the project identifier directly (no `.project_root` sidecar).
                project: parsed.cwd.map(String::from),
            });
        }

        Ok(entries)
    }
}

impl Default for QwenParser {
    fn default() -> Self {
        Self::new()
    }
}

impl CLIParser for QwenParser {
    fn name(&self) -> &str {
        self.source
    }

    fn data_dir(&self) -> &Path {
        &self.data_dir
    }

    /// Representative pattern. Actual collection uses several globs via
    /// `collect_files` (new-format plus legacy layouts).
    fn file_pattern(&self) -> &str {
        "projects/*/chats/*.jsonl"
    }

    fn collect_files(&self) -> Vec<PathBuf> {
        let patterns = [
            // new format (Qwen Code ≥ 0.18)
            "projects/*/chats/*.jsonl",
            // legacy Gemini-fork format (Qwen Code ≤ 0.17)
            "tmp/*/chats/session-*.jsonl",
            "tmp/*/chats/session-*.json",
            "tmp/*/chats/*/*.jsonl",
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
        if self.is_legacy_path(path) {
            // Legacy Gemini-fork format — reuse the Gemini reader, tagged "qwen".
            return GeminiParser::with_data_dir_source(self.data_dir.clone(), self.source)
                .parse_file(path);
        }
        match path.extension().and_then(|s| s.to_str()) {
            Some("jsonl") => self.parse_new_jsonl(path),
            _ => Ok(Vec::new()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn root() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("fixtures")
            .join("qwen_v2")
    }

    fn new_session() -> PathBuf {
        root()
            .join("projects")
            .join("g--scripts-test-qwen")
            .join("chats")
            .join("sess-new.jsonl")
    }

    // `QWEN_HOME` is process-global; splitting the default-dir and env-override
    // assertions into two `#[test]`s races because cargo runs tests in parallel
    // and each test's restore step can observe the other's `set_var`. Keep both
    // scenarios in one test so they execute serially within a single thread.
    #[test]
    fn test_parser_name_and_data_dir_with_env() {
        let saved = std::env::var("QWEN_HOME").ok();

        std::env::remove_var("QWEN_HOME");
        let parser = QwenParser::new();
        assert_eq!(parser.name(), "qwen");
        assert!(parser.data_dir().ends_with(".qwen"));

        std::env::set_var("QWEN_HOME", "/tmp/toktrack-qwen-env-test");
        assert_eq!(
            QwenParser::new().data_dir(),
            Path::new("/tmp/toktrack-qwen-env-test")
        );

        match saved {
            Some(v) => std::env::set_var("QWEN_HOME", v),
            None => std::env::remove_var("QWEN_HOME"),
        }
    }

    #[test]
    fn test_new_format_counts_only_assistant_usage_lines() {
        let parser = QwenParser::with_data_dir(root());
        let entries = parser.parse_file(&new_session()).unwrap();
        let ids: Vec<_> = entries
            .iter()
            .filter_map(|e| e.message_id.clone())
            .collect();
        assert_eq!(ids, vec!["a1".to_string(), "a2".to_string()]);
    }

    #[test]
    fn test_new_format_token_mapping() {
        let parser = QwenParser::with_data_dir(root());
        let entries = parser.parse_file(&new_session()).unwrap();

        let a1 = &entries[0];
        assert_eq!(a1.model, Some("glm-5.2".to_string()));
        assert_eq!(a1.input_tokens, 800);
        assert_eq!(a1.output_tokens, 400);
        assert_eq!(a1.cache_read_tokens, 200);
        assert_eq!(a1.cache_creation_tokens, 0);
        assert_eq!(a1.reasoning_tokens, 100);
        assert_eq!(a1.reported_total_tokens, Some(1500));
        assert_eq!(a1.total_tokens(), 1500);
        assert_eq!(a1.source, Some("qwen".into()));
        assert_eq!(a1.request_id, Some("S".to_string()));
        assert_eq!(a1.project.as_deref(), Some("G:\\proj"));

        let a2 = &entries[1];
        assert_eq!(a2.input_tokens, 300);
        assert_eq!(a2.output_tokens, 50);
        assert_eq!(a2.cache_read_tokens, 0);
        assert_eq!(a2.reasoning_tokens, 0);
        assert_eq!(a2.total_tokens(), 350);
    }

    #[test]
    fn test_telemetry_line_not_double_counted() {
        // sess-new.jsonl carries a `ui_telemetry` api_response line with the same
        // numbers as a1; if it were counted we'd see 3 entries, not 2.
        let parser = QwenParser::with_data_dir(root());
        let entries = parser.parse_file(&new_session()).unwrap();
        assert_eq!(entries.len(), 2);
    }

    #[test]
    fn test_collect_files_finds_new_and_legacy() {
        let parser = QwenParser::with_data_dir(root());
        let files = parser.collect_files();
        assert_eq!(files.len(), 2, "got files: {:?}", files);
        assert!(files
            .iter()
            .any(|f| f.to_string_lossy().contains("projects")));
        assert!(files.iter().any(|f| f.to_string_lossy().contains("tmp")));
    }

    #[test]
    fn test_is_legacy_path_ignores_projects_in_data_dir() {
        // Regression: a home/data dir that itself contains a `projects` component
        // must not cause a legacy `tmp/` log to be misclassified as new-format.
        // Only the segment below `data_dir` should be scanned for `projects`.
        let data_dir = PathBuf::from("/home/user/projects/.qwen");
        let parser = QwenParser::with_data_dir(data_dir.clone());

        let legacy = data_dir.join("tmp/hash1/chats/session-1.jsonl");
        assert!(
            parser.is_legacy_path(&legacy),
            "tmp/ log under a `projects`-containing data dir must stay legacy"
        );

        let new = data_dir.join("projects/slug/chats/sess.jsonl");
        assert!(
            !parser.is_legacy_path(&new),
            "projects/ log must be classified as new-format"
        );
    }

    #[test]
    fn test_parse_all_aggregates_new_and_legacy() {
        let parser = QwenParser::with_data_dir(root());
        let entries = parser.parse_all().unwrap();
        assert_eq!(entries.len(), 3);
        assert!(entries.iter().all(|e| e.source == Some("qwen".into())));
    }

    #[test]
    fn test_legacy_path_parsed_with_gemini_semantics() {
        let legacy = root()
            .join("tmp")
            .join("hash1")
            .join("chats")
            .join("session-2026-06-01T10-00-00-qwen0001.jsonl");
        let parser = QwenParser::with_data_dir(root());
        let entries = parser.parse_file(&legacy).unwrap();
        assert_eq!(entries.len(), 1);
        let e = &entries[0];
        assert_eq!(e.model, Some("qwen3-coder".to_string()));
        assert_eq!(e.input_tokens, 160);
        assert_eq!(e.cache_read_tokens, 40);
        assert_eq!(e.reported_total_tokens, Some(300));
        assert_eq!(e.project.as_deref(), Some("/work/legacy-demo"));
    }
}
