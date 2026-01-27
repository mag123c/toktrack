//! Parser traits and implementations for AI CLI tools
#![allow(dead_code)]

mod claude;

pub use claude::ClaudeCodeParser;

use crate::types::{Result, UsageEntry};
use rayon::prelude::*;
use std::path::{Path, PathBuf};

/// Trait for parsing usage data from AI CLI tools
pub trait CLIParser: Send + Sync {
    /// Parser name (e.g., "claude-code")
    fn name(&self) -> &str;

    /// Data directory to scan for usage files
    fn data_dir(&self) -> PathBuf;

    /// Glob pattern for finding usage files (e.g., "**/*.jsonl")
    fn file_pattern(&self) -> &str;

    /// Parse a single file and return usage entries
    fn parse_file(&self, path: &Path) -> Result<Vec<UsageEntry>>;

    /// Parse all files in parallel using rayon
    fn parse_all(&self) -> Result<Vec<UsageEntry>> {
        let pattern = self.data_dir().join(self.file_pattern());
        let files: Vec<PathBuf> = glob::glob(&pattern.to_string_lossy())
            .map(|paths| paths.filter_map(|e| e.ok()).collect())
            .unwrap_or_default();

        Ok(files
            .par_iter()
            .flat_map(|f| self.parse_file(f).unwrap_or_default())
            .collect())
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
            parsers: vec![Box::new(ClaudeCodeParser::new())],
        }
    }

    /// Get all registered parsers
    pub fn parsers(&self) -> &[Box<dyn CLIParser>] {
        &self.parsers
    }

    /// Find a parser by name
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
        assert!(!registry.parsers().is_empty());
        assert!(registry.get("claude-code").is_some());
    }

    #[test]
    fn test_registry_get_unknown() {
        let registry = ParserRegistry::new();
        assert!(registry.get("unknown-parser").is_none());
    }
}
