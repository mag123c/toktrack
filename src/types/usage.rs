//! Usage types for token tracking

use chrono::{DateTime, Local, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize)]
pub struct StatsData {
    pub total_tokens: u64,
    pub daily_avg_tokens: u64,
    pub peak_day: Option<(NaiveDate, u64)>,
    pub total_cost: f64,
    pub daily_avg_cost: f64,
    pub active_days: u32,
}

impl StatsData {
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
                + summary.total_cache_creation_tokens
                + summary.total_reasoning_tokens;

            total_tokens = total_tokens.saturating_add(day_tokens);
            total_cost += summary.total_cost_usd;

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

/// A single usage record, normalized across all CLI sources.
///
/// # Token field contract (v2)
/// Every parser MUST populate these fields with the same meaning so that
/// aggregation and cost are consistent regardless of source:
/// - `input_tokens`: billable **non-cached** input (does NOT include `cache_read_tokens`)
/// - `cache_read_tokens`: cached input that was read
/// - `cache_creation_tokens`: cache write
/// - `output_tokens`: **visible** output only (does NOT include reasoning)
/// - `reasoning_tokens`: hidden/reasoning output (formerly `thinking_tokens`)
/// - `reported_total_tokens`: upstream-reported total, for **reconciliation only** —
///   never summed into `total_tokens()` and never priced.
///
/// Invariants: `total_tokens() == input + output + cache_read + cache_creation + reasoning`;
/// cost charges `(output + reasoning)` at the output rate; where
/// `reported_total_tokens` is `Some`, it must equal `total_tokens()`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct UsageEntry {
    pub timestamp: DateTime<Utc>,
    pub model: Option<String>,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_creation_tokens: u64,
    #[serde(default, alias = "thinking_tokens")]
    pub reasoning_tokens: u64,
    #[serde(default)]
    pub cache_creation_5m_tokens: u64,
    #[serde(default)]
    pub cache_creation_1h_tokens: u64,
    #[serde(default)]
    pub web_search_requests: u64,
    /// Web fetch tool invocations. No LiteLLM price exists (Anthropic bills the
    /// fetched content as tokens), so this is tracked for completeness and priced
    /// only via a custom `global.web_fetch_per_request` override.
    #[serde(default)]
    pub web_fetch_requests: u64,
    /// Upstream-reported total token count, when the source provides one
    /// (e.g. Gemini `tokens.total`). Reconciliation only — not summed, not priced.
    #[serde(default)]
    pub reported_total_tokens: Option<u64>,
    pub cost_usd: Option<f64>,
    pub message_id: Option<String>,
    pub request_id: Option<String>,
    #[serde(default)]
    pub source: Option<String>,
    /// Provider ID (e.g., "anthropic", "github-copilot")
    #[serde(default)]
    pub provider: Option<String>,
    /// Project this usage belongs to (e.g. the working directory for Claude Code).
    /// `None` for sources that do not record a project (Codex, Gemini, etc.).
    #[serde(default)]
    pub project: Option<String>,
}

impl UsageEntry {
    #[allow(dead_code)]
    pub fn total_tokens(&self) -> u64 {
        self.input_tokens
            + self.output_tokens
            + self.cache_read_tokens
            + self.cache_creation_tokens
            + self.reasoning_tokens
    }

    /// Convert UTC timestamp to local timezone date.
    /// Ensures date grouping matches the user's local calendar.
    pub fn local_date(&self) -> NaiveDate {
        self.timestamp.with_timezone(&Local).date_naive()
    }

    pub fn dedup_hash(&self) -> Option<String> {
        match (&self.message_id, &self.request_id) {
            (Some(msg), Some(req)) => Some(format!("{}:{}", msg, req)),
            (Some(msg), None) => {
                let model = self.model.as_deref().unwrap_or("unknown");
                Some(format!(
                    "{}:{}:{}:{}",
                    msg, model, self.input_tokens, self.output_tokens
                ))
            }
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DailySummary {
    pub date: NaiveDate,
    pub total_input_tokens: u64,
    pub total_output_tokens: u64,
    pub total_cache_read_tokens: u64,
    pub total_cache_creation_tokens: u64,
    #[serde(default, alias = "total_thinking_tokens")]
    pub total_reasoning_tokens: u64,
    #[serde(default)]
    pub total_cache_creation_5m_tokens: u64,
    #[serde(default)]
    pub total_cache_creation_1h_tokens: u64,
    #[serde(default)]
    pub total_web_search_requests: u64,
    pub total_cost_usd: f64,
    pub models: HashMap<String, ModelUsage>,
    /// Per-project breakdown for this day. Keyed by project identifier (the raw
    /// working directory for Claude Code, or "(no project)" for sources that do
    /// not record one). Empty for caches written before this field existed.
    ///
    /// `skip_serializing_if` keeps the key out of JSON when empty so the public
    /// `--json` schema is unchanged for days with no project data; the CLI also
    /// strips populated maps from `--json` output (raw paths can include the OS
    /// username) — see `cli::redact_project_paths`. The persistent cache still
    /// retains the data because it serializes non-empty maps.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub projects: HashMap<String, ProjectUsage>,
}

/// The sentinel project key used for usage from sources that do not record a
/// project (Codex, Gemini, Qwen, OpenCode, PI Agent).
pub const NO_PROJECT: &str = "(no project)";

/// Usage aggregated for a single project within a day (or rolled up across days).
/// Mirrors [`ModelUsage`] but also keeps a nested per-model breakdown so a project
/// can be drilled into by model.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct ProjectUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_creation_tokens: u64,
    #[serde(default)]
    pub reasoning_tokens: u64,
    #[serde(default)]
    pub cache_creation_5m_tokens: u64,
    #[serde(default)]
    pub cache_creation_1h_tokens: u64,
    #[serde(default)]
    pub web_search_requests: u64,
    pub cost_usd: f64,
    pub count: u64,
    /// Per-model breakdown within this project.
    #[serde(default)]
    pub models: HashMap<String, ModelUsage>,
}

impl ProjectUsage {
    pub fn total_tokens(&self) -> u64 {
        self.input_tokens
            + self.output_tokens
            + self.cache_read_tokens
            + self.cache_creation_tokens
            + self.reasoning_tokens
    }

    /// Accumulate a single entry, also updating the nested per-model breakdown
    /// under `model_key`.
    pub fn add(&mut self, entry: &UsageEntry, cost: f64, model_key: &str) {
        self.input_tokens = self.input_tokens.saturating_add(entry.input_tokens);
        self.output_tokens = self.output_tokens.saturating_add(entry.output_tokens);
        self.cache_read_tokens = self
            .cache_read_tokens
            .saturating_add(entry.cache_read_tokens);
        self.cache_creation_tokens = self
            .cache_creation_tokens
            .saturating_add(entry.cache_creation_tokens);
        self.reasoning_tokens = self.reasoning_tokens.saturating_add(entry.reasoning_tokens);
        self.cache_creation_5m_tokens = self
            .cache_creation_5m_tokens
            .saturating_add(entry.cache_creation_5m_tokens);
        self.cache_creation_1h_tokens = self
            .cache_creation_1h_tokens
            .saturating_add(entry.cache_creation_1h_tokens);
        self.web_search_requests = self
            .web_search_requests
            .saturating_add(entry.web_search_requests);
        self.cost_usd += cost;
        self.count = self.count.saturating_add(1);
        self.models
            .entry(model_key.to_string())
            .or_default()
            .add(entry, cost);
    }

    /// Merge another project's usage into this one (used when rolling daily
    /// summaries up into weekly/monthly views or across sources).
    pub fn merge(&mut self, other: &ProjectUsage) {
        self.input_tokens = self.input_tokens.saturating_add(other.input_tokens);
        self.output_tokens = self.output_tokens.saturating_add(other.output_tokens);
        self.cache_read_tokens = self
            .cache_read_tokens
            .saturating_add(other.cache_read_tokens);
        self.cache_creation_tokens = self
            .cache_creation_tokens
            .saturating_add(other.cache_creation_tokens);
        self.reasoning_tokens = self.reasoning_tokens.saturating_add(other.reasoning_tokens);
        self.cache_creation_5m_tokens = self
            .cache_creation_5m_tokens
            .saturating_add(other.cache_creation_5m_tokens);
        self.cache_creation_1h_tokens = self
            .cache_creation_1h_tokens
            .saturating_add(other.cache_creation_1h_tokens);
        self.web_search_requests = self
            .web_search_requests
            .saturating_add(other.web_search_requests);
        self.cost_usd += other.cost_usd;
        self.count = self.count.saturating_add(other.count);
        for (model, usage) in &other.models {
            self.models.entry(model.clone()).or_default().merge(usage);
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct ModelUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_creation_tokens: u64,
    #[serde(default, alias = "thinking_tokens")]
    pub reasoning_tokens: u64,
    #[serde(default)]
    pub cache_creation_5m_tokens: u64,
    #[serde(default)]
    pub cache_creation_1h_tokens: u64,
    #[serde(default)]
    pub web_search_requests: u64,
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
        self.reasoning_tokens = self.reasoning_tokens.saturating_add(entry.reasoning_tokens);
        self.cache_creation_5m_tokens = self
            .cache_creation_5m_tokens
            .saturating_add(entry.cache_creation_5m_tokens);
        self.cache_creation_1h_tokens = self
            .cache_creation_1h_tokens
            .saturating_add(entry.cache_creation_1h_tokens);
        self.web_search_requests = self
            .web_search_requests
            .saturating_add(entry.web_search_requests);
        self.cost_usd += cost;
        self.count = self.count.saturating_add(1);
    }

    /// Merge another model's usage into this one.
    pub fn merge(&mut self, other: &ModelUsage) {
        self.input_tokens = self.input_tokens.saturating_add(other.input_tokens);
        self.output_tokens = self.output_tokens.saturating_add(other.output_tokens);
        self.cache_read_tokens = self
            .cache_read_tokens
            .saturating_add(other.cache_read_tokens);
        self.cache_creation_tokens = self
            .cache_creation_tokens
            .saturating_add(other.cache_creation_tokens);
        self.reasoning_tokens = self.reasoning_tokens.saturating_add(other.reasoning_tokens);
        self.cache_creation_5m_tokens = self
            .cache_creation_5m_tokens
            .saturating_add(other.cache_creation_5m_tokens);
        self.cache_creation_1h_tokens = self
            .cache_creation_1h_tokens
            .saturating_add(other.cache_creation_1h_tokens);
        self.web_search_requests = self
            .web_search_requests
            .saturating_add(other.web_search_requests);
        self.cost_usd += other.cost_usd;
        self.count = self.count.saturating_add(other.count);
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct TotalSummary {
    pub total_input_tokens: u64,
    pub total_output_tokens: u64,
    pub total_cache_read_tokens: u64,
    pub total_cache_creation_tokens: u64,
    #[serde(default, alias = "total_thinking_tokens")]
    pub total_reasoning_tokens: u64,
    #[serde(default)]
    pub total_cache_creation_5m_tokens: u64,
    #[serde(default)]
    pub total_cache_creation_1h_tokens: u64,
    #[serde(default)]
    pub total_web_search_requests: u64,
    pub total_cost_usd: f64,
    pub entry_count: u64,
    pub day_count: u64,
}

fn default_true() -> bool {
    true
}

/// Usage aggregated by source CLI (claude, opencode, gemini, etc.)
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SourceUsage {
    pub source: String,
    pub total_tokens: u64,
    pub total_cost_usd: f64,
    /// False for sources detected on disk whose token usage can't be read
    /// (no file-readable usage). Rendered as a disabled notice row. Reserved for
    /// future detect-but-can't-parse sources; all current parsers set this true.
    #[serde(default = "default_true")]
    pub supported: bool,
    /// True when this source's cost was LiteLLM-calculated (the source did not
    /// provide its own cost), i.e. an estimate. Rendered with a marker + legend.
    #[serde(default)]
    pub estimated: bool,
}

impl Default for SourceUsage {
    fn default() -> Self {
        Self {
            source: String::new(),
            total_tokens: 0,
            total_cost_usd: 0.0,
            supported: true,
            estimated: false,
        }
    }
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
            total_reasoning_tokens: 0,
            total_cache_creation_5m_tokens: 0,
            total_cache_creation_1h_tokens: 0,
            total_web_search_requests: 0,
            total_cost_usd: cost,
            models: HashMap::new(),
            projects: HashMap::new(),
        }
    }

    #[test]
    fn test_daily_summary_omits_empty_projects_from_json() {
        let s = make_summary(2026, 1, 1, 100, 50, 0, 0, 1.0);
        let json = serde_json::to_string(&s).unwrap();
        assert!(
            !json.contains("projects"),
            "empty projects map must be omitted from JSON: {json}"
        );
    }

    #[test]
    fn test_daily_summary_serializes_nonempty_projects_and_round_trips() {
        let mut s = make_summary(2026, 1, 1, 100, 50, 0, 0, 1.0);
        s.projects.insert(
            "/work/demo".to_string(),
            ProjectUsage {
                input_tokens: 100,
                ..Default::default()
            },
        );
        let json = serde_json::to_string(&s).unwrap();
        assert!(json.contains("projects"));
        // The cache relies on this round-trip to preserve per-project history.
        let back: DailySummary = serde_json::from_str(&json).unwrap();
        assert_eq!(back.projects.len(), 1);
        assert!(back.projects.contains_key("/work/demo"));
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
            reasoning_tokens: 0,
            cache_creation_5m_tokens: 0,
            cache_creation_1h_tokens: 0,
            web_search_requests: 0,
            web_fetch_requests: 0,
            reported_total_tokens: None,
            cost_usd: None,
            message_id: None,
            request_id: None,
            source: None,
            provider: None,
            project: None,
        };
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
            reasoning_tokens: 30,
            cache_creation_5m_tokens: 0,
            cache_creation_1h_tokens: 0,
            web_search_requests: 0,
            web_fetch_requests: 0,
            reported_total_tokens: None,
            cost_usd: None,
            message_id: None,
            request_id: None,
            source: Some("gemini".into()),
            provider: None,
            project: None,
        };
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
            reasoning_tokens: 0,
            cache_creation_5m_tokens: 0,
            cache_creation_1h_tokens: 0,
            web_search_requests: 0,
            web_fetch_requests: 0,
            reported_total_tokens: None,
            cost_usd: None,
            message_id: Some("msg123".into()),
            request_id: Some("req456".into()),
            source: None,
            provider: None,
            project: None,
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
            reasoning_tokens: 0,
            cache_creation_5m_tokens: 0,
            cache_creation_1h_tokens: 0,
            web_search_requests: 0,
            web_fetch_requests: 0,
            reported_total_tokens: None,
            cost_usd: None,
            message_id: None,
            request_id: Some("req456".into()),
            source: None,
            provider: None,
            project: None,
        };
        assert_eq!(entry.dedup_hash(), None);
    }

    #[test]
    fn test_usage_entry_dedup_hash_fallback_message_only() {
        let entry = UsageEntry {
            timestamp: Utc::now(),
            model: Some("gpt-4".into()),
            input_tokens: 100,
            output_tokens: 50,
            cache_read_tokens: 0,
            cache_creation_tokens: 0,
            reasoning_tokens: 0,
            cache_creation_5m_tokens: 0,
            cache_creation_1h_tokens: 0,
            web_search_requests: 0,
            web_fetch_requests: 0,
            reported_total_tokens: None,
            cost_usd: None,
            message_id: Some("msg789".into()),
            request_id: None,
            source: None,
            provider: None,
            project: None,
        };
        assert_eq!(entry.dedup_hash(), Some("msg789:gpt-4:100:50".into()));
    }

    #[test]
    fn test_local_date_matches_local_timezone() {
        use chrono::TimeZone;
        // 2024-02-06 03:00 UTC = 2024-02-06 12:00 KST(+9)
        // date_naive() would give 2024-02-06 in both cases here,
        // but the point is local_date() uses Local timezone conversion
        let utc_ts = Utc.with_ymd_and_hms(2024, 2, 6, 3, 0, 0).unwrap();
        let entry = UsageEntry {
            timestamp: utc_ts,
            model: Some("claude".into()),
            input_tokens: 100,
            output_tokens: 50,
            cache_read_tokens: 0,
            cache_creation_tokens: 0,
            reasoning_tokens: 0,
            cache_creation_5m_tokens: 0,
            cache_creation_1h_tokens: 0,
            web_search_requests: 0,
            web_fetch_requests: 0,
            reported_total_tokens: None,
            cost_usd: None,
            message_id: None,
            request_id: None,
            source: None,
            provider: None,
            project: None,
        };

        let local_date = entry.local_date();
        // Verify it matches what chrono::Local would produce
        let expected = utc_ts.with_timezone(&Local).date_naive();
        assert_eq!(local_date, expected);

        // Also verify date_naive (UTC) vs local_date can differ
        // For UTC+N timezones where N>0, a late-night UTC timestamp
        // may map to the next day in local time
        let late_utc = Utc.with_ymd_and_hms(2024, 2, 5, 23, 0, 0).unwrap();
        let late_entry = UsageEntry {
            timestamp: late_utc,
            model: None,
            input_tokens: 0,
            output_tokens: 0,
            cache_read_tokens: 0,
            cache_creation_tokens: 0,
            reasoning_tokens: 0,
            cache_creation_5m_tokens: 0,
            cache_creation_1h_tokens: 0,
            web_search_requests: 0,
            web_fetch_requests: 0,
            reported_total_tokens: None,
            cost_usd: None,
            message_id: None,
            request_id: None,
            source: None,
            provider: None,
            project: None,
        };
        let local = late_entry.local_date();
        let utc_naive = late_utc.date_naive();
        // In any timezone east of UTC, local_date >= utc date_naive
        let local_offset = Local::now().offset().local_minus_utc();
        if local_offset > 0 {
            assert!(local >= utc_naive);
        }
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
            reasoning_tokens: 0,
            cache_creation_5m_tokens: 0,
            cache_creation_1h_tokens: 0,
            web_search_requests: 0,
            web_fetch_requests: 0,
            reported_total_tokens: None,
            cost_usd: None,
            message_id: None,
            request_id: None,
            source: None,
            provider: None,
            project: None,
        };
        usage.add(&entry, 0.01);

        assert_eq!(usage.input_tokens, 100);
        assert_eq!(usage.output_tokens, 50);
        assert_eq!(usage.cache_read_tokens, 20);
        assert_eq!(usage.cost_usd, 0.01);
        assert_eq!(usage.count, 1);
    }
}
