//! Aggregator service for computing usage statistics

use crate::types::{DailySummary, ModelUsage, TotalSummary, UsageEntry};
use std::collections::{HashMap, HashSet};

/// Aggregator for computing usage statistics
pub struct Aggregator;

impl Aggregator {
    /// Aggregate entries by day (sorted by date ascending)
    pub fn daily(entries: &[UsageEntry]) -> Vec<DailySummary> {
        if entries.is_empty() {
            return Vec::new();
        }

        // Group by date
        let mut daily_map: HashMap<chrono::NaiveDate, DailySummary> = HashMap::new();

        for entry in entries {
            let date = entry.timestamp.date_naive();
            let cost = entry.cost_usd.unwrap_or(0.0);
            let model_name = entry.model.as_deref().unwrap_or("unknown").to_string();

            let summary = daily_map.entry(date).or_insert_with(|| DailySummary {
                date,
                total_input_tokens: 0,
                total_output_tokens: 0,
                total_cache_read_tokens: 0,
                total_cache_creation_tokens: 0,
                total_cost_usd: 0.0,
                models: HashMap::new(),
            });

            summary.total_input_tokens = summary
                .total_input_tokens
                .saturating_add(entry.input_tokens);
            summary.total_output_tokens = summary
                .total_output_tokens
                .saturating_add(entry.output_tokens);
            summary.total_cache_read_tokens = summary
                .total_cache_read_tokens
                .saturating_add(entry.cache_read_tokens);
            summary.total_cache_creation_tokens = summary
                .total_cache_creation_tokens
                .saturating_add(entry.cache_creation_tokens);
            summary.total_cost_usd += cost;

            // Update model breakdown
            let model_usage = summary.models.entry(model_name).or_default();
            model_usage.add(entry, cost);
        }

        // Sort by date ascending
        let mut result: Vec<DailySummary> = daily_map.into_values().collect();
        result.sort_by_key(|s| s.date);
        result
    }

    /// Aggregate entries by model (None → "unknown")
    pub fn by_model(entries: &[UsageEntry]) -> HashMap<String, ModelUsage> {
        let mut model_map: HashMap<String, ModelUsage> = HashMap::new();

        for entry in entries {
            let model_name = entry.model.as_deref().unwrap_or("unknown").to_string();
            let cost = entry.cost_usd.unwrap_or(0.0);

            let usage = model_map.entry(model_name).or_default();
            usage.add(entry, cost);
        }

        model_map
    }

    /// Compute total summary across all entries
    pub fn total(entries: &[UsageEntry]) -> TotalSummary {
        if entries.is_empty() {
            return TotalSummary::default();
        }

        let mut dates: HashSet<chrono::NaiveDate> = HashSet::new();
        let mut summary = TotalSummary::default();

        for entry in entries {
            summary.total_input_tokens = summary
                .total_input_tokens
                .saturating_add(entry.input_tokens);
            summary.total_output_tokens = summary
                .total_output_tokens
                .saturating_add(entry.output_tokens);
            summary.total_cache_read_tokens = summary
                .total_cache_read_tokens
                .saturating_add(entry.cache_read_tokens);
            summary.total_cache_creation_tokens = summary
                .total_cache_creation_tokens
                .saturating_add(entry.cache_creation_tokens);
            summary.total_cost_usd += entry.cost_usd.unwrap_or(0.0);
            summary.entry_count = summary.entry_count.saturating_add(1);

            dates.insert(entry.timestamp.date_naive());
        }

        summary.day_count = dates.len() as u64;
        summary
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Utc};

    fn make_entry(
        year: i32,
        month: u32,
        day: u32,
        model: Option<&str>,
        input: u64,
        output: u64,
        cost: Option<f64>,
    ) -> UsageEntry {
        UsageEntry {
            timestamp: Utc.with_ymd_and_hms(year, month, day, 12, 0, 0).unwrap(),
            model: model.map(String::from),
            input_tokens: input,
            output_tokens: output,
            cache_read_tokens: 0,
            cache_creation_tokens: 0,
            thinking_tokens: 0,
            cost_usd: cost,
            message_id: None,
            request_id: None,
            source: None,
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn make_entry_full(
        year: i32,
        month: u32,
        day: u32,
        model: Option<&str>,
        input: u64,
        output: u64,
        cache_read: u64,
        cache_creation: u64,
        cost: Option<f64>,
    ) -> UsageEntry {
        UsageEntry {
            timestamp: Utc.with_ymd_and_hms(year, month, day, 12, 0, 0).unwrap(),
            model: model.map(String::from),
            input_tokens: input,
            output_tokens: output,
            cache_read_tokens: cache_read,
            cache_creation_tokens: cache_creation,
            thinking_tokens: 0,
            cost_usd: cost,
            message_id: None,
            request_id: None,
            source: None,
        }
    }

    // ========== daily() tests ==========

    #[test]
    fn test_daily_empty_entries() {
        let result = Aggregator::daily(&[]);
        assert!(result.is_empty());
    }

    #[test]
    fn test_daily_single_entry() {
        let entries = vec![make_entry(
            2024,
            1,
            15,
            Some("claude-sonnet"),
            100,
            50,
            Some(0.01),
        )];

        let result = Aggregator::daily(&entries);

        assert_eq!(result.len(), 1);
        assert_eq!(result[0].date.to_string(), "2024-01-15");
        assert_eq!(result[0].total_input_tokens, 100);
        assert_eq!(result[0].total_output_tokens, 50);
        assert!((result[0].total_cost_usd - 0.01).abs() < f64::EPSILON);
    }

    #[test]
    fn test_daily_multiple_days_sorted_ascending() {
        let entries = vec![
            make_entry(2024, 1, 20, Some("claude"), 100, 50, Some(0.01)),
            make_entry(2024, 1, 10, Some("claude"), 200, 100, Some(0.02)),
            make_entry(2024, 1, 15, Some("claude"), 150, 75, Some(0.015)),
        ];

        let result = Aggregator::daily(&entries);

        assert_eq!(result.len(), 3);
        // Should be sorted ascending by date
        assert_eq!(result[0].date.to_string(), "2024-01-10");
        assert_eq!(result[1].date.to_string(), "2024-01-15");
        assert_eq!(result[2].date.to_string(), "2024-01-20");
    }

    #[test]
    fn test_daily_same_day_aggregation() {
        let entries = vec![
            make_entry_full(2024, 1, 15, Some("claude"), 100, 50, 10, 5, Some(0.01)),
            make_entry_full(2024, 1, 15, Some("gpt-4"), 200, 100, 20, 10, Some(0.02)),
        ];

        let result = Aggregator::daily(&entries);

        assert_eq!(result.len(), 1);
        assert_eq!(result[0].total_input_tokens, 300);
        assert_eq!(result[0].total_output_tokens, 150);
        assert_eq!(result[0].total_cache_read_tokens, 30);
        assert_eq!(result[0].total_cache_creation_tokens, 15);
        assert!((result[0].total_cost_usd - 0.03).abs() < f64::EPSILON);
        // Should have 2 models in the breakdown
        assert_eq!(result[0].models.len(), 2);
    }

    // ========== by_model() tests ==========

    #[test]
    fn test_by_model_empty() {
        let result = Aggregator::by_model(&[]);
        assert!(result.is_empty());
    }

    #[test]
    fn test_by_model_single_model() {
        let entries = vec![make_entry_full(
            2024,
            1,
            15,
            Some("claude-sonnet"),
            100,
            50,
            10,
            5,
            Some(0.01),
        )];

        let result = Aggregator::by_model(&entries);

        assert_eq!(result.len(), 1);
        let usage = result.get("claude-sonnet").unwrap();
        assert_eq!(usage.input_tokens, 100);
        assert_eq!(usage.output_tokens, 50);
        assert_eq!(usage.cache_read_tokens, 10);
        assert_eq!(usage.cache_creation_tokens, 5);
        assert!((usage.cost_usd - 0.01).abs() < f64::EPSILON);
        assert_eq!(usage.count, 1);
    }

    #[test]
    fn test_by_model_none_model_becomes_unknown() {
        let entries = vec![make_entry(2024, 1, 15, None, 100, 50, Some(0.01))];

        let result = Aggregator::by_model(&entries);

        assert_eq!(result.len(), 1);
        assert!(result.contains_key("unknown"));
        let usage = result.get("unknown").unwrap();
        assert_eq!(usage.input_tokens, 100);
    }

    #[test]
    fn test_by_model_multiple_models() {
        let entries = vec![
            make_entry_full(2024, 1, 15, Some("claude"), 100, 50, 10, 5, Some(0.01)),
            make_entry_full(2024, 1, 16, Some("claude"), 200, 100, 20, 10, Some(0.02)),
            make_entry_full(2024, 1, 15, Some("gpt-4"), 300, 150, 30, 15, Some(0.03)),
        ];

        let result = Aggregator::by_model(&entries);

        assert_eq!(result.len(), 2);

        let claude = result.get("claude").unwrap();
        assert_eq!(claude.input_tokens, 300); // 100 + 200
        assert_eq!(claude.output_tokens, 150); // 50 + 100
        assert_eq!(claude.count, 2);

        let gpt = result.get("gpt-4").unwrap();
        assert_eq!(gpt.input_tokens, 300);
        assert_eq!(gpt.output_tokens, 150);
        assert_eq!(gpt.count, 1);
    }

    // ========== total() tests ==========

    #[test]
    fn test_total_empty() {
        let result = Aggregator::total(&[]);

        assert_eq!(result.total_input_tokens, 0);
        assert_eq!(result.total_output_tokens, 0);
        assert_eq!(result.total_cache_read_tokens, 0);
        assert_eq!(result.total_cache_creation_tokens, 0);
        assert!((result.total_cost_usd - 0.0).abs() < f64::EPSILON);
        assert_eq!(result.entry_count, 0);
        assert_eq!(result.day_count, 0);
    }

    #[test]
    fn test_total_single() {
        let entries = vec![make_entry_full(
            2024,
            1,
            15,
            Some("claude"),
            100,
            50,
            10,
            5,
            Some(0.01),
        )];

        let result = Aggregator::total(&entries);

        assert_eq!(result.total_input_tokens, 100);
        assert_eq!(result.total_output_tokens, 50);
        assert_eq!(result.total_cache_read_tokens, 10);
        assert_eq!(result.total_cache_creation_tokens, 5);
        assert!((result.total_cost_usd - 0.01).abs() < f64::EPSILON);
        assert_eq!(result.entry_count, 1);
        assert_eq!(result.day_count, 1);
    }

    #[test]
    fn test_total_multiple() {
        let entries = vec![
            make_entry_full(2024, 1, 15, Some("claude"), 100, 50, 10, 5, Some(0.01)),
            make_entry_full(2024, 1, 15, Some("gpt-4"), 200, 100, 20, 10, Some(0.02)),
            make_entry_full(2024, 1, 16, Some("claude"), 300, 150, 30, 15, Some(0.03)),
        ];

        let result = Aggregator::total(&entries);

        assert_eq!(result.total_input_tokens, 600); // 100 + 200 + 300
        assert_eq!(result.total_output_tokens, 300); // 50 + 100 + 150
        assert_eq!(result.total_cache_read_tokens, 60); // 10 + 20 + 30
        assert_eq!(result.total_cache_creation_tokens, 30); // 5 + 10 + 15
        assert!((result.total_cost_usd - 0.06).abs() < f64::EPSILON);
        assert_eq!(result.entry_count, 3);
        assert_eq!(result.day_count, 2); // 2 distinct days
    }

    #[test]
    fn test_total_with_none_cost() {
        let entries = vec![
            make_entry(2024, 1, 15, Some("claude"), 100, 50, Some(0.01)),
            make_entry(2024, 1, 15, Some("claude"), 100, 50, None), // No cost
        ];

        let result = Aggregator::total(&entries);

        // None cost should be treated as 0.0
        assert!((result.total_cost_usd - 0.01).abs() < f64::EPSILON);
    }
}
