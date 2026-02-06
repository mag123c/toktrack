//! Unified data loading service for CLI and TUI
//!
//! This module provides a single `DataLoaderService` that consolidates
//! the duplicated data loading logic from CLI and TUI.

use std::collections::HashMap;

use crate::parsers::ParserRegistry;
use crate::services::{Aggregator, DailySummaryCacheService, PricingService};
use crate::types::{CacheWarning, DailySummary, Result, SourceUsage, ToktrackError, UsageEntry};

/// Result of loading data from all parsers
#[derive(Debug)]
pub struct LoadResult {
    /// Daily summaries from all sources, merged by date
    pub summaries: Vec<DailySummary>,
    /// Usage breakdown by source CLI
    pub source_usage: Vec<SourceUsage>,
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

    /// Check if any parser has a valid (version-matching) cache
    fn has_valid_cache(&self) -> bool {
        self.cache_service.as_ref().is_some_and(|cs| {
            self.registry
                .parsers()
                .iter()
                .any(|p| cs.is_version_current(p.name()))
        })
    }

    /// Warm path: use cached DailySummaries + parse only recent files
    fn load_warm_path(&self) -> Result<LoadResult> {
        let cache_service = self
            .cache_service
            .as_ref()
            .ok_or_else(|| ToktrackError::Cache("No cache service".into()))?;

        let since = std::time::SystemTime::now() - std::time::Duration::from_secs(24 * 3600);

        let mut all_summaries = Vec::new();
        let mut source_stats: HashMap<String, (u64, f64)> = HashMap::new();
        let mut cache_warning = None;

        for parser in self.registry.parsers() {
            let has_parser_cache = cache_service.cache_path(parser.name()).exists();

            let entries = if has_parser_cache {
                match parser.parse_recent_files(since) {
                    Ok(e) => e,
                    Err(e) => {
                        eprintln!("[toktrack] Warning: {} failed: {}", parser.name(), e);
                        continue;
                    }
                }
            } else {
                match parser.parse_all() {
                    Ok(e) => e,
                    Err(e) => {
                        eprintln!("[toktrack] Warning: {} failed: {}", parser.name(), e);
                        continue;
                    }
                }
            };

            let entries = self.apply_pricing(entries);

            match cache_service.load_or_compute(parser.name(), &entries) {
                Ok((summaries, warning)) => {
                    if warning.is_some() && cache_warning.is_none() {
                        cache_warning = warning;
                    }
                    self.collect_source_stats(&summaries, parser.name(), &mut source_stats);
                    all_summaries.extend(summaries);
                }
                Err(e) => {
                    eprintln!(
                        "[toktrack] Warning: cache for {} failed: {}",
                        parser.name(),
                        e
                    );
                }
            }
        }

        let all_summaries = Aggregator::merge_by_date(all_summaries);
        let source_usage = Self::build_source_usage(source_stats);

        Ok(LoadResult {
            summaries: all_summaries,
            source_usage,
            cache_warning,
        })
    }

    /// Cold path: full parse_all() per parser + build cache
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
        let mut cache_warning = None;
        let mut any_entries = false;

        for parser in self.registry.parsers() {
            let entries = match parser.parse_all() {
                Ok(e) => e,
                Err(e) => {
                    eprintln!("[toktrack] Warning: {} failed: {}", parser.name(), e);
                    continue;
                }
            };

            if entries.is_empty() {
                continue;
            }
            any_entries = true;

            let entries = self.apply_pricing_with_ref(entries, pricing_ref);

            // Try to use cache service
            if let Some(cs) = &self.cache_service {
                match cs.load_or_compute(parser.name(), &entries) {
                    Ok((summaries, warning)) => {
                        if warning.is_some() && cache_warning.is_none() {
                            cache_warning = warning;
                        }
                        self.collect_source_stats(&summaries, parser.name(), &mut source_stats);
                        all_summaries.extend(summaries);
                        continue;
                    }
                    Err(e) => {
                        eprintln!(
                            "[toktrack] Warning: cache for {} failed: {}",
                            parser.name(),
                            e
                        );
                    }
                }
            }

            // Cache unavailable: compute summaries directly
            let summaries = Aggregator::daily(&entries);
            self.collect_source_stats(&summaries, parser.name(), &mut source_stats);
            all_summaries.extend(summaries);
        }

        if !any_entries {
            return Err(ToktrackError::Parse(
                "No usage data found from any CLI".into(),
            ));
        }

        let all_summaries = Aggregator::merge_by_date(all_summaries);
        let source_usage = Self::build_source_usage(source_stats);

        Ok(LoadResult {
            summaries: all_summaries,
            source_usage,
            cache_warning,
        })
    }

    /// Apply pricing to entries using cached pricing service
    fn apply_pricing(&self, entries: Vec<UsageEntry>) -> Vec<UsageEntry> {
        self.apply_pricing_with_ref(entries, self.pricing.as_ref())
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
                } else if entry.cost_usd.is_none() || entry.cost_usd == Some(0.0) {
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
                + s.total_thinking_tokens;
            let stat = stats.entry(source_name.to_string()).or_default();
            stat.0 = stat.0.saturating_add(tokens);
            stat.1 += s.total_cost_usd;
        }
    }

    /// Convert source stats map to sorted SourceUsage vector
    fn build_source_usage(source_stats: HashMap<String, (u64, f64)>) -> Vec<SourceUsage> {
        let mut result: Vec<SourceUsage> = source_stats
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
    use super::*;

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

    // ========== build_source_usage tests ==========

    #[test]
    fn test_build_source_usage_empty() {
        let stats = HashMap::new();
        let result = DataLoaderService::build_source_usage(stats);
        assert!(result.is_empty());
    }

    #[test]
    fn test_build_source_usage_single_source() {
        let mut stats = HashMap::new();
        stats.insert("claude".to_string(), (1000u64, 0.05f64));

        let result = DataLoaderService::build_source_usage(stats);

        assert_eq!(result.len(), 1);
        assert_eq!(result[0].source, "claude");
        assert_eq!(result[0].total_tokens, 1000);
        assert!((result[0].total_cost_usd - 0.05).abs() < f64::EPSILON);
    }

    #[test]
    fn test_build_source_usage_sorted_by_tokens_descending() {
        let mut stats = HashMap::new();
        stats.insert("claude".to_string(), (500u64, 0.03f64));
        stats.insert("opencode".to_string(), (2000u64, 0.10f64));
        stats.insert("gemini".to_string(), (1000u64, 0.05f64));

        let result = DataLoaderService::build_source_usage(stats);

        assert_eq!(result.len(), 3);
        assert_eq!(result[0].source, "opencode");
        assert_eq!(result[0].total_tokens, 2000);
        assert_eq!(result[1].source, "gemini");
        assert_eq!(result[1].total_tokens, 1000);
        assert_eq!(result[2].source, "claude");
        assert_eq!(result[2].total_tokens, 500);
    }

    // ========== DataLoaderService::new tests ==========

    #[test]
    fn test_data_loader_service_new() {
        let service = DataLoaderService::new();
        // Just verify it can be constructed
        assert!(!service.registry.parsers().is_empty());
    }

    #[test]
    fn test_data_loader_service_default() {
        let service = DataLoaderService::default();
        assert!(!service.registry.parsers().is_empty());
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
            thinking_tokens: 0,
            cost_usd,
            message_id: None,
            request_id: None,
            source: None,
            provider: provider.map(|s| s.to_string()),
        }
    }

    #[test]
    fn test_apply_pricing_zero_cost_triggers_recalculation() {
        let service = DataLoaderService::new();
        let entries = vec![make_entry(Some(0.0), Some("anthropic"))];
        let result = service.apply_pricing(entries);
        // Some(0.0) should NOT be trusted — should recalculate (or remain 0 if no pricing)
        // Key: the condition should treat Some(0.0) same as None
        assert_ne!(result[0].cost_usd, Some(0.0));
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
}
