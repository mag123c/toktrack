//! Parser traits and implementations for AI CLI tools

mod claude;
mod codex;
pub mod discovery;
mod gemini;
mod opencode;
mod pi_agent;

pub use claude::ClaudeCodeParser;
pub use codex::CodexParser;
pub use gemini::GeminiParser;
pub use opencode::OpenCodeParser;
pub use pi_agent::PiAgentParser;

use crate::types::{Result, UsageEntry};
use rayon::prelude::*;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// Trait for parsing usage data from AI CLI tools
pub trait CLIParser: Send + Sync {
    /// Parser name (e.g., "claude-code")
    #[allow(dead_code)] // Part of trait API, used in tests
    fn name(&self) -> &str;

    /// Data directory to scan for usage files
    fn data_dir(&self) -> &Path;

    /// Glob pattern for finding usage files (e.g., "**/*.jsonl")
    fn file_pattern(&self) -> &str;

    /// Parse a single file and return usage entries
    fn parse_file(&self, path: &Path) -> Result<Vec<UsageEntry>>;

    /// Parse all files in parallel using rayon, with deduplication
    fn parse_all(&self) -> Result<Vec<UsageEntry>> {
        let files = self.collect_files();
        Self::parse_and_dedup(self, &files)
    }

    /// Parse only files modified since `since`, with deduplication.
    /// Falls back to including files whose mtime cannot be read.
    fn parse_recent_files(&self, since: SystemTime) -> Result<Vec<UsageEntry>> {
        let all_files = self.collect_files();
        let recent: Vec<PathBuf> = all_files
            .into_iter()
            .filter(|f| {
                f.metadata()
                    .and_then(|m| m.modified())
                    .map(|mtime| mtime >= since)
                    .unwrap_or(true) // include on mtime failure (safe direction)
            })
            .collect();
        Self::parse_and_dedup(self, &recent)
    }

    /// Collect all files matching the glob pattern
    fn collect_files(&self) -> Vec<PathBuf> {
        let pattern = self.data_dir().join(self.file_pattern());
        glob::glob(&pattern.to_string_lossy())
            .map(|paths| paths.filter_map(|e| e.ok()).collect())
            .unwrap_or_default()
    }

    /// Parse files in parallel and deduplicate
    fn parse_and_dedup(&self, files: &[PathBuf]) -> Result<Vec<UsageEntry>> {
        let all_entries: Vec<UsageEntry> = files
            .par_iter()
            .flat_map(|f| match self.parse_file(f) {
                Ok(entries) => entries,
                Err(e) => {
                    eprintln!("[toktrack] Warning: Failed to parse {:?}: {}", f, e);
                    Vec::new()
                }
            })
            .collect();

        // Deduplicate by message_id:request_id (same as ccusage)
        let mut seen: HashSet<String> = HashSet::new();
        let mut deduped: Vec<UsageEntry> = Vec::with_capacity(all_entries.len());

        for entry in all_entries {
            if let Some(hash) = entry.dedup_hash() {
                if seen.insert(hash) {
                    deduped.push(entry);
                }
                // Skip duplicate (hash already in set)
            } else {
                // No hash (missing message_id or request_id) - keep entry
                deduped.push(entry);
            }
        }

        Ok(deduped)
    }
}

/// A concrete source of usage data backed by one parser.
///
/// The parser kind identifies the log format (for example, `codex`), while the
/// source id identifies one concrete location that uses that format (for
/// example, `codex` locally or `codex@devbox` for a future remote snapshot).
pub struct SourceInstance {
    pub id: String,
    pub label: String,
    pub kind: String,
    pub parser: Box<dyn CLIParser>,
}

impl SourceInstance {
    /// Create a source instance with explicit identity metadata.
    #[allow(dead_code)] // Used by tests and future remote-source registration.
    pub fn new(
        id: impl Into<String>,
        label: impl Into<String>,
        kind: impl Into<String>,
        parser: Box<dyn CLIParser>,
    ) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            kind: kind.into(),
            parser,
        }
    }

    /// Create the default local source for a parser.
    pub fn local(parser: Box<dyn CLIParser>) -> Self {
        let kind = parser.name().to_string();
        Self {
            id: kind.clone(),
            label: kind.clone(),
            kind,
            parser,
        }
    }
}

/// Registry of available usage sources
pub struct ParserRegistry {
    sources: Vec<SourceInstance>,
}

impl ParserRegistry {
    /// Create a new registry with default parsers
    pub fn new() -> Self {
        Self {
            sources: vec![
                SourceInstance::local(Box::new(ClaudeCodeParser::new())),
                SourceInstance::local(Box::new(CodexParser::new())),
                SourceInstance::local(Box::new(GeminiParser::new())),
                SourceInstance::local(Box::new(GeminiParser::new_qwen())),
                SourceInstance::local(Box::new(OpenCodeParser::new())),
                SourceInstance::local(Box::new(PiAgentParser::new())),
            ],
        }
    }

    /// Create a registry from explicit source instances.
    #[allow(dead_code)] // Used by tests and future remote-source configuration.
    pub fn with_sources(sources: Vec<SourceInstance>) -> Self {
        Self { sources }
    }

    /// Create the default registry and append extra source instances.
    pub fn with_extra_sources(mut extra_sources: Vec<SourceInstance>) -> Self {
        let mut registry = Self::new();
        registry.sources.append(&mut extra_sources);
        registry
    }

    /// Get all registered source instances
    pub fn sources(&self) -> &[SourceInstance] {
        &self.sources
    }

    /// Find a parser by source id.
    #[allow(dead_code)] // Used in tests and future features
    pub fn get(&self, id: &str) -> Option<&dyn CLIParser> {
        self.get_source(id).map(|source| source.parser.as_ref())
    }

    /// Find a source instance by id.
    #[allow(dead_code)] // Used in tests and future features
    pub fn get_source(&self, id: &str) -> Option<&SourceInstance> {
        self.sources.iter().find(|source| source.id == id)
    }
}

impl Default for ParserRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_registry_default_parsers() {
        let registry = ParserRegistry::new();
        assert_eq!(registry.sources().len(), 6);
        assert!(registry.get("claude-code").is_some());
        assert!(registry.get("codex").is_some());
        assert!(registry.get("gemini").is_some());
        assert!(registry.get("qwen").is_some());
        assert!(registry.get("opencode").is_some());
        assert!(registry.get("pi-agent").is_some());
    }

    #[test]
    fn test_local_source_uses_parser_name_as_identity() {
        let source = SourceInstance::local(Box::new(CodexParser::with_data_dir(PathBuf::from(
            "tests/fixtures/codex",
        ))));

        assert_eq!(source.id, "codex");
        assert_eq!(source.label, "codex");
        assert_eq!(source.kind, "codex");
        assert_eq!(source.parser.name(), "codex");
    }

    #[test]
    fn test_registry_accepts_multiple_sources_for_same_parser_kind() {
        let registry = ParserRegistry::with_sources(vec![
            SourceInstance::new(
                "codex",
                "codex",
                "codex",
                Box::new(CodexParser::with_data_dir(PathBuf::from(
                    "tests/fixtures/codex",
                ))),
            ),
            SourceInstance::new(
                "codex@testbox",
                "codex (testbox)",
                "codex",
                Box::new(CodexParser::with_data_dir(PathBuf::from(
                    "tests/fixtures/codex",
                ))),
            ),
        ]);

        assert_eq!(registry.sources().len(), 2);
        assert_eq!(registry.get("codex").unwrap().name(), "codex");
        assert_eq!(registry.get("codex@testbox").unwrap().name(), "codex");
        assert_eq!(
            registry.get_source("codex@testbox").unwrap().id,
            "codex@testbox"
        );
    }

    #[test]
    fn test_registry_appends_extra_sources_after_defaults() {
        let registry = ParserRegistry::with_extra_sources(vec![SourceInstance::new(
            "codex@testbox",
            "codex (testbox)",
            "codex",
            Box::new(CodexParser::with_data_dir(PathBuf::from(
                "tests/fixtures/codex",
            ))),
        )]);

        assert_eq!(registry.sources().len(), 7);
        assert_eq!(registry.sources()[6].id, "codex@testbox");
    }

    #[test]
    fn test_registry_get_unknown() {
        let registry = ParserRegistry::new();
        assert!(registry.get("unknown-parser").is_none());
    }

    #[test]
    fn test_parse_all_empty_directory() {
        let parser = ClaudeCodeParser::with_data_dir(PathBuf::from("tests/fixtures/nonexistent"));
        let result = parser.parse_all().unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn test_parse_all_fixtures_directory() {
        let parser = ClaudeCodeParser::with_data_dir(PathBuf::from("tests/fixtures"));
        let result = parser.parse_all().unwrap();
        assert!(!result.is_empty());
        // claude-sample.jsonl (4) + empty.jsonl (0) + multi/*.jsonl (2)
        //   + claude/real-shape-session.jsonl (1) = 7
        assert_eq!(result.len(), 7);
    }

    #[test]
    fn test_parse_all_multiple_files() {
        let parser = ClaudeCodeParser::with_data_dir(PathBuf::from("tests/fixtures/multi"));
        let result = parser.parse_all().unwrap();
        // 2 files × 1 entry each = 2 entries
        assert_eq!(result.len(), 2);
    }

    #[test]
    fn test_parse_all_with_empty_file() {
        // tests/fixtures has claude-sample.jsonl (3), empty.jsonl (0), multi/*.jsonl (2)
        let parser = ClaudeCodeParser::with_data_dir(PathBuf::from("tests/fixtures"));
        let result = parser.parse_all().unwrap();
        // empty.jsonl contributes 0 entries, total = 7
        assert_eq!(result.len(), 7);
    }

    #[test]
    fn test_parse_recent_files_includes_all_recent() {
        // All fixture files were modified recently (exist on disk now)
        // Using epoch as since → all files should be included
        let parser = ClaudeCodeParser::with_data_dir(PathBuf::from("tests/fixtures"));
        let since = std::time::UNIX_EPOCH;
        let result = parser.parse_recent_files(since).unwrap();
        // Same as parse_all: all files are "recent" relative to epoch
        assert_eq!(result.len(), 7);
    }

    #[test]
    fn test_parse_recent_files_filters_old() {
        // Using a future time as since → no files should match
        let parser = ClaudeCodeParser::with_data_dir(PathBuf::from("tests/fixtures"));
        let since = SystemTime::now() + std::time::Duration::from_secs(3600);
        let result = parser.parse_recent_files(since).unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn test_parse_recent_files_empty_directory() {
        let parser = ClaudeCodeParser::with_data_dir(PathBuf::from("tests/fixtures/nonexistent"));
        let since = std::time::UNIX_EPOCH;
        let result = parser.parse_recent_files(since).unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn test_collect_files() {
        let parser = ClaudeCodeParser::with_data_dir(PathBuf::from("tests/fixtures"));
        let files = parser.collect_files();
        // claude-sample.jsonl, empty.jsonl, multi/file1.jsonl, multi/file2.jsonl,
        // codex/sample-session.jsonl, codex/multi-turn-session.jsonl,
        // codex/openai-session.jsonl, codex/multi-session-meta.jsonl,
        // codex/cwd-session.jsonl,
        // pi_agent/sample-session.jsonl,
        // gemini/tmp_jsonl/chats/session-*.jsonl,
        // gemini/tmp_jsonl/chats/parent-session-xyz/sub-abc.jsonl,
        // gemini/tmp_jsonl_malformed/chats/session-bad.jsonl,
        // gemini/tmp_jsonl_no_meta/chats/session-*.jsonl
        // claude/real-shape-session.jsonl
        // qwen/proj/chats/session-*.jsonl
        // (claude parser uses `**/*.jsonl`; gemini/qwen-format files parse to 0 entries.)
        assert_eq!(files.len(), 16);
    }
}
