//! Report data structures and period filtering

use chrono::NaiveDate;

use crate::services::{display_name, Aggregator};
use crate::types::{DailySummary, SourceUsage};

/// A single model's aggregated report
#[derive(Debug, Clone)]
pub struct ModelReport {
    pub name: String,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cost_usd: f64,
    #[allow(dead_code)]
    pub count: u64,
}

/// A single source's aggregated report
#[derive(Debug, Clone)]
pub struct SourceReport {
    pub name: String,
    pub total_tokens: u64,
    pub cost_usd: f64,
}

/// A single day's report
#[derive(Debug, Clone)]
pub struct DayReport {
    pub date: NaiveDate,
    #[allow(dead_code)]
    pub total_tokens: u64,
    pub cost_usd: f64,
}

/// Intermediate report data, ready for rendering
#[derive(Debug, Clone)]
pub struct ReportData {
    pub period_label: String,
    pub total_cost: f64,
    pub total_input_tokens: u64,
    pub total_output_tokens: u64,
    pub active_days: u32,
    pub total_days: u32,
    pub by_model: Vec<ModelReport>,
    pub by_source: Vec<SourceReport>,
    pub daily: Vec<DayReport>,
    pub most_expensive_day: Option<DayReport>,
}

impl ReportData {
    /// Build report data from summaries filtered to [start, end]
    pub fn from_summaries(
        summaries: &[DailySummary],
        source_usage: &[SourceUsage],
        start: NaiveDate,
        end: NaiveDate,
    ) -> Self {
        let period_label = format!("{} ~ {}", start.format("%Y-%m-%d"), end.format("%Y-%m-%d"));
        let total_days = (end - start).num_days().max(0) as u32 + 1;

        let filtered: Vec<&DailySummary> = summaries
            .iter()
            .filter(|s| s.date >= start && s.date <= end)
            .collect();

        if filtered.is_empty() {
            return Self {
                period_label,
                total_cost: 0.0,
                total_input_tokens: 0,
                total_output_tokens: 0,
                active_days: 0,
                total_days,
                by_model: Vec::new(),
                by_source: Vec::new(),
                daily: Vec::new(),
                most_expensive_day: None,
            };
        }

        let active_days = filtered.len() as u32;
        let mut total_cost = 0.0;
        let mut total_input_tokens: u64 = 0;
        let mut total_output_tokens: u64 = 0;
        let mut most_expensive_day: Option<DayReport> = None;
        let mut daily = Vec::with_capacity(filtered.len());

        for s in &filtered {
            total_cost += s.total_cost_usd;
            total_input_tokens = total_input_tokens.saturating_add(s.total_input_tokens);
            total_output_tokens = total_output_tokens.saturating_add(s.total_output_tokens);

            let day_tokens = s.total_input_tokens.saturating_add(s.total_output_tokens);
            let day = DayReport {
                date: s.date,
                total_tokens: day_tokens,
                cost_usd: s.total_cost_usd,
            };

            match &most_expensive_day {
                None => most_expensive_day = Some(day.clone()),
                Some(prev) if s.total_cost_usd > prev.cost_usd => {
                    most_expensive_day = Some(day.clone());
                }
                _ => {}
            }

            daily.push(day);
        }

        daily.sort_by_key(|d| d.date);

        // Build model reports
        let owned_filtered: Vec<DailySummary> = filtered.into_iter().cloned().collect();
        let model_map = Aggregator::by_model_from_daily(&owned_filtered);
        let mut by_model: Vec<ModelReport> = model_map
            .into_iter()
            .map(|(name, usage)| ModelReport {
                name: display_name(&name),
                input_tokens: usage.input_tokens,
                output_tokens: usage.output_tokens,
                cost_usd: usage.cost_usd,
                count: usage.count,
            })
            .collect();
        by_model.sort_by(|a, b| b.cost_usd.total_cmp(&a.cost_usd));

        // Build source reports
        let mut by_source: Vec<SourceReport> = source_usage
            .iter()
            .map(|s| SourceReport {
                name: s.source.clone(),
                total_tokens: s.total_tokens,
                cost_usd: s.total_cost_usd,
            })
            .collect();
        by_source.sort_by(|a, b| b.cost_usd.total_cmp(&a.cost_usd));

        Self {
            period_label,
            total_cost,
            total_input_tokens,
            total_output_tokens,
            active_days,
            total_days,
            by_model,
            by_source,
            daily,
            most_expensive_day,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::ModelUsage;
    use std::collections::HashMap;

    fn make_summary(year: i32, month: u32, day: u32, cost: f64) -> DailySummary {
        let mut models = HashMap::new();
        models.insert(
            "claude-sonnet-4".to_string(),
            ModelUsage {
                input_tokens: 1000,
                output_tokens: 500,
                cost_usd: cost,
                count: 5,
                ..Default::default()
            },
        );
        DailySummary {
            date: NaiveDate::from_ymd_opt(year, month, day).unwrap(),
            total_input_tokens: 1000,
            total_output_tokens: 500,
            total_cache_read_tokens: 0,
            total_cache_creation_tokens: 0,
            total_thinking_tokens: 0,
            total_cost_usd: cost,
            models,
        }
    }

    #[test]
    fn test_empty_data() {
        let data = ReportData::from_summaries(
            &[],
            &[],
            NaiveDate::from_ymd_opt(2024, 1, 1).unwrap(),
            NaiveDate::from_ymd_opt(2024, 1, 7).unwrap(),
        );
        assert_eq!(data.active_days, 0);
        assert_eq!(data.total_days, 7);
        assert!(data.by_model.is_empty());
        assert!(data.most_expensive_day.is_none());
    }

    #[test]
    fn test_single_day() {
        let summaries = vec![make_summary(2024, 1, 5, 1.50)];
        let data = ReportData::from_summaries(
            &summaries,
            &[],
            NaiveDate::from_ymd_opt(2024, 1, 1).unwrap(),
            NaiveDate::from_ymd_opt(2024, 1, 7).unwrap(),
        );
        assert_eq!(data.active_days, 1);
        assert_eq!(data.total_days, 7);
        assert!((data.total_cost - 1.50).abs() < f64::EPSILON);
        assert_eq!(data.total_input_tokens, 1000);
        assert_eq!(data.total_output_tokens, 500);
        assert_eq!(data.by_model.len(), 1);
        assert_eq!(data.by_model[0].name, "Sonnet 4");
        assert!(data.most_expensive_day.is_some());
    }

    #[test]
    fn test_multiple_days_sorted() {
        let summaries = vec![
            make_summary(2024, 1, 3, 0.50),
            make_summary(2024, 1, 5, 2.00),
            make_summary(2024, 1, 1, 1.00),
        ];
        let data = ReportData::from_summaries(
            &summaries,
            &[],
            NaiveDate::from_ymd_opt(2024, 1, 1).unwrap(),
            NaiveDate::from_ymd_opt(2024, 1, 7).unwrap(),
        );
        assert_eq!(data.active_days, 3);
        assert!((data.total_cost - 3.50).abs() < f64::EPSILON);
        assert_eq!(
            data.daily[0].date,
            NaiveDate::from_ymd_opt(2024, 1, 1).unwrap()
        );
        assert_eq!(
            data.daily[2].date,
            NaiveDate::from_ymd_opt(2024, 1, 5).unwrap()
        );
        let expensive = data.most_expensive_day.unwrap();
        assert_eq!(expensive.date, NaiveDate::from_ymd_opt(2024, 1, 5).unwrap());
    }

    #[test]
    fn test_filter_excludes_out_of_range() {
        let summaries = vec![
            make_summary(2024, 1, 1, 1.00),
            make_summary(2024, 1, 10, 2.00),
        ];
        let data = ReportData::from_summaries(
            &summaries,
            &[],
            NaiveDate::from_ymd_opt(2024, 1, 1).unwrap(),
            NaiveDate::from_ymd_opt(2024, 1, 7).unwrap(),
        );
        assert_eq!(data.active_days, 1);
        assert!((data.total_cost - 1.00).abs() < f64::EPSILON);
    }

    #[test]
    fn test_model_sort_by_cost_desc() {
        let mut models = HashMap::new();
        models.insert(
            "claude-sonnet-4".to_string(),
            ModelUsage {
                input_tokens: 100,
                output_tokens: 50,
                cost_usd: 0.50,
                count: 1,
                ..Default::default()
            },
        );
        models.insert(
            "claude-opus-4-5".to_string(),
            ModelUsage {
                input_tokens: 200,
                output_tokens: 100,
                cost_usd: 2.00,
                count: 1,
                ..Default::default()
            },
        );
        let summaries = vec![DailySummary {
            date: NaiveDate::from_ymd_opt(2024, 1, 5).unwrap(),
            total_input_tokens: 300,
            total_output_tokens: 150,
            total_cache_read_tokens: 0,
            total_cache_creation_tokens: 0,
            total_thinking_tokens: 0,
            total_cost_usd: 2.50,
            models,
        }];
        let data = ReportData::from_summaries(
            &summaries,
            &[],
            NaiveDate::from_ymd_opt(2024, 1, 1).unwrap(),
            NaiveDate::from_ymd_opt(2024, 1, 7).unwrap(),
        );
        assert_eq!(data.by_model[0].name, "Opus 4.5");
        assert_eq!(data.by_model[1].name, "Sonnet 4");
    }

    #[test]
    fn test_source_reports() {
        let sources = vec![
            SourceUsage {
                source: "claude".to_string(),
                total_tokens: 5000,
                total_cost_usd: 3.00,
            },
            SourceUsage {
                source: "codex".to_string(),
                total_tokens: 2000,
                total_cost_usd: 1.00,
            },
        ];
        let summaries = vec![make_summary(2024, 1, 5, 4.00)];
        let data = ReportData::from_summaries(
            &summaries,
            &sources,
            NaiveDate::from_ymd_opt(2024, 1, 1).unwrap(),
            NaiveDate::from_ymd_opt(2024, 1, 7).unwrap(),
        );
        assert_eq!(data.by_source.len(), 2);
        assert_eq!(data.by_source[0].name, "claude");
        assert_eq!(data.by_source[1].name, "codex");
    }

    #[test]
    fn test_period_label_format() {
        let data = ReportData::from_summaries(
            &[],
            &[],
            NaiveDate::from_ymd_opt(2024, 3, 1).unwrap(),
            NaiveDate::from_ymd_opt(2024, 3, 7).unwrap(),
        );
        assert_eq!(data.period_label, "2024-03-01 ~ 2024-03-07");
    }
}
