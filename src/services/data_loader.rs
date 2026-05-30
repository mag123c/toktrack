//! Unified data loading service for CLI and TUI
//!
//! This module provides a single `DataLoaderService` that consolidates
//! the duplicated data loading logic from CLI and TUI.

use std::collections::HashMap;
use std::time::SystemTime;

use chrono::{Local, TimeZone};

use crate::parsers::{ParserRegistry, SourceInstance};
use crate::services::{Aggregator, DailySummaryCacheService, PricingService};
use crate::types::{CacheWarning, DailySummary, Result, SourceUsage, ToktrackError, UsageEntry};

/// Check if there's a date gap between the latest cached date and yesterday.
///
/// A gap exists when `latest_cached < yesterday - 1 day`, meaning there are
/// dates between the cache's coverage and the warm path's window that would
/// never be parsed. When a gap is detected, the parser should fall back to
/// a full re-parse to fill missing dates.
fn has_date_gap(latest_cached: Option<chrono::NaiveDate>, yesterday: chrono::NaiveDate) -> bool {
    match latest_cached {
        Some(latest) => latest < yesterday - chrono::Duration::days(1),
        None => false, // No cache → no gap to detect; cold path handles this
    }
}

/// Compute the warm-path cutoff: yesterday 00:00:00 local time.
///
/// Files modified on or after this time are re-parsed, ensuring that
/// "yesterday" (the most recent completed day) is always recomputed
/// before being trusted as a complete cached date.
fn warm_path_since() -> SystemTime {
    let yesterday = Local::now().date_naive() - chrono::Duration::days(1);
    let yesterday_midnight = yesterday.and_hms_opt(0, 0, 0).unwrap();
    let utc = match Local.from_local_datetime(&yesterday_midnight) {
        chrono::LocalResult::Single(dt) => dt.to_utc(),
        chrono::LocalResult::Ambiguous(earlier, _) => earlier.to_utc(),
        chrono::LocalResult::None => {
            // DST spring-forward: midnight doesn't exist, use 01:00
            let fallback = yesterday.and_hms_opt(1, 0, 0).unwrap();
            Local
                .from_local_datetime(&fallback)
                .earliest()
                .expect("01:00 should always exist after spring-forward")
                .to_utc()
        }
    };
    SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(utc.timestamp() as u64)
}

/// Result of loading data from all parsers
#[derive(Debug)]
pub struct LoadResult {
    /// Daily summaries from all sources, merged by date
    pub summaries: Vec<DailySummary>,
    /// Usage breakdown by source CLI
    pub source_usage: Vec<SourceUsage>,
    /// Per-source daily summaries (not merged across sources)
    pub source_summaries: HashMap<String, Vec<DailySummary>>,
    /// Cache warning indicator (if any)
    pub cache_warning: Option<CacheWarning>,
}

/// Unified data loading service
///
/// Provides cache-first loading strategy:
/// - Warm path: uses cached summaries + parses only recent files
/// - Cold path: full parse, builds cache for next run
pub struct DataLoaderService {
    registry: ParserRegistry,
    cache_service: Option<DailySummaryCacheService>,
    pricing: Option<PricingService>,
}

impl DataLoaderService {
    /// Create a new data loader service
    pub fn new() -> Self {
        Self {
            registry: ParserRegistry::new(),
            cache_service: DailySummaryCacheService::new().ok(),
            pricing: PricingService::from_cache_only(),
        }
    }

    /// Create a data loader with default local sources plus additional sources.
    pub fn with_extra_sources(extra_sources: Vec<SourceInstance>) -> Self {
        Self {
            registry: ParserRegistry::with_extra_sources(extra_sources),
            cache_service: DailySummaryCacheService::new().ok(),
            pricing: PricingService::from_cache_only(),
        }
    }

    /// Load data from all parsers using cache-first strategy
    pub fn load(&self) -> Result<LoadResult> {
        if self.has_valid_cache() {
            if let Ok(result) = self.load_warm_path() {
                if !result.summaries.is_empty() {
                    return Ok(result);
                }
            }
        }

        self.load_cold_path()
    }

    /// Check if any source has a valid (version-matching) cache
    fn has_valid_cache(&self) -> bool {
        self.cache_service.as_ref().is_some_and(|cs| {
            self.registry
                .sources()
                .iter()
                .any(|source| cs.is_version_current(&source.id))
        })
    }

    /// Warm path: use cached DailySummaries + parse only recent files
    fn load_warm_path(&self) -> Result<LoadResult> {
        let cache_service = self
            .cache_service
            .as_ref()
            .ok_or_else(|| ToktrackError::Cache("No cache service".into()))?;

        let since = warm_path_since();
        let yesterday = Local::now().date_naive() - chrono::Duration::days(1);

        let mut all_summaries = Vec::new();
        let mut source_stats: HashMap<String, (u64, f64)> = HashMap::new();
        let mut source_estimated: HashMap<String, bool> = HashMap::new();
        let mut source_summaries: HashMap<String, Vec<DailySummary>> = HashMap::new();
        let mut cache_warning = None;

        for source in self.registry.sources() {
            let parser = source.parser.as_ref();
            debug_assert_eq!(source.kind, parser.name());
            let has_source_cache = cache_service.cache_path(&source.id).exists();

            let entries = if has_source_cache {
                let latest = cache_service.latest_cached_date(&source.id);
                if has_date_gap(latest, yesterday) {
                    // Gap detected: full re-parse to fill missing dates
                    match parser.parse_all() {
                        Ok(e) => e,
                        Err(e) => {
                            eprintln!("[toktrack] Warning: {} failed: {}", source.label, e);
                            continue;
                        }
                    }
                } else {
                    match parser.parse_recent_files(since) {
                        Ok(e) => e
                            .into_iter()
                            .filter(|entry| entry.local_date() >= yesterday)
                            .collect(),
                        Err(e) => {
                            eprintln!("[toktrack] Warning: {} failed: {}", source.label, e);
                            continue;
                        }
                    }
                }
            } else {
                match parser.parse_all() {
                    Ok(e) => e,
                    Err(e) => {
                        eprintln!("[toktrack] Warning: {} failed: {}", source.label, e);
                        continue;
                    }
                }
            };

            let est = Self::batch_estimated(&entries);
            source_estimated
                .entry(source.id.clone())
                .and_modify(|e| *e |= est)
                .or_insert(est);
            let entries = Self::assign_source_id(entries, &source.id);
            let entries = self.apply_pricing(entries);

            match cache_service.load_or_compute(&source.id, &entries) {
                Ok((summaries, warning)) => {
                    if warning.is_some() && cache_warning.is_none() {
                        cache_warning = warning;
                    }
                    self.collect_source_stats(&summaries, &source.id, &mut source_stats);
                    source_summaries
                        .entry(source.id.clone())
                        .or_default()
                        .extend(summaries.iter().cloned());
                    all_summaries.extend(summaries);
                }
                Err(e) => {
                    eprintln!(
                        "[toktrack] Warning: cache for {} failed: {}",
                        source.label, e
                    );
                }
            }
        }

        let all_summaries = Aggregator::merge_by_date(all_summaries);
        let mut source_usage = Self::build_source_usage(source_stats, &source_estimated);
        source_usage.extend(Self::antigravity_notice());

        Ok(LoadResult {
            summaries: all_summaries,
            source_usage,
            source_summaries,
            cache_warning,
        })
    }

    /// Cold path: full parse_all() per source + build cache
    fn load_cold_path(&self) -> Result<LoadResult> {
        // Try network pricing if cache-only failed
        let fallback_pricing;
        let pricing_ref = match &self.pricing {
            Some(p) => Some(p),
            None => {
                fallback_pricing = PricingService::new().ok();
                fallback_pricing.as_ref()
            }
        };

        let mut all_summaries = Vec::new();
        let mut source_stats: HashMap<String, (u64, f64)> = HashMap::new();
        let mut source_estimated: HashMap<String, bool> = HashMap::new();
        let mut source_summaries: HashMap<String, Vec<DailySummary>> = HashMap::new();
        let mut cache_warning = None;
        let mut any_entries = false;

        for source in self.registry.sources() {
            let parser = source.parser.as_ref();
            debug_assert_eq!(source.kind, parser.name());
            let entries = match parser.parse_all() {
                Ok(e) => e,
                Err(e) => {
                    eprintln!("[toktrack] Warning: {} failed: {}", source.label, e);
                    continue;
                }
            };

            if entries.is_empty() {
                continue;
            }
            any_entries = true;

            let est = Self::batch_estimated(&entries);
            source_estimated
                .entry(source.id.clone())
                .and_modify(|e| *e |= est)
                .or_insert(est);
            let entries = Self::assign_source_id(entries, &source.id);
            let entries = self.apply_pricing_with_ref(entries, pricing_ref);

            // Try to use cache service
            if let Some(cs) = &self.cache_service {
                match cs.load_or_compute(&source.id, &entries) {
                    Ok((summaries, warning)) => {
                        if warning.is_some() && cache_warning.is_none() {
                            cache_warning = warning;
                        }
                        self.collect_source_stats(&summaries, &source.id, &mut source_stats);
                        source_summaries
                            .entry(source.id.clone())
                            .or_default()
                            .extend(summaries.iter().cloned());
                        all_summaries.extend(summaries);
                        continue;
                    }
                    Err(e) => {
                        eprintln!(
                            "[toktrack] Warning: cache for {} failed: {}",
                            source.label, e
                        );
                    }
                }
            }

            // Cache unavailable: compute summaries directly
            let summaries = Aggregator::daily(&entries);
            self.collect_source_stats(&summaries, &source.id, &mut source_stats);
            source_summaries
                .entry(source.id.clone())
                .or_default()
                .extend(summaries.iter().cloned());
            all_summaries.extend(summaries);
        }

        if !any_entries {
            return Err(ToktrackError::Parse(
                "No usage data found from any CLI".into(),
            ));
        }

        let all_summaries = Aggregator::merge_by_date(all_summaries);
        let mut source_usage = Self::build_source_usage(source_stats, &source_estimated);
        source_usage.extend(Self::antigravity_notice());

        Ok(LoadResult {
            summaries: all_summaries,
            source_usage,
            source_summaries,
            cache_warning,
        })
    }

    /// Apply pricing to entries using cached pricing service
    fn apply_pricing(&self, entries: Vec<UsageEntry>) -> Vec<UsageEntry> {
        self.apply_pricing_with_ref(entries, self.pricing.as_ref())
    }

    /// Whether a batch of entries is "estimated" — at least one non-Copilot entry
    /// had no upstream cost, so its cost will be LiteLLM-calculated.
    fn batch_estimated(entries: &[UsageEntry]) -> bool {
        entries
            .iter()
            .any(|e| e.cost_usd.is_none() && !is_copilot_provider(e.provider.as_deref()))
    }

    /// Override parser-provided source names with the concrete source id.
    fn assign_source_id(entries: Vec<UsageEntry>, source_id: &str) -> Vec<UsageEntry> {
        entries
            .into_iter()
            .map(|mut entry| {
                entry.source = Some(source_id.to_string());
                entry
            })
            .collect()
    }

    /// Apply pricing to entries using the given pricing service reference
    fn apply_pricing_with_ref(
        &self,
        entries: Vec<UsageEntry>,
        pricing: Option<&PricingService>,
    ) -> Vec<UsageEntry> {
        entries
            .into_iter()
            .map(|mut entry| {
                // GitHub Copilot is free, override cost to 0
                if is_copilot_provider(entry.provider.as_deref()) {
                    entry.cost_usd = Some(0.0);
                } else if entry.cost_usd.is_none() {
                    if let Some(p) = pricing {
                        entry.cost_usd = Some(p.calculate_cost(&entry));
                    }
                }
                entry
            })
            .collect()
    }

    /// Collect source statistics from summaries
    fn collect_source_stats(
        &self,
        summaries: &[DailySummary],
        source_name: &str,
        stats: &mut HashMap<String, (u64, f64)>,
    ) {
        for s in summaries {
            let tokens = s.total_input_tokens
                + s.total_output_tokens
                + s.total_cache_read_tokens
                + s.total_cache_creation_tokens
                + s.total_reasoning_tokens;
            let stat = stats.entry(source_name.to_string()).or_default();
            stat.0 = stat.0.saturating_add(tokens);
            stat.1 += s.total_cost_usd;
        }
    }

    /// Convert source stats map to sorted SourceUsage vector
    fn build_source_usage(
        source_stats: HashMap<String, (u64, f64)>,
        estimated: &HashMap<String, bool>,
    ) -> Vec<SourceUsage> {
        let mut result: Vec<SourceUsage> = source_stats
            .into_iter()
            .map(|(source, (total_tokens, total_cost_usd))| SourceUsage {
                supported: true,
                estimated: estimated.get(&source).copied().unwrap_or(false),
                source,
                total_tokens,
                total_cost_usd,
            })
            .collect();
        // Sort by total_tokens descending
        result.sort_by_key(|b| std::cmp::Reverse(b.total_tokens));
        result
    }

    /// Detect an Antigravity CLI install (`<gemini home>/.gemini/antigravity-cli`).
    /// It is unsupported — Antigravity does not write file-readable token usage —
    /// so surface a disabled source row (shown in the Sources list) rather than
    /// silence. No stderr notice: it would bleed over the TUI while it renders.
    fn antigravity_notice() -> Option<SourceUsage> {
        let base = crate::parsers::discovery::first_env_dir(&["GEMINI_CLI_HOME"])
            .or_else(|| directories::BaseDirs::new().map(|d| d.home_dir().to_path_buf()))?;
        Self::antigravity_notice_at(&base)
    }

    /// Detection split out for testing without touching the global `GEMINI_CLI_HOME`.
    fn antigravity_notice_at(home_base: &std::path::Path) -> Option<SourceUsage> {
        if home_base.join(".gemini").join("antigravity-cli").exists() {
            Some(SourceUsage {
                source: "antigravity".into(),
                total_tokens: 0,
                total_cost_usd: 0.0,
                supported: false,
                estimated: false,
            })
        } else {
            None
        }
    }
}

impl Default for DataLoaderService {
    fn default() -> Self {
        Self::new()
    }
}

/// Check if provider is GitHub Copilot (free service)
pub fn is_copilot_provider(provider: Option<&str>) -> bool {
    matches!(
        provider,
        Some("github-copilot") | Some("github-copilot-enterprise")
    )
}

#[cfg(test)]
mod tests {
    use crate::parsers::{CodexParser, SourceInstance};

    use super::*;
    use std::path::PathBuf;

    // ========== is_copilot_provider tests ==========

    #[test]
    fn test_is_copilot_provider_github_copilot() {
        assert!(is_copilot_provider(Some("github-copilot")));
    }

    #[test]
    fn test_is_copilot_provider_github_copilot_enterprise() {
        assert!(is_copilot_provider(Some("github-copilot-enterprise")));
    }

    #[test]
    fn test_is_copilot_provider_anthropic() {
        assert!(!is_copilot_provider(Some("anthropic")));
    }

    #[test]
    fn test_is_copilot_provider_openai() {
        assert!(!is_copilot_provider(Some("openai")));
    }

    #[test]
    fn test_is_copilot_provider_none() {
        assert!(!is_copilot_provider(None));
    }

    #[test]
    fn test_is_copilot_provider_empty_string() {
        assert!(!is_copilot_provider(Some("")));
    }

    // ========== Antigravity detection ==========

    #[test]
    fn test_antigravity_notice_detected_as_unsupported() {
        let tmp = tempfile::TempDir::new().unwrap();
        std::fs::create_dir_all(tmp.path().join(".gemini").join("antigravity-cli")).unwrap();
        let notice =
            DataLoaderService::antigravity_notice_at(tmp.path()).expect("dir present → notice");
        assert_eq!(notice.source, "antigravity");
        assert!(!notice.supported);
        assert_eq!(notice.total_tokens, 0);
        assert_eq!(notice.total_cost_usd, 0.0);
    }

    #[test]
    fn test_antigravity_notice_absent_when_no_dir() {
        let tmp = tempfile::TempDir::new().unwrap();
        assert!(DataLoaderService::antigravity_notice_at(tmp.path()).is_none());
    }

    // ========== build_source_usage tests ==========

    #[test]
    fn test_build_source_usage_empty() {
        let stats = HashMap::new();
        let result = DataLoaderService::build_source_usage(stats, &HashMap::new());
        assert!(result.is_empty());
    }

    #[test]
    fn test_build_source_usage_single_source() {
        let mut stats = HashMap::new();
        stats.insert("claude".to_string(), (1000u64, 0.05f64));
        let estimated = HashMap::from([("claude".to_string(), true)]);

        let result = DataLoaderService::build_source_usage(stats, &estimated);

        assert_eq!(result.len(), 1);
        assert_eq!(result[0].source, "claude");
        assert_eq!(result[0].total_tokens, 1000);
        assert!((result[0].total_cost_usd - 0.05).abs() < f64::EPSILON);
        assert!(result[0].supported);
        assert!(
            result[0].estimated,
            "estimated flag should flow from the map"
        );
    }

    #[test]
    fn test_batch_estimated() {
        use chrono::Utc;
        let mk = |cost: Option<f64>, provider: Option<&str>| UsageEntry {
            timestamp: Utc::now(),
            model: Some("m".into()),
            input_tokens: 1,
            output_tokens: 1,
            cache_read_tokens: 0,
            cache_creation_tokens: 0,
            reasoning_tokens: 0,
            cache_creation_5m_tokens: 0,
            cache_creation_1h_tokens: 0,
            web_search_requests: 0,
            web_fetch_requests: 0,
            reported_total_tokens: None,
            cost_usd: cost,
            message_id: None,
            request_id: None,
            source: None,
            provider: provider.map(String::from),
        };
        // upstream cost present → not estimated
        assert!(!DataLoaderService::batch_estimated(&[mk(Some(0.1), None)]));
        // calculated (no upstream cost) → estimated
        assert!(DataLoaderService::batch_estimated(&[mk(None, None)]));
        // copilot without cost → free, not an estimate
        assert!(!DataLoaderService::batch_estimated(&[mk(
            None,
            Some("github-copilot")
        )]));
    }

    #[test]
    fn test_build_source_usage_sorted_by_tokens_descending() {
        let mut stats = HashMap::new();
        stats.insert("claude".to_string(), (500u64, 0.03f64));
        stats.insert("opencode".to_string(), (2000u64, 0.10f64));
        stats.insert("gemini".to_string(), (1000u64, 0.05f64));

        let result = DataLoaderService::build_source_usage(stats, &HashMap::new());

        assert_eq!(result.len(), 3);
        assert_eq!(result[0].source, "opencode");
        assert_eq!(result[0].total_tokens, 2000);
        assert_eq!(result[1].source, "gemini");
        assert_eq!(result[1].total_tokens, 1000);
        assert_eq!(result[2].source, "claude");
        assert_eq!(result[2].total_tokens, 500);
    }

    // ========== warm_path_since tests ==========

    use chrono::Timelike;

    #[test]
    fn test_warm_path_since_is_start_of_yesterday_local() {
        let since = warm_path_since();
        let since_duration = since
            .duration_since(std::time::SystemTime::UNIX_EPOCH)
            .unwrap();
        let since_secs = since_duration.as_secs() as i64;

        // Expected: yesterday 00:00:00 in local timezone
        let yesterday = chrono::Local::now().date_naive() - chrono::Duration::days(1);
        let yesterday_midnight = yesterday.and_hms_opt(0, 0, 0).unwrap();
        let expected_utc = chrono::Local
            .from_local_datetime(&yesterday_midnight)
            .earliest()
            .unwrap()
            .to_utc();
        let expected_secs = expected_utc.timestamp();

        assert_eq!(since_secs, expected_secs);
    }

    #[test]
    fn test_warm_path_since_is_before_now() {
        let since = warm_path_since();
        assert!(since < std::time::SystemTime::now());
    }

    #[test]
    fn test_warm_path_since_is_at_midnight() {
        let since = warm_path_since();
        let since_duration = since
            .duration_since(std::time::SystemTime::UNIX_EPOCH)
            .unwrap();
        let since_secs = since_duration.as_secs() as i64;

        let dt = chrono::DateTime::from_timestamp(since_secs, 0).unwrap();
        let local_dt = dt.with_timezone(&chrono::Local);
        // Must be exactly 00:00:00 in local time
        assert_eq!(local_dt.hour(), 0);
        assert_eq!(local_dt.minute(), 0);
        assert_eq!(local_dt.second(), 0);
    }

    // ========== DataLoaderService::new tests ==========

    #[test]
    fn test_data_loader_service_new() {
        let service = DataLoaderService::new();
        // Just verify it can be constructed
        assert!(!service.registry.sources().is_empty());
    }

    #[test]
    fn test_data_loader_service_default() {
        let service = DataLoaderService::default();
        assert!(!service.registry.sources().is_empty());
    }

    #[test]
    fn test_load_uses_source_id_for_cache_and_source_breakdown() {
        let temp_dir = tempfile::tempdir().unwrap();
        let cache_dir = temp_dir.path().join("cache");
        let pricing_path = temp_dir.path().join("pricing.json");
        let fetched_at = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        std::fs::write(
            &pricing_path,
            format!(r#"{{"fetched_at":{},"models":{{}}}}"#, fetched_at),
        )
        .unwrap();

        let service = DataLoaderService {
            registry: ParserRegistry::with_sources(vec![
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
            ]),
            cache_service: Some(DailySummaryCacheService::with_cache_dir(cache_dir.clone())),
            pricing: PricingService::from_cache_only_with_path(&pricing_path),
        };

        let result = service.load().unwrap();

        assert!(cache_dir.join("codex_daily.json").exists());
        assert!(cache_dir.join("codex@testbox_daily.json").exists());
        assert!(result.source_summaries.contains_key("codex"));
        assert!(result.source_summaries.contains_key("codex@testbox"));

        let sources: std::collections::HashSet<&str> = result
            .source_usage
            .iter()
            .map(|usage| usage.source.as_str())
            .collect();
        assert!(sources.contains("codex"));
        assert!(sources.contains("codex@testbox"));
    }

    // ========== apply_pricing tests ==========

    fn make_entry(cost_usd: Option<f64>, provider: Option<&str>) -> UsageEntry {
        UsageEntry {
            timestamp: chrono::Utc::now(),
            model: Some("claude-sonnet-4-5-20250514".to_string()),
            input_tokens: 1000,
            output_tokens: 500,
            cache_read_tokens: 0,
            cache_creation_tokens: 0,
            reasoning_tokens: 0,
            cache_creation_5m_tokens: 0,
            cache_creation_1h_tokens: 0,
            web_search_requests: 0,
            web_fetch_requests: 0,
            reported_total_tokens: None,
            cost_usd,
            message_id: None,
            request_id: None,
            source: None,
            provider: provider.map(|s| s.to_string()),
        }
    }

    #[test]
    fn test_apply_pricing_zero_cost_is_trusted() {
        let service = DataLoaderService::new();
        let entries = vec![make_entry(Some(0.0), Some("anthropic"))];
        let result = service.apply_pricing(entries);
        // Some(0.0) is a legitimate cost (e.g. free-tier providers) — trust it as-is
        assert_eq!(result[0].cost_usd, Some(0.0));
    }

    #[test]
    fn test_apply_pricing_none_cost_triggers_recalculation() {
        let service = DataLoaderService::new();
        let entries = vec![make_entry(None, Some("anthropic"))];
        let result = service.apply_pricing(entries);
        // None should trigger recalculation
        assert_ne!(result[0].cost_usd, None);
    }

    #[test]
    fn test_apply_pricing_nonzero_cost_preserved() {
        let service = DataLoaderService::new();
        let entries = vec![make_entry(Some(0.05), Some("anthropic"))];
        let result = service.apply_pricing(entries);
        assert_eq!(result[0].cost_usd, Some(0.05));
    }

    #[test]
    fn test_apply_pricing_copilot_zero_cost() {
        let service = DataLoaderService::new();
        let entries = vec![make_entry(Some(0.10), Some("github-copilot"))];
        let result = service.apply_pricing(entries);
        // Copilot should always be $0 regardless of original cost
        assert_eq!(result[0].cost_usd, Some(0.0));
    }

    // ========== warm path filtering tests ==========

    #[test]
    fn test_warm_path_filters_old_date_entries() {
        // Simulate cross-day JSONL: a file modified today that contains
        // entries spanning multiple days (e.g., a long session from 5 days ago).
        // Only yesterday and today entries should survive warm-path filtering.
        let today = Local::now().date_naive();
        let yesterday = today - chrono::Duration::days(1);
        let five_days_ago = today - chrono::Duration::days(5);

        let old_entry = UsageEntry {
            timestamp: five_days_ago.and_hms_opt(23, 0, 0).unwrap().and_utc(),
            model: Some("claude".to_string()),
            input_tokens: 50_000_000,
            output_tokens: 10_000_000,
            cache_read_tokens: 0,
            cache_creation_tokens: 0,
            reasoning_tokens: 0,
            cache_creation_5m_tokens: 0,
            cache_creation_1h_tokens: 0,
            web_search_requests: 0,
            web_fetch_requests: 0,
            reported_total_tokens: None,
            cost_usd: Some(100.0),
            message_id: None,
            request_id: None,
            source: None,
            provider: None,
        };
        let yesterday_entry = UsageEntry {
            timestamp: yesterday.and_hms_opt(12, 0, 0).unwrap().and_utc(),
            model: Some("claude".to_string()),
            input_tokens: 1000,
            output_tokens: 500,
            cache_read_tokens: 0,
            cache_creation_tokens: 0,
            reasoning_tokens: 0,
            cache_creation_5m_tokens: 0,
            cache_creation_1h_tokens: 0,
            web_search_requests: 0,
            web_fetch_requests: 0,
            reported_total_tokens: None,
            cost_usd: Some(0.01),
            message_id: None,
            request_id: None,
            source: None,
            provider: None,
        };
        let today_entry = UsageEntry {
            timestamp: today.and_hms_opt(10, 0, 0).unwrap().and_utc(),
            model: Some("claude".to_string()),
            input_tokens: 2000,
            output_tokens: 1000,
            cache_read_tokens: 0,
            cache_creation_tokens: 0,
            reasoning_tokens: 0,
            cache_creation_5m_tokens: 0,
            cache_creation_1h_tokens: 0,
            web_search_requests: 0,
            web_fetch_requests: 0,
            reported_total_tokens: None,
            cost_usd: Some(0.02),
            message_id: None,
            request_id: None,
            source: None,
            provider: None,
        };

        let all_entries = vec![old_entry, yesterday_entry, today_entry];

        // Apply the same filter used in load_warm_path
        let filtered: Vec<UsageEntry> = all_entries
            .into_iter()
            .filter(|entry| entry.local_date() >= yesterday)
            .collect();

        assert_eq!(filtered.len(), 2);
        assert_eq!(filtered[0].local_date(), yesterday);
        assert_eq!(filtered[1].local_date(), today);
        // The old entry with massive tokens should be gone
        assert!(filtered.iter().all(|e| e.input_tokens < 50_000_000));
    }

    // ========== gap detection tests ==========

    #[test]
    fn test_has_date_gap_no_gap() {
        // latest_cached = yesterday - 1 → warm path covers [yesterday, today] → no gap
        let today = Local::now().date_naive();
        let latest_cached = today - chrono::Duration::days(2); // day before yesterday
        let yesterday = today - chrono::Duration::days(1);
        assert!(!has_date_gap(Some(latest_cached), yesterday));
    }

    #[test]
    fn test_has_date_gap_with_gap() {
        // latest_cached = yesterday - 2 → gap of 1 day between cache and warm path
        let today = Local::now().date_naive();
        let latest_cached = today - chrono::Duration::days(3); // 2 days before yesterday
        let yesterday = today - chrono::Duration::days(1);
        assert!(has_date_gap(Some(latest_cached), yesterday));
    }

    #[test]
    fn test_has_date_gap_none_latest() {
        // No cached date → no gap to detect (cold path handles this)
        let today = Local::now().date_naive();
        let yesterday = today - chrono::Duration::days(1);
        assert!(!has_date_gap(None, yesterday));
    }

    #[test]
    fn test_has_date_gap_latest_is_yesterday() {
        // latest_cached = yesterday → warm path covers [yesterday, today] → no gap
        let today = Local::now().date_naive();
        let yesterday = today - chrono::Duration::days(1);
        assert!(!has_date_gap(Some(yesterday), yesterday));
    }
}
