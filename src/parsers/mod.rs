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
        let pattern = self.data_dir().join(self.file_pattern());
        let files: Vec<PathBuf> = glob::glob(&pattern.to_string_lossy())
            .map(|paths| paths.filter_map(|e| e.ok()).collect())
            .unwrap_or_default();

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
}
