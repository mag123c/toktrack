//! Usage types for token tracking

use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Data for the stats view (CLI and TUI)
#[derive(Debug, Clone, Serialize)]
pub struct StatsData {
    /// Total tokens across all days
    pub total_tokens: u64,
    /// Daily average tokens
    pub daily_avg_tokens: u64,
    /// Peak day (date, tokens)
    pub peak_day: Option<(NaiveDate, u64)>,
    /// Total cost in USD
    pub total_cost: f64,
    /// Daily average cost
    pub daily_avg_cost: f64,
    /// Number of active days
    pub active_days: u32,
}

impl StatsData {
    /// Create StatsData from daily summaries
    pub fn from_daily_summaries(summaries: &[DailySummary]) -> Self {
        if summaries.is_empty() {
            return Self {
                total_tokens: 0,
                daily_avg_tokens: 0,
                peak_day: None,
                total_cost: 0.0,
                daily_avg_cost: 0.0,
                active_days: 0,
            };
        }

        let active_days = summaries.len() as u32;

        // Calculate totals
        let mut total_tokens: u64 = 0;
        let mut total_cost: f64 = 0.0;
        let mut peak_day: Option<(NaiveDate, u64)> = None;

        for summary in summaries {
            let day_tokens = summary.total_input_tokens
                + summary.total_output_tokens
                + summary.total_cache_read_tokens
                + summary.total_cache_creation_tokens;

            total_tokens += day_tokens;
            total_cost += summary.total_cost_usd;

            // Track peak day
            match &peak_day {
                None => peak_day = Some((summary.date, day_tokens)),
                Some((_, max_tokens)) if day_tokens > *max_tokens => {
                    peak_day = Some((summary.date, day_tokens));
                }
                _ => {}
            }
        }

        let daily_avg_tokens = total_tokens / active_days as u64;
        let daily_avg_cost = total_cost / active_days as f64;

        Self {
            total_tokens,
            daily_avg_tokens,
            peak_day,
            total_cost,
            daily_avg_cost,
            active_days,
        }
    }
}

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

    /// Thinking tokens (for models with extended thinking, e.g., Gemini)
    #[serde(default)]
    pub thinking_tokens: u64,

    /// Pre-calculated cost in USD (if available)
    pub cost_usd: Option<f64>,

    /// Message ID for deduplication
    pub message_id: Option<String>,

    /// Request ID for deduplication
    pub request_id: Option<String>,

    /// Parser source identifier (e.g., "claude", "codex", "gemini")
    #[serde(default)]
    pub source: Option<String>,
}

impl UsageEntry {
    /// Total tokens (input + output + cache_read + cache_creation + thinking)
    /// This matches ccusage's calculation which includes all token types
    #[allow(dead_code)] // Part of public API
    pub fn total_tokens(&self) -> u64 {
        self.input_tokens
            + self.output_tokens
            + self.cache_read_tokens
            + self.cache_creation_tokens
            + self.thinking_tokens
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

    #[allow(clippy::too_many_arguments)]
    fn make_summary(
        year: i32,
        month: u32,
        day: u32,
        input: u64,
        output: u64,
        cache_read: u64,
        cache_creation: u64,
        cost: f64,
    ) -> DailySummary {
        DailySummary {
            date: NaiveDate::from_ymd_opt(year, month, day).unwrap(),
            total_input_tokens: input,
            total_output_tokens: output,
            total_cache_read_tokens: cache_read,
            total_cache_creation_tokens: cache_creation,
            total_cost_usd: cost,
            models: HashMap::new(),
        }
    }

    #[test]
    fn test_stats_data_empty() {
        let data = StatsData::from_daily_summaries(&[]);

        assert_eq!(data.total_tokens, 0);
        assert_eq!(data.daily_avg_tokens, 0);
        assert!(data.peak_day.is_none());
        assert!((data.total_cost - 0.0).abs() < f64::EPSILON);
        assert!((data.daily_avg_cost - 0.0).abs() < f64::EPSILON);
        assert_eq!(data.active_days, 0);
    }

    #[test]
    fn test_stats_data_single_day() {
        let summaries = vec![make_summary(2024, 1, 15, 1000, 500, 100, 50, 0.10)];
        let data = StatsData::from_daily_summaries(&summaries);

        assert_eq!(data.total_tokens, 1650); // 1000 + 500 + 100 + 50
        assert_eq!(data.daily_avg_tokens, 1650);
        assert_eq!(
            data.peak_day,
            Some((NaiveDate::from_ymd_opt(2024, 1, 15).unwrap(), 1650))
        );
        assert!((data.total_cost - 0.10).abs() < f64::EPSILON);
        assert!((data.daily_avg_cost - 0.10).abs() < f64::EPSILON);
        assert_eq!(data.active_days, 1);
    }

    #[test]
    fn test_stats_data_multiple_days() {
        let summaries = vec![
            make_summary(2024, 1, 10, 100, 50, 10, 5, 0.05), // 165 tokens
            make_summary(2024, 1, 15, 500, 250, 50, 25, 0.20), // 825 tokens (peak)
            make_summary(2024, 1, 20, 200, 100, 20, 10, 0.10), // 330 tokens
        ];
        let data = StatsData::from_daily_summaries(&summaries);

        assert_eq!(data.total_tokens, 165 + 825 + 330); // 1320
        assert_eq!(data.daily_avg_tokens, 1320 / 3); // 440
        assert_eq!(
            data.peak_day,
            Some((NaiveDate::from_ymd_opt(2024, 1, 15).unwrap(), 825))
        );
        assert!((data.total_cost - 0.35).abs() < f64::EPSILON);
        assert!((data.daily_avg_cost - 0.35 / 3.0).abs() < 0.001);
        assert_eq!(data.active_days, 3);
    }

    #[test]
    fn test_stats_data_peak_day_tie_keeps_first() {
        // When multiple days have the same max tokens, first one wins
        let summaries = vec![
            make_summary(2024, 1, 10, 500, 250, 50, 25, 0.10), // 825 tokens (first peak)
            make_summary(2024, 1, 15, 500, 250, 50, 25, 0.10), // 825 tokens (tie)
            make_summary(2024, 1, 20, 100, 50, 10, 5, 0.05),   // 165 tokens
        ];
        let data = StatsData::from_daily_summaries(&summaries);

        // First day with max should win
        assert_eq!(
            data.peak_day,
            Some((NaiveDate::from_ymd_opt(2024, 1, 10).unwrap(), 825))
        );
    }

    #[test]
    fn test_usage_entry_total_tokens() {
        let entry = UsageEntry {
            timestamp: Utc::now(),
            model: Some("claude-sonnet-4".into()),
            input_tokens: 100,
            output_tokens: 50,
            cache_read_tokens: 20,
            cache_creation_tokens: 10,
            thinking_tokens: 0,
            cost_usd: None,
            message_id: None,
            request_id: None,
            source: None,
        };
        // total = input + output + cache_read + cache_creation + thinking
        assert_eq!(entry.total_tokens(), 180);
    }

    #[test]
    fn test_usage_entry_total_tokens_with_thinking() {
        let entry = UsageEntry {
            timestamp: Utc::now(),
            model: Some("gemini-2.5-pro".into()),
            input_tokens: 100,
            output_tokens: 50,
            cache_read_tokens: 20,
            cache_creation_tokens: 10,
            thinking_tokens: 30,
            cost_usd: None,
            message_id: None,
            request_id: None,
            source: Some("gemini".into()),
        };
        // total = 100 + 50 + 20 + 10 + 30 = 210
        assert_eq!(entry.total_tokens(), 210);
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
            thinking_tokens: 0,
            cost_usd: None,
            message_id: Some("msg123".into()),
            request_id: Some("req456".into()),
            source: None,
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
            thinking_tokens: 0,
            cost_usd: None,
            message_id: None,
            request_id: Some("req456".into()),
            source: None,
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
            thinking_tokens: 0,
            cost_usd: None,
            message_id: None,
            request_id: None,
            source: None,
        };
        usage.add(&entry, 0.01);

        assert_eq!(usage.input_tokens, 100);
        assert_eq!(usage.output_tokens, 50);
        assert_eq!(usage.cache_read_tokens, 20);
        assert_eq!(usage.cost_usd, 0.01);
        assert_eq!(usage.count, 1);
    }
}
