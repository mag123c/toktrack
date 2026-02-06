//! Aggregator service for computing usage statistics

use super::normalize_model_name;
use crate::types::{DailySummary, ModelUsage, SourceUsage, TotalSummary, UsageEntry};
use chrono::Datelike;
use std::collections::{HashMap, HashSet};

pub struct Aggregator;

/// Accumulate token fields and cost from `source` into `target`
fn accumulate_summary(target: &mut DailySummary, source: &DailySummary) {
    target.total_input_tokens = target
        .total_input_tokens
        .saturating_add(source.total_input_tokens);
    target.total_output_tokens = target
        .total_output_tokens
        .saturating_add(source.total_output_tokens);
    target.total_cache_read_tokens = target
        .total_cache_read_tokens
        .saturating_add(source.total_cache_read_tokens);
    target.total_cache_creation_tokens = target
        .total_cache_creation_tokens
        .saturating_add(source.total_cache_creation_tokens);
    target.total_thinking_tokens = target
        .total_thinking_tokens
        .saturating_add(source.total_thinking_tokens);
    target.total_cost_usd += source.total_cost_usd;

    for (model_name, model_usage) in &source.models {
        let t = target.models.entry(model_name.clone()).or_default();
        merge_model_usage(t, model_usage);
    }
}

/// Merge model usage fields from `source` into `target`
fn merge_model_usage(target: &mut ModelUsage, source: &ModelUsage) {
    target.input_tokens = target.input_tokens.saturating_add(source.input_tokens);
    target.output_tokens = target.output_tokens.saturating_add(source.output_tokens);
    target.cache_read_tokens = target
        .cache_read_tokens
        .saturating_add(source.cache_read_tokens);
    target.cache_creation_tokens = target
        .cache_creation_tokens
        .saturating_add(source.cache_creation_tokens);
    target.thinking_tokens = target
        .thinking_tokens
        .saturating_add(source.thinking_tokens);
    target.cost_usd += source.cost_usd;
    target.count = target.count.saturating_add(source.count);
}

impl Aggregator {
    pub fn daily(entries: &[UsageEntry]) -> Vec<DailySummary> {
        if entries.is_empty() {
            return Vec::new();
        }

        // Group by date
        let mut daily_map: HashMap<chrono::NaiveDate, DailySummary> = HashMap::new();

        for entry in entries {
            let date = entry.timestamp.date_naive();
            let cost = entry.cost_usd.unwrap_or(0.0);
            let model_name = normalize_model_name(entry.model.as_deref().unwrap_or("unknown"));

            let summary = daily_map.entry(date).or_insert_with(|| DailySummary {
                date,
                total_input_tokens: 0,
                total_output_tokens: 0,
                total_cache_read_tokens: 0,
                total_cache_creation_tokens: 0,
                total_thinking_tokens: 0,
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
            summary.total_thinking_tokens = summary
                .total_thinking_tokens
                .saturating_add(entry.thinking_tokens);
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

    /// Aggregate daily summaries into weekly summaries (Sunday-start weeks)
    pub fn weekly(daily_summaries: &[DailySummary]) -> Vec<DailySummary> {
        if daily_summaries.is_empty() {
            return Vec::new();
        }

        let mut week_map: HashMap<chrono::NaiveDate, DailySummary> = HashMap::new();

        for summary in daily_summaries {
            // Calculate the Sunday that starts this week
            let days_from_sunday = summary.date.weekday().num_days_from_sunday();
            let week_start = summary
                .date
                .checked_sub_signed(chrono::Duration::days(days_from_sunday as i64))
                .unwrap_or(summary.date);

            let week_summary = week_map.entry(week_start).or_insert_with(|| DailySummary {
                date: week_start,
                total_input_tokens: 0,
                total_output_tokens: 0,
                total_cache_read_tokens: 0,
                total_cache_creation_tokens: 0,
                total_thinking_tokens: 0,
                total_cost_usd: 0.0,
                models: HashMap::new(),
            });

            accumulate_summary(week_summary, summary);
        }

        let mut result: Vec<DailySummary> = week_map.into_values().collect();
        result.sort_by_key(|s| s.date);
        result
    }

    /// Aggregate daily summaries into monthly summaries (calendar months)
    pub fn monthly(daily_summaries: &[DailySummary]) -> Vec<DailySummary> {
        if daily_summaries.is_empty() {
            return Vec::new();
        }

        let mut month_map: HashMap<(i32, u32), DailySummary> = HashMap::new();

        for summary in daily_summaries {
            let key = (summary.date.year(), summary.date.month());
            let month_start =
                chrono::NaiveDate::from_ymd_opt(key.0, key.1, 1).unwrap_or(summary.date);

            let month_summary = month_map.entry(key).or_insert_with(|| DailySummary {
                date: month_start,
                total_input_tokens: 0,
                total_output_tokens: 0,
                total_cache_read_tokens: 0,
                total_cache_creation_tokens: 0,
                total_thinking_tokens: 0,
                total_cost_usd: 0.0,
                models: HashMap::new(),
            });

            accumulate_summary(month_summary, summary);
        }

        let mut result: Vec<DailySummary> = month_map.into_values().collect();
        result.sort_by_key(|s| s.date);
        result
    }

    #[allow(dead_code)]
    pub fn by_model(entries: &[UsageEntry]) -> HashMap<String, ModelUsage> {
        let mut model_map: HashMap<String, ModelUsage> = HashMap::new();

        for entry in entries {
            let model_name = normalize_model_name(entry.model.as_deref().unwrap_or("unknown"));
            let cost = entry.cost_usd.unwrap_or(0.0);

            let usage = model_map.entry(model_name).or_default();
            usage.add(entry, cost);
        }

        model_map
    }

    /// Compute TotalSummary from DailySummary slice (no raw entries needed)
    pub fn total_from_daily(summaries: &[DailySummary]) -> TotalSummary {
        if summaries.is_empty() {
            return TotalSummary::default();
        }

        let mut summary = TotalSummary::default();
        for s in summaries {
            summary.total_input_tokens = summary
                .total_input_tokens
                .saturating_add(s.total_input_tokens);
            summary.total_output_tokens = summary
                .total_output_tokens
                .saturating_add(s.total_output_tokens);
            summary.total_cache_read_tokens = summary
                .total_cache_read_tokens
                .saturating_add(s.total_cache_read_tokens);
            summary.total_cache_creation_tokens = summary
                .total_cache_creation_tokens
                .saturating_add(s.total_cache_creation_tokens);
            summary.total_thinking_tokens = summary
                .total_thinking_tokens
                .saturating_add(s.total_thinking_tokens);
            summary.total_cost_usd += s.total_cost_usd;

            // entry_count = sum of per-model counts across all daily summaries
            for model_usage in s.models.values() {
                summary.entry_count = summary.entry_count.saturating_add(model_usage.count);
            }
        }

        summary.day_count = summaries.len() as u64;
        summary
    }

    /// Compute model breakdown from DailySummary slice (no raw entries needed)
    pub fn by_model_from_daily(summaries: &[DailySummary]) -> HashMap<String, ModelUsage> {
        let mut model_map: HashMap<String, ModelUsage> = HashMap::new();

        for s in summaries {
            for (model_name, usage) in &s.models {
                let target = model_map.entry(model_name.clone()).or_default();
                merge_model_usage(target, usage);
            }
        }

        model_map
    }

    #[allow(dead_code)]
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
            summary.total_thinking_tokens = summary
                .total_thinking_tokens
                .saturating_add(entry.thinking_tokens);
            summary.total_cost_usd += entry.cost_usd.unwrap_or(0.0);
            summary.entry_count = summary.entry_count.saturating_add(1);

            dates.insert(entry.timestamp.date_naive());
        }

        summary.day_count = dates.len() as u64;
        summary
    }

    /// Aggregate usage by source CLI (claude, opencode, gemini, etc.)
    #[allow(dead_code)]
    pub fn by_source(entries: &[UsageEntry]) -> Vec<SourceUsage> {
        let mut source_map: HashMap<String, (u64, f64)> = HashMap::new();

        for entry in entries {
            let source = entry.source.as_deref().unwrap_or("unknown").to_string();
            let total_tokens = entry.input_tokens
                + entry.output_tokens
                + entry.cache_read_tokens
                + entry.cache_creation_tokens
                + entry.thinking_tokens;
            let cost = entry.cost_usd.unwrap_or(0.0);

            let entry_stats = source_map.entry(source).or_insert((0, 0.0));
            entry_stats.0 = entry_stats.0.saturating_add(total_tokens);
            entry_stats.1 += cost;
        }

        let mut result: Vec<SourceUsage> = source_map
            .into_iter()
            .map(|(source, (total_tokens, total_cost_usd))| SourceUsage {
                source,
                total_tokens,
                total_cost_usd,
            })
            .collect();

        // Sort by total_tokens descending
        result.sort_by(|a, b| b.total_tokens.cmp(&a.total_tokens));
        result
    }

    /// Merge DailySummaries with the same date.
    /// Useful when combining summaries from multiple CLI sources.
    pub fn merge_by_date(summaries: Vec<DailySummary>) -> Vec<DailySummary> {
        if summaries.is_empty() {
            return Vec::new();
        }

        let mut date_map: HashMap<chrono::NaiveDate, DailySummary> = HashMap::new();

        for summary in summaries {
            let target = date_map
                .entry(summary.date)
                .or_insert_with(|| DailySummary {
                    date: summary.date,
                    total_input_tokens: 0,
                    total_output_tokens: 0,
                    total_cache_read_tokens: 0,
                    total_cache_creation_tokens: 0,
                    total_thinking_tokens: 0,
                    total_cost_usd: 0.0,
                    models: HashMap::new(),
                });
            accumulate_summary(target, &summary);
        }

        let mut result: Vec<DailySummary> = date_map.into_values().collect();
        result.sort_by_key(|s| s.date);
        result
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
            provider: None,
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
            provider: None,
        }
    }

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

    #[test]
    fn test_by_model_normalizes_date_suffix() {
        // claude-sonnet-4-20250514 and claude-sonnet-4 should be grouped together
        let entries = vec![
            make_entry(
                2024,
                1,
                15,
                Some("claude-sonnet-4-20250514"),
                100,
                50,
                Some(0.01),
            ),
            make_entry(2024, 1, 16, Some("claude-sonnet-4"), 200, 100, Some(0.02)),
        ];

        let result = Aggregator::by_model(&entries);

        // Should have only one model: claude-sonnet-4
        assert_eq!(result.len(), 1);
        let usage = result.get("claude-sonnet-4").unwrap();
        assert_eq!(usage.input_tokens, 300); // 100 + 200
        assert_eq!(usage.count, 2);
    }

    #[test]
    fn test_daily_normalizes_model_names() {
        let entries = vec![
            make_entry(
                2024,
                1,
                15,
                Some("claude-opus-4-5-20251101"),
                100,
                50,
                Some(0.01),
            ),
            make_entry(2024, 1, 15, Some("claude-opus-4-5"), 200, 100, Some(0.02)),
        ];

        let result = Aggregator::daily(&entries);

        assert_eq!(result.len(), 1);
        // Should have only one model in the breakdown
        assert_eq!(result[0].models.len(), 1);
        assert!(result[0].models.contains_key("claude-opus-4-5"));
    }

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

    // ========== Weekly aggregation tests ==========

    fn make_daily_summary(
        year: i32,
        month: u32,
        day: u32,
        input: u64,
        output: u64,
        cost: f64,
    ) -> DailySummary {
        DailySummary {
            date: chrono::NaiveDate::from_ymd_opt(year, month, day).unwrap(),
            total_input_tokens: input,
            total_output_tokens: output,
            total_cache_read_tokens: 0,
            total_cache_creation_tokens: 0,
            total_thinking_tokens: 0,
            total_cost_usd: cost,
            models: HashMap::new(),
        }
    }

    fn make_daily_summary_with_models(
        year: i32,
        month: u32,
        day: u32,
        input: u64,
        output: u64,
        cost: f64,
        models: HashMap<String, ModelUsage>,
    ) -> DailySummary {
        DailySummary {
            date: chrono::NaiveDate::from_ymd_opt(year, month, day).unwrap(),
            total_input_tokens: input,
            total_output_tokens: output,
            total_cache_read_tokens: 0,
            total_cache_creation_tokens: 0,
            total_thinking_tokens: 0,
            total_cost_usd: cost,
            models,
        }
    }

    #[test]
    fn test_weekly_empty() {
        let result = Aggregator::weekly(&[]);
        assert!(result.is_empty());
    }

    #[test]
    fn test_weekly_single_day() {
        // 2025-01-15 is Wednesday → week starts on 2025-01-12 (Sunday)
        let summaries = vec![make_daily_summary(2025, 1, 15, 100, 50, 0.01)];
        let result = Aggregator::weekly(&summaries);

        assert_eq!(result.len(), 1);
        assert_eq!(result[0].date.to_string(), "2025-01-12");
        assert_eq!(result[0].total_input_tokens, 100);
        assert_eq!(result[0].total_output_tokens, 50);
    }

    #[test]
    fn test_weekly_same_week_merge() {
        // 2025-01-13 (Mon) and 2025-01-15 (Wed) → both in week starting 2025-01-12 (Sun)
        let summaries = vec![
            make_daily_summary(2025, 1, 13, 100, 50, 0.01),
            make_daily_summary(2025, 1, 15, 200, 100, 0.02),
        ];
        let result = Aggregator::weekly(&summaries);

        assert_eq!(result.len(), 1);
        assert_eq!(result[0].date.to_string(), "2025-01-12");
        assert_eq!(result[0].total_input_tokens, 300);
        assert_eq!(result[0].total_output_tokens, 150);
        assert!((result[0].total_cost_usd - 0.03).abs() < f64::EPSILON);
    }

    #[test]
    fn test_weekly_cross_week() {
        // 2025-01-18 (Sat) → week of 2025-01-12
        // 2025-01-19 (Sun) → week of 2025-01-19
        let summaries = vec![
            make_daily_summary(2025, 1, 18, 100, 50, 0.01),
            make_daily_summary(2025, 1, 19, 200, 100, 0.02),
        ];
        let result = Aggregator::weekly(&summaries);

        assert_eq!(result.len(), 2);
        assert_eq!(result[0].date.to_string(), "2025-01-12");
        assert_eq!(result[1].date.to_string(), "2025-01-19");
    }

    #[test]
    fn test_weekly_sunday_stays() {
        // Sunday itself is the start of its own week
        // 2025-01-12 is a Sunday
        let summaries = vec![make_daily_summary(2025, 1, 12, 100, 50, 0.01)];
        let result = Aggregator::weekly(&summaries);

        assert_eq!(result.len(), 1);
        assert_eq!(result[0].date.to_string(), "2025-01-12");
    }

    #[test]
    fn test_weekly_saturday_maps_to_sunday() {
        // 2025-01-18 is Saturday → maps to Sunday 2025-01-12
        let summaries = vec![make_daily_summary(2025, 1, 18, 100, 50, 0.01)];
        let result = Aggregator::weekly(&summaries);

        assert_eq!(result.len(), 1);
        assert_eq!(result[0].date.to_string(), "2025-01-12");
    }

    #[test]
    fn test_weekly_models_merged() {
        let mut models_a = HashMap::new();
        models_a.insert(
            "claude".to_string(),
            ModelUsage {
                input_tokens: 100,
                output_tokens: 50,
                cost_usd: 0.01,
                count: 1,
                ..Default::default()
            },
        );

        let mut models_b = HashMap::new();
        models_b.insert(
            "claude".to_string(),
            ModelUsage {
                input_tokens: 200,
                output_tokens: 100,
                cost_usd: 0.02,
                count: 2,
                ..Default::default()
            },
        );
        models_b.insert(
            "gpt-4".to_string(),
            ModelUsage {
                input_tokens: 50,
                output_tokens: 25,
                cost_usd: 0.005,
                count: 1,
                ..Default::default()
            },
        );

        // Same week (Mon and Wed of 2025-01-12 week)
        let summaries = vec![
            make_daily_summary_with_models(2025, 1, 13, 100, 50, 0.01, models_a),
            make_daily_summary_with_models(2025, 1, 15, 250, 125, 0.025, models_b),
        ];

        let result = Aggregator::weekly(&summaries);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].models.len(), 2);

        let claude = result[0].models.get("claude").unwrap();
        assert_eq!(claude.input_tokens, 300);
        assert_eq!(claude.count, 3);

        let gpt = result[0].models.get("gpt-4").unwrap();
        assert_eq!(gpt.input_tokens, 50);
        assert_eq!(gpt.count, 1);
    }

    #[test]
    fn test_weekly_sorted() {
        let summaries = vec![
            make_daily_summary(2025, 1, 20, 100, 50, 0.01), // week of Jan 19
            make_daily_summary(2025, 1, 6, 200, 100, 0.02), // week of Jan 5
            make_daily_summary(2025, 1, 13, 150, 75, 0.015), // week of Jan 12
        ];
        let result = Aggregator::weekly(&summaries);

        assert_eq!(result.len(), 3);
        assert_eq!(result[0].date.to_string(), "2025-01-05");
        assert_eq!(result[1].date.to_string(), "2025-01-12");
        assert_eq!(result[2].date.to_string(), "2025-01-19");
    }

    // ========== Monthly aggregation tests ==========

    #[test]
    fn test_monthly_empty() {
        let result = Aggregator::monthly(&[]);
        assert!(result.is_empty());
    }

    #[test]
    fn test_monthly_single_day() {
        let summaries = vec![make_daily_summary(2025, 1, 15, 100, 50, 0.01)];
        let result = Aggregator::monthly(&summaries);

        assert_eq!(result.len(), 1);
        assert_eq!(result[0].date.to_string(), "2025-01-01");
        assert_eq!(result[0].total_input_tokens, 100);
    }

    #[test]
    fn test_monthly_same_month_merge() {
        let summaries = vec![
            make_daily_summary(2025, 1, 5, 100, 50, 0.01),
            make_daily_summary(2025, 1, 20, 200, 100, 0.02),
        ];
        let result = Aggregator::monthly(&summaries);

        assert_eq!(result.len(), 1);
        assert_eq!(result[0].date.to_string(), "2025-01-01");
        assert_eq!(result[0].total_input_tokens, 300);
        assert_eq!(result[0].total_output_tokens, 150);
        assert!((result[0].total_cost_usd - 0.03).abs() < f64::EPSILON);
    }

    #[test]
    fn test_monthly_cross_month() {
        let summaries = vec![
            make_daily_summary(2025, 1, 31, 100, 50, 0.01),
            make_daily_summary(2025, 2, 1, 200, 100, 0.02),
        ];
        let result = Aggregator::monthly(&summaries);

        assert_eq!(result.len(), 2);
        assert_eq!(result[0].date.to_string(), "2025-01-01");
        assert_eq!(result[1].date.to_string(), "2025-02-01");
    }

    #[test]
    fn test_monthly_first_of_month() {
        // Date is already first of month
        let summaries = vec![make_daily_summary(2025, 3, 1, 100, 50, 0.01)];
        let result = Aggregator::monthly(&summaries);

        assert_eq!(result.len(), 1);
        assert_eq!(result[0].date.to_string(), "2025-03-01");
    }

    #[test]
    fn test_monthly_sorted() {
        let summaries = vec![
            make_daily_summary(2025, 3, 15, 100, 50, 0.01),
            make_daily_summary(2025, 1, 10, 200, 100, 0.02),
            make_daily_summary(2025, 2, 20, 150, 75, 0.015),
        ];
        let result = Aggregator::monthly(&summaries);

        assert_eq!(result.len(), 3);
        assert_eq!(result[0].date.to_string(), "2025-01-01");
        assert_eq!(result[1].date.to_string(), "2025-02-01");
        assert_eq!(result[2].date.to_string(), "2025-03-01");
    }

    // ========== total_from_daily tests ==========

    #[test]
    fn test_total_from_daily_empty() {
        let result = Aggregator::total_from_daily(&[]);
        assert_eq!(result.total_input_tokens, 0);
        assert_eq!(result.total_output_tokens, 0);
        assert_eq!(result.entry_count, 0);
        assert_eq!(result.day_count, 0);
    }

    #[test]
    fn test_total_from_daily_single() {
        let mut models = HashMap::new();
        models.insert(
            "claude".to_string(),
            ModelUsage {
                input_tokens: 100,
                output_tokens: 50,
                cache_read_tokens: 10,
                cache_creation_tokens: 5,
                thinking_tokens: 0,
                cost_usd: 0.01,
                count: 3,
            },
        );
        let summaries = vec![make_daily_summary_with_models(
            2024, 1, 15, 100, 50, 0.01, models,
        )];

        let result = Aggregator::total_from_daily(&summaries);

        assert_eq!(result.total_input_tokens, 100);
        assert_eq!(result.total_output_tokens, 50);
        assert!((result.total_cost_usd - 0.01).abs() < f64::EPSILON);
        assert_eq!(result.entry_count, 3);
        assert_eq!(result.day_count, 1);
    }

    #[test]
    fn test_total_from_daily_multiple() {
        let mut models_a = HashMap::new();
        models_a.insert(
            "claude".to_string(),
            ModelUsage {
                input_tokens: 100,
                output_tokens: 50,
                cost_usd: 0.01,
                count: 2,
                ..Default::default()
            },
        );
        let mut models_b = HashMap::new();
        models_b.insert(
            "gpt-4".to_string(),
            ModelUsage {
                input_tokens: 200,
                output_tokens: 100,
                cost_usd: 0.02,
                count: 1,
                ..Default::default()
            },
        );
        let summaries = vec![
            make_daily_summary_with_models(2024, 1, 15, 100, 50, 0.01, models_a),
            make_daily_summary_with_models(2024, 1, 16, 200, 100, 0.02, models_b),
        ];

        let result = Aggregator::total_from_daily(&summaries);

        assert_eq!(result.total_input_tokens, 300);
        assert_eq!(result.total_output_tokens, 150);
        assert!((result.total_cost_usd - 0.03).abs() < f64::EPSILON);
        assert_eq!(result.entry_count, 3); // 2 + 1
        assert_eq!(result.day_count, 2);
    }

    // ========== by_model_from_daily tests ==========

    #[test]
    fn test_by_model_from_daily_empty() {
        let result = Aggregator::by_model_from_daily(&[]);
        assert!(result.is_empty());
    }

    #[test]
    fn test_by_model_from_daily_merges_across_days() {
        let mut models_a = HashMap::new();
        models_a.insert(
            "claude".to_string(),
            ModelUsage {
                input_tokens: 100,
                output_tokens: 50,
                cost_usd: 0.01,
                count: 1,
                ..Default::default()
            },
        );
        let mut models_b = HashMap::new();
        models_b.insert(
            "claude".to_string(),
            ModelUsage {
                input_tokens: 200,
                output_tokens: 100,
                cost_usd: 0.02,
                count: 2,
                ..Default::default()
            },
        );
        models_b.insert(
            "gpt-4".to_string(),
            ModelUsage {
                input_tokens: 50,
                output_tokens: 25,
                cost_usd: 0.005,
                count: 1,
                ..Default::default()
            },
        );

        let summaries = vec![
            make_daily_summary_with_models(2024, 1, 15, 100, 50, 0.01, models_a),
            make_daily_summary_with_models(2024, 1, 16, 250, 125, 0.025, models_b),
        ];

        let result = Aggregator::by_model_from_daily(&summaries);

        assert_eq!(result.len(), 2);
        let claude = result.get("claude").unwrap();
        assert_eq!(claude.input_tokens, 300);
        assert_eq!(claude.output_tokens, 150);
        assert_eq!(claude.count, 3);

        let gpt = result.get("gpt-4").unwrap();
        assert_eq!(gpt.input_tokens, 50);
        assert_eq!(gpt.count, 1);
    }

    // ========== accumulate_summary / merge_model_usage gap tests ==========

    #[test]
    fn test_accumulate_summary_with_cache_tokens() {
        let mut target = DailySummary {
            date: chrono::NaiveDate::from_ymd_opt(2025, 1, 1).unwrap(),
            total_input_tokens: 100,
            total_output_tokens: 50,
            total_cache_read_tokens: 10,
            total_cache_creation_tokens: 5,
            total_thinking_tokens: 0,
            total_cost_usd: 0.01,
            models: HashMap::new(),
        };
        let source = DailySummary {
            date: chrono::NaiveDate::from_ymd_opt(2025, 1, 1).unwrap(),
            total_input_tokens: 200,
            total_output_tokens: 100,
            total_cache_read_tokens: 30,
            total_cache_creation_tokens: 15,
            total_thinking_tokens: 0,
            total_cost_usd: 0.02,
            models: HashMap::new(),
        };

        accumulate_summary(&mut target, &source);

        assert_eq!(target.total_input_tokens, 300);
        assert_eq!(target.total_output_tokens, 150);
        assert_eq!(target.total_cache_read_tokens, 40);
        assert_eq!(target.total_cache_creation_tokens, 20);
        assert!((target.total_cost_usd - 0.03).abs() < f64::EPSILON);
    }

    #[test]
    fn test_merge_model_usage_all_fields() {
        let mut target = ModelUsage {
            input_tokens: 100,
            output_tokens: 50,
            cache_read_tokens: 10,
            cache_creation_tokens: 5,
            thinking_tokens: 0,
            cost_usd: 0.01,
            count: 2,
        };
        let source = ModelUsage {
            input_tokens: 200,
            output_tokens: 100,
            cache_read_tokens: 20,
            cache_creation_tokens: 10,
            thinking_tokens: 0,
            cost_usd: 0.02,
            count: 3,
        };

        merge_model_usage(&mut target, &source);

        assert_eq!(target.input_tokens, 300);
        assert_eq!(target.output_tokens, 150);
        assert_eq!(target.cache_read_tokens, 30);
        assert_eq!(target.cache_creation_tokens, 15);
        assert!((target.cost_usd - 0.03).abs() < f64::EPSILON);
        assert_eq!(target.count, 5);
    }

    #[test]
    fn test_total_from_daily_entry_count_zero_count_models() {
        // Models with count=0 should not inflate entry_count
        let mut models = HashMap::new();
        models.insert(
            "claude".to_string(),
            ModelUsage {
                input_tokens: 100,
                output_tokens: 50,
                cost_usd: 0.01,
                count: 0, // zero count edge case
                ..Default::default()
            },
        );
        models.insert(
            "gpt-4".to_string(),
            ModelUsage {
                input_tokens: 200,
                output_tokens: 100,
                cost_usd: 0.02,
                count: 5,
                ..Default::default()
            },
        );
        let summaries = vec![make_daily_summary_with_models(
            2025, 1, 15, 300, 150, 0.03, models,
        )];

        let result = Aggregator::total_from_daily(&summaries);

        assert_eq!(result.entry_count, 5); // 0 + 5
        assert_eq!(result.day_count, 1);
    }

    #[test]
    fn test_accumulate_summary_merges_models() {
        let mut models_target = HashMap::new();
        models_target.insert(
            "claude".to_string(),
            ModelUsage {
                input_tokens: 100,
                output_tokens: 50,
                cost_usd: 0.01,
                count: 1,
                ..Default::default()
            },
        );
        let mut target = DailySummary {
            date: chrono::NaiveDate::from_ymd_opt(2025, 1, 1).unwrap(),
            total_input_tokens: 100,
            total_output_tokens: 50,
            total_cache_read_tokens: 0,
            total_cache_creation_tokens: 0,
            total_thinking_tokens: 0,
            total_cost_usd: 0.01,
            models: models_target,
        };

        let mut models_source = HashMap::new();
        models_source.insert(
            "claude".to_string(),
            ModelUsage {
                input_tokens: 200,
                output_tokens: 100,
                cost_usd: 0.02,
                count: 2,
                ..Default::default()
            },
        );
        models_source.insert(
            "gpt-4".to_string(),
            ModelUsage {
                input_tokens: 50,
                output_tokens: 25,
                cost_usd: 0.005,
                count: 1,
                ..Default::default()
            },
        );
        let source = DailySummary {
            date: chrono::NaiveDate::from_ymd_opt(2025, 1, 1).unwrap(),
            total_input_tokens: 250,
            total_output_tokens: 125,
            total_cache_read_tokens: 0,
            total_cache_creation_tokens: 0,
            total_thinking_tokens: 0,
            total_cost_usd: 0.025,
            models: models_source,
        };

        accumulate_summary(&mut target, &source);

        // Models should be merged
        assert_eq!(target.models.len(), 2);
        let claude = target.models.get("claude").unwrap();
        assert_eq!(claude.input_tokens, 300);
        assert_eq!(claude.count, 3);
        let gpt = target.models.get("gpt-4").unwrap();
        assert_eq!(gpt.input_tokens, 50);
        assert_eq!(gpt.count, 1);
    }

    // ========== by_source tests ==========

    #[allow(clippy::too_many_arguments)]
    fn make_entry_with_source(
        year: i32,
        month: u32,
        day: u32,
        model: Option<&str>,
        input: u64,
        output: u64,
        cost: Option<f64>,
        source: Option<&str>,
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
            source: source.map(String::from),
            provider: None,
        }
    }

    #[test]
    fn test_by_source_empty() {
        let result = Aggregator::by_source(&[]);
        assert!(result.is_empty());
    }

    #[test]
    fn test_by_source_single_source() {
        let entries = vec![
            make_entry_with_source(
                2024,
                1,
                15,
                Some("claude"),
                100,
                50,
                Some(0.01),
                Some("claude"),
            ),
            make_entry_with_source(
                2024,
                1,
                16,
                Some("claude"),
                200,
                100,
                Some(0.02),
                Some("claude"),
            ),
        ];
        let result = Aggregator::by_source(&entries);

        assert_eq!(result.len(), 1);
        assert_eq!(result[0].source, "claude");
        assert_eq!(result[0].total_tokens, 450); // 100+50 + 200+100
        assert!((result[0].total_cost_usd - 0.03).abs() < f64::EPSILON);
    }

    #[test]
    fn test_by_source_multiple_sources() {
        let entries = vec![
            make_entry_with_source(
                2024,
                1,
                15,
                Some("claude"),
                100,
                50,
                Some(0.01),
                Some("claude"),
            ),
            make_entry_with_source(
                2024,
                1,
                16,
                Some("gpt-4"),
                300,
                150,
                Some(0.03),
                Some("opencode"),
            ),
            make_entry_with_source(
                2024,
                1,
                17,
                Some("gemini"),
                50,
                25,
                Some(0.005),
                Some("gemini"),
            ),
        ];
        let result = Aggregator::by_source(&entries);

        assert_eq!(result.len(), 3);
        // Sorted by total_tokens descending
        assert_eq!(result[0].source, "opencode");
        assert_eq!(result[0].total_tokens, 450); // 300+150
        assert_eq!(result[1].source, "claude");
        assert_eq!(result[1].total_tokens, 150); // 100+50
        assert_eq!(result[2].source, "gemini");
        assert_eq!(result[2].total_tokens, 75); // 50+25
    }

    #[test]
    fn test_by_source_none_becomes_unknown() {
        let entries = vec![make_entry_with_source(
            2024,
            1,
            15,
            Some("model"),
            100,
            50,
            Some(0.01),
            None,
        )];
        let result = Aggregator::by_source(&entries);

        assert_eq!(result.len(), 1);
        assert_eq!(result[0].source, "unknown");
    }

    // ========== merge_by_date tests ==========

    #[test]
    fn test_merge_by_date_empty() {
        let result = Aggregator::merge_by_date(vec![]);
        assert!(result.is_empty());
    }

    #[test]
    fn test_merge_by_date_no_duplicates() {
        let summaries = vec![
            make_daily_summary(2025, 1, 10, 100, 50, 0.01),
            make_daily_summary(2025, 1, 15, 200, 100, 0.02),
        ];
        let result = Aggregator::merge_by_date(summaries);

        assert_eq!(result.len(), 2);
        assert_eq!(result[0].date.to_string(), "2025-01-10");
        assert_eq!(result[1].date.to_string(), "2025-01-15");
    }

    #[test]
    fn test_merge_by_date_merges_same_date() {
        // Two summaries from different sources for the same date
        let summaries = vec![
            make_daily_summary(2025, 1, 15, 100, 50, 0.01),
            make_daily_summary(2025, 1, 15, 200, 100, 0.02),
        ];
        let result = Aggregator::merge_by_date(summaries);

        assert_eq!(result.len(), 1);
        assert_eq!(result[0].date.to_string(), "2025-01-15");
        assert_eq!(result[0].total_input_tokens, 300);
        assert_eq!(result[0].total_output_tokens, 150);
        assert!((result[0].total_cost_usd - 0.03).abs() < f64::EPSILON);
    }

    #[test]
    fn test_merge_by_date_sorted() {
        let summaries = vec![
            make_daily_summary(2025, 1, 20, 100, 50, 0.01),
            make_daily_summary(2025, 1, 10, 200, 100, 0.02),
            make_daily_summary(2025, 1, 15, 150, 75, 0.015),
        ];
        let result = Aggregator::merge_by_date(summaries);

        assert_eq!(result.len(), 3);
        assert_eq!(result[0].date.to_string(), "2025-01-10");
        assert_eq!(result[1].date.to_string(), "2025-01-15");
        assert_eq!(result[2].date.to_string(), "2025-01-20");
    }

    #[test]
    fn test_merge_by_date_merges_models() {
        let mut models_a = HashMap::new();
        models_a.insert(
            "claude".to_string(),
            ModelUsage {
                input_tokens: 100,
                output_tokens: 50,
                cost_usd: 0.01,
                count: 1,
                ..Default::default()
            },
        );

        let mut models_b = HashMap::new();
        models_b.insert(
            "gpt-4".to_string(),
            ModelUsage {
                input_tokens: 200,
                output_tokens: 100,
                cost_usd: 0.02,
                count: 2,
                ..Default::default()
            },
        );

        let summaries = vec![
            make_daily_summary_with_models(2025, 1, 15, 100, 50, 0.01, models_a),
            make_daily_summary_with_models(2025, 1, 15, 200, 100, 0.02, models_b),
        ];
        let result = Aggregator::merge_by_date(summaries);

        assert_eq!(result.len(), 1);
        assert_eq!(result[0].models.len(), 2);
        assert!(result[0].models.contains_key("claude"));
        assert!(result[0].models.contains_key("gpt-4"));
    }
}
