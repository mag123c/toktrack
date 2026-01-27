//! Usage types for token tracking
#![allow(dead_code)]

use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// A single usage entry from an AI CLI session
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct UsageEntry {
    /// Timestamp of the usage
    pub timestamp: DateTime<Utc>,

    /// Model name (e.g., "claude-sonnet-4-20250514")
    pub model: Option<String>,

    /// Input tokens consumed
    pub input_tokens: u64,

    /// Output tokens generated
    pub output_tokens: u64,

    /// Cache read tokens
    pub cache_read_tokens: u64,

    /// Cache creation tokens
    pub cache_creation_tokens: u64,

    /// Pre-calculated cost in USD (if available)
    pub cost_usd: Option<f64>,

    /// Message ID for deduplication
    pub message_id: Option<String>,

    /// Request ID for deduplication
    pub request_id: Option<String>,
}

impl UsageEntry {
    /// Total tokens (input + output + cache_read + cache_creation)
    /// This matches ccusage's calculation which includes all token types
    pub fn total_tokens(&self) -> u64 {
        self.input_tokens + self.output_tokens + self.cache_read_tokens + self.cache_creation_tokens
    }

    /// Create a unique hash for deduplication
    pub fn dedup_hash(&self) -> Option<String> {
        match (&self.message_id, &self.request_id) {
            (Some(msg), Some(req)) => Some(format!("{}:{}", msg, req)),
            _ => None,
        }
    }
}

/// Daily summary of usage
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DailySummary {
    pub date: NaiveDate,
    pub total_input_tokens: u64,
    pub total_output_tokens: u64,
    pub total_cache_read_tokens: u64,
    pub total_cache_creation_tokens: u64,
    pub total_cost_usd: f64,
    pub models: HashMap<String, ModelUsage>,
}

/// Per-model usage breakdown
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct ModelUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_creation_tokens: u64,
    pub cost_usd: f64,
    pub count: u64,
}

impl ModelUsage {
    pub fn add(&mut self, entry: &UsageEntry, cost: f64) {
        self.input_tokens = self.input_tokens.saturating_add(entry.input_tokens);
        self.output_tokens = self.output_tokens.saturating_add(entry.output_tokens);
        self.cache_read_tokens = self
            .cache_read_tokens
            .saturating_add(entry.cache_read_tokens);
        self.cache_creation_tokens = self
            .cache_creation_tokens
            .saturating_add(entry.cache_creation_tokens);
        self.cost_usd += cost;
        self.count = self.count.saturating_add(1);
    }
}

/// Total summary across all data
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct TotalSummary {
    pub total_input_tokens: u64,
    pub total_output_tokens: u64,
    pub total_cache_read_tokens: u64,
    pub total_cache_creation_tokens: u64,
    pub total_cost_usd: f64,
    pub entry_count: u64,
    pub day_count: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_usage_entry_total_tokens() {
        let entry = UsageEntry {
            timestamp: Utc::now(),
            model: Some("claude-sonnet-4".into()),
            input_tokens: 100,
            output_tokens: 50,
            cache_read_tokens: 20,
            cache_creation_tokens: 10,
            cost_usd: None,
            message_id: None,
            request_id: None,
        };
        // total = input + output + cache_read + cache_creation
        assert_eq!(entry.total_tokens(), 180);
    }

    #[test]
    fn test_usage_entry_dedup_hash() {
        let entry = UsageEntry {
            timestamp: Utc::now(),
            model: None,
            input_tokens: 0,
            output_tokens: 0,
            cache_read_tokens: 0,
            cache_creation_tokens: 0,
            cost_usd: None,
            message_id: Some("msg123".into()),
            request_id: Some("req456".into()),
        };
        assert_eq!(entry.dedup_hash(), Some("msg123:req456".into()));
    }

    #[test]
    fn test_usage_entry_dedup_hash_missing() {
        let entry = UsageEntry {
            timestamp: Utc::now(),
            model: None,
            input_tokens: 0,
            output_tokens: 0,
            cache_read_tokens: 0,
            cache_creation_tokens: 0,
            cost_usd: None,
            message_id: None,
            request_id: Some("req456".into()),
        };
        assert_eq!(entry.dedup_hash(), None);
    }

    #[test]
    fn test_model_usage_add() {
        let mut usage = ModelUsage::default();
        let entry = UsageEntry {
            timestamp: Utc::now(),
            model: Some("claude".into()),
            input_tokens: 100,
            output_tokens: 50,
            cache_read_tokens: 20,
            cache_creation_tokens: 10,
            cost_usd: None,
            message_id: None,
            request_id: None,
        };
        usage.add(&entry, 0.01);

        assert_eq!(usage.input_tokens, 100);
        assert_eq!(usage.output_tokens, 50);
        assert_eq!(usage.cache_read_tokens, 20);
        assert_eq!(usage.cost_usd, 0.01);
        assert_eq!(usage.count, 1);
    }
}
