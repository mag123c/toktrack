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

/// Warm-path guard: whether a recent-parse result must fall back to a full
/// re-parse instead of the `>= recompute_start` filter.
///
/// True only when the source reconciles retroactively (see
/// `CLIParser::retroactive_reconciliation`) and the recent parse produced an
/// entry dated before `recompute_start`, the first date the warm path
/// recomputes (see `warm_path_recompute_start`). Keeping such an entry and
/// recomputing its day from the recent files alone would drop that day's
/// entries in older, non-recent files; dropping it leaves the day's cache
/// stale. A full re-parse recomputes the day from the complete file set instead.
fn needs_full_reparse(
    retroactive: bool,
    entries: &[UsageEntry],
    recompute_start: NaiveDate,
) -> bool {
    retroactive && entries.iter().any(|e| e.local_date() < recompute_start)
}

/// First date the warm path recomputes: the previous run's date, or yesterday
/// if that is earlier.
///
/// Every day before the previous run's date was already over when that run
/// saved the cache; the run's own date may be a partial-day snapshot. Any entry
/// dated on or after the start sits in a file modified on or after it, so
/// recomputing from there brings every later day up to date after a gap of any
/// length, without a full re-parse and without touching an older cached day.
/// (Gating on the latest cached date instead would mistake days without usage
/// for a gap.)
fn warm_path_recompute_start(last_run: NaiveDate, yesterday: NaiveDate) -> NaiveDate {
    last_run.min(yesterday)
}

/// Compute the warm-path cutoff: 00:00:00 local time on `start`.
///
/// Files modified on or after this time are re-parsed. Any entry dated on or
/// after `start` sits in a file modified on or after it, so every date from
/// `start` on is recomputed completely before being trusted as cached.
fn warm_path_since(start: NaiveDate) -> SystemTime {
    let start_midnight = start.and_hms_opt(0, 0, 0).unwrap();
    let utc = match Local.from_local_datetime(&start_midnight) {
        chrono::LocalResult::Single(dt) => dt.to_utc(),
        chrono::LocalResult::Ambiguous(earlier, _) => earlier.to_utc(),
        chrono::LocalResult::None => {
            // DST spring-forward: midnight doesn't exist, use 01:00
            let fallback = start.and_hms_opt(1, 0, 0).unwrap();
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
                // Without a previous-run date the warm path cannot know which
                // days are final: re-parse fully.
                if let Some(last_run) = cache_service.last_run_date(&source.id) {
                    let recompute_start = warm_path_recompute_start(last_run, yesterday);
                    match parser.parse_recent_files(warm_path_since(recompute_start)) {
                        Ok(e) => {
                            if needs_full_reparse(
                                parser.retroactive_reconciliation(),
                                &e,
                                recompute_start,
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
                                    .filter(|entry| entry.local_date() >= recompute_start)
                                    .collect()
                            }
                        }
                        Err(e) => {
                            eprintln!("[toktrack] Warning: {} failed: {}", source.label, e);
                            continue;
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
    use crate::parsers::{CLIParser, CodexParser, SourceInstance};

    use super::*;
    use std::path::PathBuf;

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
            fast_speed: false,
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
        assert!(!DataLoaderService::batch_estimated(&[mk(Some(0.1), None)]));
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

    use chrono::Timelike;

    #[test]
    fn test_warm_path_since_is_start_of_yesterday_local() {
        let since = warm_path_since(chrono::Local::now().date_naive() - chrono::Duration::days(1));
        let since_duration = since
            .duration_since(std::time::SystemTime::UNIX_EPOCH)
            .unwrap();
        let since_secs = since_duration.as_secs() as i64;

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
        let since = warm_path_since(chrono::Local::now().date_naive() - chrono::Duration::days(1));
        assert!(since < std::time::SystemTime::now());
    }

    #[test]
    fn test_warm_path_since_is_at_midnight() {
        let since = warm_path_since(chrono::Local::now().date_naive() - chrono::Duration::days(1));
        let since_duration = since
            .duration_since(std::time::SystemTime::UNIX_EPOCH)
            .unwrap();
        let since_secs = since_duration.as_secs() as i64;

        let dt = chrono::DateTime::from_timestamp(since_secs, 0).unwrap();
        let local_dt = dt.with_timezone(&chrono::Local);
        assert_eq!(local_dt.hour(), 0);
        assert_eq!(local_dt.minute(), 0);
        assert_eq!(local_dt.second(), 0);
    }

    #[test]
    fn test_data_loader_service_new() {
        let service = DataLoaderService::new();
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
        let keepwarm_dir = temp_dir.path().join("keepwarm");
        std::fs::create_dir_all(&keepwarm_dir).unwrap();
        let today = Local::now().date_naive();
        let mut keepwarm_entry = make_entry(Some(0.01), Some("openai"));
        keepwarm_entry.timestamp = today.and_hms_opt(12, 0, 0).unwrap().and_utc();
        cache_service
            .load_or_compute("keepwarm", &[keepwarm_entry])
            .unwrap();

        // Seed a STALE (version 0) codex cache: an ancient cache-only day (no raw
        // backing → preserved history) plus a recent day so the fixture matches a
        // real cache. The stale version alone routes this source to a full
        // reparse.
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
                    // Isolated: pointing this at the temp root would also glob
                    // the other source's fixtures into this source.
                    Box::new(CodexParser::with_data_dir(keepwarm_dir.clone())),
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

    #[test]
    fn test_v16_cache_backfills_archived_codex_sessions_and_preserves_history() {
        // Even with a current cache for another source, upgrade from v16 must
        // backfill old archive files and retain days with no remaining raw logs.
        let temp_dir = tempfile::tempdir().unwrap();
        let archived = temp_dir.path().join("archived_sessions");
        std::fs::create_dir_all(&archived).unwrap();
        std::fs::copy(
            "tests/fixtures/codex/cwd-session.jsonl",
            archived.join("archived.jsonl"),
        )
        .unwrap();
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
        let keepwarm_dir = temp_dir.path().join("keepwarm");
        std::fs::create_dir_all(&keepwarm_dir).unwrap();
        let today = Local::now().date_naive();
        let mut keepwarm_entry = make_entry(Some(0.01), Some("openai"));
        keepwarm_entry.timestamp = today.and_hms_opt(12, 0, 0).unwrap().and_utc();
        cache_service
            .load_or_compute("keepwarm", &[keepwarm_entry])
            .unwrap();

        // Seed a STALE (version 16) codex cache: an ancient cache-only day (no raw
        // backing → preserved history) plus a recent day so the fixture matches a
        // real cache. The stale version alone routes this source to a full
        // reparse.
        let yesterday = today - chrono::Duration::days(1);
        let stale = format!(
            r#"{{"cli":"codex","version":16,"updated_at":0,"summaries":[
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
                    Box::new(CodexParser::with_data_dir(temp_dir.path().join("sessions"))),
                ),
                SourceInstance::new(
                    "keepwarm",
                    "keepwarm",
                    "codex",
                    // Isolated: pointing this at the temp root would also glob
                    // the other source's fixtures into this source.
                    Box::new(CodexParser::with_data_dir(keepwarm_dir.clone())),
                ),
            ]),
            cache_service: Some(DailySummaryCacheService::with_cache_dir(cache_dir.clone())),
            pricing: PricingService::from_cache_only_with_path(&pricing_path),
        };

        // The control source must see no files of its own. Pointing it at the
        // temp root instead would sweep this test's archive fixture into it.
        assert!(service
            .registry
            .get("keepwarm")
            .expect("keepwarm source registered")
            .collect_files()
            .is_empty());

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
            dates.iter().any(|d| d.starts_with("2026-06")),
            "stale-version source was not re-parsed (no backfill): {dates:?}"
        );
    }

    #[test]
    fn test_codex_does_not_reconcile_retroactively() {
        // Deliberate, not an oversight: Codex sessions are resumed across days,
        // so a file touched today routinely carries entries dated before
        // yesterday and the flag would force a full reparse on nearly every
        // run. Those days are already summarized in the cache from when they
        // were current. See .dev/DECISIONS.md (2026-09-20) for the measurement.
        assert!(!CodexParser::new().retroactive_reconciliation());
    }

    fn make_entry(cost_usd: Option<f64>, provider: Option<&str>) -> UsageEntry {
        UsageEntry {
            fast_speed: false,
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
        assert_eq!(result[0].cost_usd, Some(0.0));
    }

    #[test]
    fn test_apply_pricing_none_cost_triggers_recalculation() {
        let service = DataLoaderService::new();
        let entries = vec![make_entry(None, Some("anthropic"))];
        let result = service.apply_pricing(entries);
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
        let entries = vec![make_entry(Some(0.10), Some("github-copilot"))];
        let result = service.apply_pricing(entries);
        assert_eq!(result[0].cost_usd, Some(0.10));
    }

    #[test]
    fn test_warm_path_filters_old_date_entries() {
        // Simulate cross-day JSONL: a file modified today that contains
        // entries spanning multiple days (e.g., a long session from 5 days ago).
        // Only yesterday and today entries should survive warm-path filtering.
        let today = Local::now().date_naive();
        let yesterday = today - chrono::Duration::days(1);
        let five_days_ago = today - chrono::Duration::days(5);

        let old_entry = UsageEntry {
            fast_speed: false,
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
            fast_speed: false,
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
            fast_speed: false,
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

        let filtered: Vec<UsageEntry> = all_entries
            .into_iter()
            .filter(|entry| entry.local_date() >= yesterday)
            .collect();

        assert_eq!(filtered.len(), 2);
        assert_eq!(filtered[0].local_date(), yesterday);
        assert_eq!(filtered[1].local_date(), today);
        assert!(filtered.iter().all(|e| e.input_tokens < 50_000_000));
    }

    #[test]
    fn test_needs_full_reparse_only_retroactive_source_with_old_entry() {
        let today = Local::now().date_naive();
        let yesterday = today - chrono::Duration::days(1);
        let two_days_ago = today - chrono::Duration::days(2);

        let dated = |d: NaiveDate| UsageEntry {
            fast_speed: false,
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

    #[test]
    fn test_warm_path_recompute_start_is_previous_run_capped_at_yesterday() {
        let today = Local::now().date_naive();
        let yesterday = today - chrono::Duration::days(1);
        let four_days_ago = today - chrono::Duration::days(4);
        assert_eq!(warm_path_recompute_start(today, yesterday), yesterday);
        assert_eq!(warm_path_recompute_start(yesterday, yesterday), yesterday);
        assert_eq!(
            warm_path_recompute_start(four_days_ago, yesterday),
            four_days_ago
        );
    }

    /// RFC 3339 UTC timestamp for local noon (+`second`) on `day`, so the entry's
    /// local date is `day` in every time zone.
    fn local_noon(day: NaiveDate, second: u32) -> String {
        Local
            .from_local_datetime(&day.and_hms_opt(12, 0, second).unwrap())
            .earliest()
            .unwrap()
            .to_utc()
            .to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
    }

    /// `(input, cached input, output)`
    type Usage = (u64, u64, u64);

    /// One Codex rollout with a token_count per `(day, total, last)`.
    fn codex_rollout(events: &[(NaiveDate, Usage, Usage)]) -> String {
        let usage = |(i, c, o): Usage| {
            format!(
                r#"{{"input_tokens":{i},"cached_input_tokens":{c},"output_tokens":{o},"reasoning_output_tokens":0,"total_tokens":{}}}"#,
                i + o
            )
        };
        let first = events[0].0;
        let mut lines = vec![
            format!(
                r#"{{"timestamp":"{}","type":"session_meta","payload":{{"id":"sess-a","model_provider":"openai"}}}}"#,
                local_noon(first, 0)
            ),
            format!(
                r#"{{"timestamp":"{}","type":"turn_context","payload":{{"model":"gpt-5.5","cwd":"/work/a"}}}}"#,
                local_noon(first, 1)
            ),
        ];
        for (n, (day, total, last)) in events.iter().enumerate() {
            lines.push(format!(
                r#"{{"timestamp":"{}","type":"event_msg","payload":{{"type":"token_count","info":{{"total_token_usage":{},"last_token_usage":{}}}}}}}"#,
                local_noon(*day, 2 + n as u32),
                usage(*total),
                usage(*last)
            ));
        }
        lines.join("\n")
    }

    /// Cache `cached` as `(day, input tokens)` summaries saved by a run at local
    /// noon on `last_run`, write `rollout`, then load once through the warm
    /// path and return the Codex daily summaries.
    fn load_codex_over_cache(
        rollout: &str,
        cached: &[(NaiveDate, u64)],
        last_run: NaiveDate,
    ) -> Vec<DailySummary> {
        let temp_dir = tempfile::tempdir().unwrap();
        let sessions = temp_dir.path().join("sessions");
        std::fs::create_dir_all(&sessions).unwrap();
        std::fs::write(sessions.join("a.jsonl"), rollout).unwrap();

        let cache_dir = temp_dir.path().join("cache");
        let entries: Vec<UsageEntry> = cached
            .iter()
            .map(|(day, input)| {
                let mut e = make_entry(Some(0.0), Some("openai"));
                e.timestamp = Local
                    .from_local_datetime(&day.and_hms_opt(12, 0, 0).unwrap())
                    .earliest()
                    .unwrap()
                    .to_utc();
                e.model = Some("gpt-5.5".to_string());
                e.input_tokens = *input;
                e.output_tokens = 0;
                e
            })
            .collect();
        let seed = DailySummaryCacheService::with_cache_dir(cache_dir.clone());
        seed.load_or_compute("codex", &entries).unwrap();
        let cache_path = seed.cache_path("codex");
        let mut cache: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&cache_path).unwrap()).unwrap();
        cache["updated_at"] = Local
            .from_local_datetime(&last_run.and_hms_opt(12, 0, 0).unwrap())
            .earliest()
            .unwrap()
            .timestamp()
            .into();
        std::fs::write(&cache_path, cache.to_string()).unwrap();

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
            registry: ParserRegistry::with_sources(vec![SourceInstance::new(
                "codex",
                "codex",
                "codex",
                Box::new(CodexParser::with_data_dir(sessions)),
            )]),
            cache_service: Some(DailySummaryCacheService::with_cache_dir(cache_dir)),
            pricing: PricingService::from_cache_only_with_path(&pricing_path),
        };
        service.load().unwrap().source_summaries["codex"].clone()
    }

    fn day_tokens(summaries: &[DailySummary], day: NaiveDate) -> u64 {
        let s = summaries.iter().find(|s| s.date == day).unwrap();
        s.total_input_tokens + s.total_cache_read_tokens + s.total_output_tokens
    }

    #[test]
    fn test_partial_latest_day_recomputed_when_next_run_is_two_days_later() {
        // A run on day D cached D after its first turn; the next run is on D+2.
        let d = Local::now().date_naive() - chrono::Duration::days(2);
        let rollout = codex_rollout(&[
            (d, (100, 10, 50), (100, 10, 50)), // 150 tokens
            (d, (130, 10, 60), (30, 0, 10)),   // 40 tokens, after the snapshot
        ]);
        let summaries = load_codex_over_cache(&rollout, &[(d, 150)], d);
        assert_eq!(day_tokens(&summaries, d), 190);
    }

    #[test]
    fn test_two_day_gap_keeps_older_cached_days() {
        // An older day whose logs are mostly gone: only one 150-token turn is
        // still on disk (in a long-running rollout that also covers D), while
        // the cache holds its complete 150,000. Recomputing D must not touch it.
        let d = Local::now().date_naive() - chrono::Duration::days(2);
        let older = d - chrono::Duration::days(3);
        let rollout = codex_rollout(&[
            (older, (150, 0, 0), (150, 0, 0)),
            (d, (250, 10, 50), (100, 10, 50)),
            (d, (280, 10, 60), (30, 0, 10)),
        ]);
        let summaries = load_codex_over_cache(&rollout, &[(older, 150_000), (d, 150)], d);
        assert_eq!(day_tokens(&summaries, d), 190);
        assert_eq!(day_tokens(&summaries, older), 150_000);
    }

    #[test]
    fn test_long_gap_recomputes_from_previous_run_and_keeps_older_days() {
        // The previous run was on D = today - 4 and cached D after its first
        // turn; usage continued on D, D+1 and D+2. An older day's logs are
        // mostly gone (150 of its 150,000 tokens are still on disk).
        let today = Local::now().date_naive();
        let d = today - chrono::Duration::days(4);
        let day = chrono::Duration::days(1);
        let older = d - day * 3;
        let rollout = codex_rollout(&[
            (older, (150, 0, 0), (150, 0, 0)),
            (d, (250, 10, 50), (100, 10, 50)),
            (d, (280, 10, 60), (30, 0, 10)),
            (d + day, (300, 10, 70), (20, 0, 10)),
            (d + day * 2, (400, 10, 80), (100, 0, 10)),
        ]);
        let summaries = load_codex_over_cache(&rollout, &[(older, 150_000), (d, 150)], d);
        assert_eq!(day_tokens(&summaries, d), 190);
        assert_eq!(day_tokens(&summaries, d + day), 30);
        assert_eq!(day_tokens(&summaries, d + day * 2), 110);
        assert_eq!(day_tokens(&summaries, older), 150_000);
    }

    #[test]
    fn test_days_without_usage_do_not_force_full_reparse() {
        // toktrack ran yesterday, but the latest day with usage is a week old
        // and its logs are mostly gone. Nothing older than yesterday may change.
        let today = Local::now().date_naive();
        let older = today - chrono::Duration::days(7);
        let rollout = codex_rollout(&[
            (older, (150, 0, 0), (150, 0, 0)),
            (today, (250, 0, 10), (100, 0, 10)),
        ]);
        let yesterday = today - chrono::Duration::days(1);
        let summaries = load_codex_over_cache(&rollout, &[(older, 150_000)], yesterday);
        assert_eq!(day_tokens(&summaries, older), 150_000);
        assert_eq!(day_tokens(&summaries, today), 110);
    }

    #[test]
    fn opencode_v2_backfills_v18_cache_without_losing_preserved_history() {
        use crate::parsers::OpenCodeParser;
        let tmp = tempfile::tempdir().unwrap();
        let cache_dir = tmp.path().join("cache");
        std::fs::create_dir_all(&cache_dir).unwrap();
        let yesterday = Local::now().date_naive() - chrono::Duration::days(1);
        let stale = serde_json::json!({
            "cli": "opencode", "version": 18, "updated_at": 0,
            "summaries": (["2020-01-01".to_owned(), yesterday.to_string()].map(|date| {
                serde_json::json!({"date": date, "total_input_tokens": 111,
                    "total_output_tokens": 0, "total_cache_read_tokens": 0,
                    "total_cache_creation_tokens": 0, "total_cost_usd": 0,
                    "models": {}, "projects": {}})
            }))
        });
        std::fs::write(cache_dir.join("opencode_daily.json"), stale.to_string()).unwrap();
        let conn = rusqlite::Connection::open(tmp.path().join("opencode.db")).unwrap();
        conn.execute_batch(r#"CREATE TABLE session_message (id TEXT, session_id TEXT,
            type TEXT, time_created INTEGER, data TEXT);
            INSERT INTO session_message VALUES ('m1', 's1', 'assistant', 1700000000000,
            '{"model":{"id":"mimo-v2.6-flash","providerID":"opencode-go"},
              "time":{"created":1700000000000},"tokens":{"input":100,"output":20},"cost":0.125}');"#
        ).unwrap();
        let service = DataLoaderService {
            registry: ParserRegistry::with_sources(vec![SourceInstance::new(
                "opencode",
                "opencode",
                "opencode",
                Box::new(OpenCodeParser::with_base_dir(tmp.path().into())),
            )]),
            cache_service: Some(DailySummaryCacheService::with_cache_dir(cache_dir.clone())),
            pricing: None,
        };
        // Exercise the warm path directly: v18 alone must force a full scan.
        let result = service.load_warm_path().unwrap();
        let summaries = &result.source_summaries["opencode"];
        assert_eq!(summaries.len(), 3);
        assert!(summaries
            .iter()
            .any(|s| s.date.to_string() == "2020-01-01" && s.total_input_tokens == 111));
        let backfill = summaries
            .iter()
            .find(|s| s.date.to_string().starts_with("2023-"))
            .unwrap();
        assert_eq!(backfill.total_input_tokens, 100);
        assert_eq!(backfill.total_output_tokens, 20);
        assert_eq!(backfill.total_cost_usd, 0.125);
        assert!(backfill.models.keys().any(|k| k.contains("mimo")));
        assert!(service
            .cache_service
            .as_ref()
            .unwrap()
            .is_version_current("opencode"));
        let warm = service.load_warm_path().unwrap();
        assert_eq!(
            serde_json::to_value(&warm.source_summaries).unwrap(),
            serde_json::to_value(&result.source_summaries).unwrap()
        );
    }
}
