//! Unified data loading service for CLI and TUI
//!
//! This module provides a single `DataLoaderService` that consolidates
//! the duplicated data loading logic from CLI and TUI.

use std::collections::HashMap;
use std::time::SystemTime;

use chrono::{Local, NaiveDate, TimeZone};

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

/// Warm-path guard: whether a recent-parse result must fall back to a full
/// re-parse instead of the `>= yesterday` filter.
///
/// True only when the source reconciles retroactively (see
/// `CLIParser::retroactive_reconciliation`) and the recent parse produced an
/// entry dated before `yesterday`. Keeping such an entry and recomputing its day
/// from the recent files alone would drop that day's entries in older,
/// non-recent files; dropping it leaves the day's cache stale. A full re-parse
/// recomputes the day from the complete file set instead.
fn needs_full_reparse(retroactive: bool, entries: &[UsageEntry], yesterday: NaiveDate) -> bool {
    retroactive && entries.iter().any(|e| e.local_date() < yesterday)
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
            // A cache is only usable on the warm (recent-only) path when it both
            // exists AND matches the current CACHE_VERSION. A stale-version cache
            // must be fully re-parsed so schema changes (e.g. the per-project
            // breakdown) are backfilled for every date whose raw files still
            // exist — `load_or_compute` still merges in older cache-only days, so
            // preserved >30-day history is kept. Without this, a stale source is
            // never refreshed as long as any *other* source has a current cache
            // (which keeps the global warm path active).
            let has_source_cache = cache_service.cache_path(&source.id).exists()
                && cache_service.is_version_current(&source.id);

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
                        Ok(e) => {
                            if needs_full_reparse(
                                parser.retroactive_reconciliation(),
                                &e,
                                yesterday,
                            ) {
                                match parser.parse_all() {
                                    Ok(all) => all,
                                    Err(err) => {
                                        eprintln!(
                                            "[toktrack] Warning: {} failed: {}",
                                            source.label, err
                                        );
                                        continue;
                                    }
                                }
                            } else {
                                e.into_iter()
                                    .filter(|entry| entry.local_date() >= yesterday)
                                    .collect()
                            }
                        }
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
        let source_usage = Self::build_source_usage(source_stats, &source_estimated);

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
        let source_usage = Self::build_source_usage(source_stats, &source_estimated);

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

    /// Whether a batch of entries is "estimated" — at least one entry had no
    /// upstream cost, so its cost will be LiteLLM-calculated (this includes
    /// Copilot, whose subscription cost is estimated from model pricing).
    fn batch_estimated(entries: &[UsageEntry]) -> bool {
        entries.iter().any(|e| e.cost_usd.is_none())
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
                // Copilot usage is metered through a subscription rather than
                // per-token, but we still surface an *estimated* cost using the
                // model's LiteLLM pricing so totals are meaningful. Entries that
                // already carry a cost (e.g. parsed from JSONL) are trusted.
                if entry.cost_usd.is_none() {
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
}

impl Default for DataLoaderService {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use crate::parsers::{CodexParser, SourceInstance};

    use super::*;
    use std::path::PathBuf;

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
            project: None,
        };
        // upstream cost present → not estimated
        assert!(!DataLoaderService::batch_estimated(&[mk(Some(0.1), None)]));
        // calculated (no upstream cost) → estimated
        assert!(DataLoaderService::batch_estimated(&[mk(None, None)]));
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

    #[test]
    fn test_stale_version_source_is_reparsed_and_preserves_history() {
        // A source whose cache is STALE-versioned must be fully re-parsed (so
        // schema changes like the per-project breakdown are backfilled), while
        // older cache-only days are preserved — even when another source keeps
        // the global warm path active. The stale cache's latest date is recent
        // (no date gap), so the ONLY thing that can trigger the reparse is the
        // version check added to the warm path.
        let temp_dir = tempfile::tempdir().unwrap();
        let cache_dir = temp_dir.path().join("cache");
        std::fs::create_dir_all(&cache_dir).unwrap();
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

        let cache_service = DailySummaryCacheService::with_cache_dir(cache_dir.clone());

        // Seed a CURRENT-version cache for a second source so has_valid_cache()
        // is true and load() takes the warm path.
        let today = Local::now().date_naive();
        let mut keepwarm_entry = make_entry(Some(0.01), Some("openai"));
        keepwarm_entry.timestamp = today.and_hms_opt(12, 0, 0).unwrap().and_utc();
        cache_service
            .load_or_compute("keepwarm", &[keepwarm_entry])
            .unwrap();

        // Seed a STALE (version 0) codex cache: an ancient cache-only day (no raw
        // backing → preserved history) + a recent day so latest_cached_date is
        // recent (no gap).
        let yesterday = today - chrono::Duration::days(1);
        let stale = format!(
            r#"{{"cli":"codex","version":0,"updated_at":0,"summaries":[
                {{"date":"2020-01-01","total_input_tokens":111,"total_output_tokens":0,
                  "total_cache_read_tokens":0,"total_cache_creation_tokens":0,
                  "total_cost_usd":0.0,"models":{{}},"projects":{{}}}},
                {{"date":"{}","total_input_tokens":5,"total_output_tokens":5,
                  "total_cache_read_tokens":0,"total_cache_creation_tokens":0,
                  "total_cost_usd":0.0,"models":{{}},"projects":{{}}}}
            ]}}"#,
            yesterday
        );
        std::fs::write(cache_dir.join("codex_daily.json"), stale).unwrap();

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
                    "keepwarm",
                    "keepwarm",
                    "codex",
                    Box::new(CodexParser::with_data_dir(temp_dir.path().to_path_buf())),
                ),
            ]),
            cache_service: Some(DailySummaryCacheService::with_cache_dir(cache_dir.clone())),
            pricing: PricingService::from_cache_only_with_path(&pricing_path),
        };

        let result = service.load().unwrap();
        let codex = result
            .source_summaries
            .get("codex")
            .expect("codex summaries present");
        let dates: Vec<String> = codex.iter().map(|s| s.date.to_string()).collect();

        // Ancient cache-only day preserved across the reparse.
        assert!(
            dates.iter().any(|d| d == "2020-01-01"),
            "preserved >30-day history was dropped: {dates:?}"
        );
        // Fixture dates re-parsed — proves the stale source was fully re-parsed
        // rather than served recent-only.
        assert!(
            dates.iter().any(|d| d.starts_with("2026-01")),
            "stale-version source was not re-parsed (no backfill): {dates:?}"
        );
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
            project: None,
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
    fn test_apply_pricing_copilot_estimated_cost() {
        let service = DataLoaderService::new();
        // Copilot entries without a parsed cost get an estimated cost from the
        // model's LiteLLM pricing (e.g. gpt-5.3-codex) instead of being forced to $0.
        let mut entry = make_entry(None, Some("github-copilot"));
        entry.model = Some("gpt-5.3-codex".to_string());
        let result = service.apply_pricing(vec![entry]);
        assert_ne!(result[0].cost_usd, Some(0.0));
        assert_ne!(result[0].cost_usd, None);
    }

    #[test]
    fn test_apply_pricing_copilot_existing_cost_preserved() {
        let service = DataLoaderService::new();
        // A cost parsed from JSONL is trusted as-is.
        let entries = vec![make_entry(Some(0.10), Some("github-copilot"))];
        let result = service.apply_pricing(entries);
        assert_eq!(result[0].cost_usd, Some(0.10));
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
            project: None,
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
            project: None,
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
            project: None,
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

    #[test]
    fn test_needs_full_reparse_only_retroactive_source_with_old_entry() {
        let today = Local::now().date_naive();
        let yesterday = today - chrono::Duration::days(1);
        let two_days_ago = today - chrono::Duration::days(2);

        let dated = |d: NaiveDate| UsageEntry {
            timestamp: d.and_hms_opt(12, 0, 0).unwrap().and_utc(),
            model: Some("gpt-5.3-codex".to_string()),
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
            cost_usd: None,
            message_id: None,
            request_id: None,
            source: Some("copilot".to_string()),
            provider: Some("github-copilot".to_string()),
            project: None,
        };

        let old = dated(two_days_ago);
        let recent = dated(yesterday);

        // Non-retroactive source never full-reparses: a long Claude/Codex session
        // whose old-dated entries are already final must stay on the fast path.
        assert!(!needs_full_reparse(
            false,
            std::slice::from_ref(&old),
            yesterday
        ));
        // Retroactive source with only in-window entries: normal filter path.
        assert!(!needs_full_reparse(
            true,
            std::slice::from_ref(&recent),
            yesterday
        ));
        // Retroactive source with a retroactively-dated entry: full re-parse so
        // the old day is recomputed from the complete file set, not clobbered.
        assert!(needs_full_reparse(true, &[old, recent], yesterday));
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
