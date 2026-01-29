//! Parser traits and implementations for AI CLI tools

mod claude;
mod codex;
mod gemini;

pub use claude::ClaudeCodeParser;
pub use codex::CodexParser;
pub use gemini::GeminiParser;

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
    fn data_dir(&self) -> PathBuf;

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

/// Registry of available parsers
pub struct ParserRegistry {
    parsers: Vec<Box<dyn CLIParser>>,
}

impl ParserRegistry {
    /// Create a new registry with default parsers
    pub fn new() -> Self {
        Self {
            parsers: vec![
                Box::new(ClaudeCodeParser::new()),
                Box::new(CodexParser::new()),
                Box::new(GeminiParser::new()),
            ],
        }
    }

    /// Get all registered parsers
    pub fn parsers(&self) -> &[Box<dyn CLIParser>] {
        &self.parsers
    }

    /// Find a parser by name
    #[allow(dead_code)] // Used in tests and future features
    pub fn get(&self, name: &str) -> Option<&dyn CLIParser> {
        self.parsers
            .iter()
            .find(|p| p.name() == name)
            .map(|p| p.as_ref())
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
        assert_eq!(registry.parsers().len(), 3);
        assert!(registry.get("claude-code").is_some());
        assert!(registry.get("codex").is_some());
        assert!(registry.get("gemini").is_some());
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
        // claude-sample.jsonl (3) + empty.jsonl (0) + multi/*.jsonl (2) = 5
        assert_eq!(result.len(), 5);
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
        // empty.jsonl contributes 0 entries, total = 5
        assert_eq!(result.len(), 5);
    }

    #[test]
    fn test_parse_recent_files_includes_all_recent() {
        // All fixture files were modified recently (exist on disk now)
        // Using epoch as since → all files should be included
        let parser = ClaudeCodeParser::with_data_dir(PathBuf::from("tests/fixtures"));
        let since = std::time::UNIX_EPOCH;
        let result = parser.parse_recent_files(since).unwrap();
        // Same as parse_all: all files are "recent" relative to epoch
        assert_eq!(result.len(), 5);
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
        // claude-sample.jsonl, empty.jsonl, multi/file1.jsonl, multi/file2.jsonl, codex/sample-session.jsonl
        assert_eq!(files.len(), 5);
    }
}
