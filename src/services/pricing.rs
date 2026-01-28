//! Pricing service for cost calculation
//!
//! Provides token-based cost calculation using LiteLLM pricing data.
//! Supports auto mode: uses pre-calculated cost_usd when available,
//! falls back to token-based calculation otherwise.

use crate::types::{Result, ToktrackError, UsageEntry};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

/// LiteLLM pricing URL
const LITELLM_PRICING_URL: &str =
    "https://raw.githubusercontent.com/BerriAI/litellm/main/model_prices_and_context_window.json";

/// Cache TTL in seconds (1 hour)
const CACHE_TTL_SECS: i64 = 3600;

/// HTTP request timeout in seconds
const REQUEST_TIMEOUT_SECS: u64 = 10;

/// Pricing information for a model
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ModelPricing {
    #[serde(default)]
    pub input_cost_per_token: Option<f64>,
    #[serde(default)]
    pub output_cost_per_token: Option<f64>,
    #[serde(default)]
    pub cache_read_input_token_cost: Option<f64>,
    #[serde(default)]
    pub cache_creation_input_token_cost: Option<f64>,
}

/// Cached pricing data
#[derive(Debug, Serialize, Deserialize)]
pub struct PricingCache {
    /// Unix timestamp when the cache was fetched
    pub fetched_at: i64,
    /// Model pricing data
    pub models: HashMap<String, ModelPricing>,
}

impl PricingCache {
    /// Check if the cache has expired
    pub fn is_expired(&self) -> bool {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        now - self.fetched_at > CACHE_TTL_SECS
    }
}

/// Pricing service for calculating token costs
pub struct PricingService {
    cache: PricingCache,
    cache_path: PathBuf,
}

impl PricingService {
    /// Create a new PricingService, loading from cache or fetching fresh data
    pub fn new() -> Result<Self> {
        let cache_path = Self::default_cache_path()?;
        Self::with_cache_path(cache_path)
    }

    /// Create a new PricingService with a custom cache path
    pub fn with_cache_path(cache_path: PathBuf) -> Result<Self> {
        let cache = Self::load_or_fetch_cache(&cache_path)?;
        Ok(Self { cache, cache_path })
    }

    /// Get the default cache path (~/.toktrack/pricing.json)
    fn default_cache_path() -> Result<PathBuf> {
        let home = directories::UserDirs::new()
            .ok_or_else(|| ToktrackError::Pricing("Failed to get home directory".into()))?
            .home_dir()
            .to_path_buf();
        Ok(home.join(".toktrack").join("pricing.json"))
    }

    /// Load cache from disk or fetch fresh data
    fn load_or_fetch_cache(cache_path: &PathBuf) -> Result<PricingCache> {
        // Try loading existing cache
        if let Ok(cache) = Self::load_cache(cache_path) {
            if !cache.is_expired() {
                return Ok(cache);
            }
            // Cache expired, try to refresh
            if let Ok(fresh_cache) = Self::fetch_pricing() {
                let _ = Self::save_cache(cache_path, &fresh_cache);
                return Ok(fresh_cache);
            }
            // Fetch failed, use expired cache
            return Ok(cache);
        }

        // No cache exists, must fetch
        let cache = Self::fetch_pricing()
            .map_err(|e| ToktrackError::Pricing(format!("Failed to fetch pricing data: {}", e)))?;
        let _ = Self::save_cache(cache_path, &cache);
        Ok(cache)
    }

    /// Load cache from disk
    fn load_cache(cache_path: &PathBuf) -> Result<PricingCache> {
        let content = fs::read_to_string(cache_path)?;
        let cache: PricingCache = serde_json::from_str(&content)
            .map_err(|e| ToktrackError::Pricing(format!("Invalid cache format: {}", e)))?;
        Ok(cache)
    }

    /// Save cache to disk
    fn save_cache(cache_path: &PathBuf, cache: &PricingCache) -> Result<()> {
        if let Some(parent) = cache_path.parent() {
            fs::create_dir_all(parent)?;
        }
        let content = serde_json::to_string_pretty(cache)
            .map_err(|e| ToktrackError::Pricing(format!("Serialization failed: {}", e)))?;
        fs::write(cache_path, content)?;
        Ok(())
    }

    /// Fetch pricing data from LiteLLM
    fn fetch_pricing() -> std::result::Result<PricingCache, String> {
        let client = reqwest::blocking::Client::builder()
            .timeout(std::time::Duration::from_secs(REQUEST_TIMEOUT_SECS))
            .build()
            .map_err(|e| format!("HTTP client error: {}", e))?;

        let response = client
            .get(LITELLM_PRICING_URL)
            .send()
            .map_err(|e| format!("HTTP request failed: {}", e))?;

        let models: HashMap<String, ModelPricing> = response
            .json()
            .map_err(|e| format!("JSON parse error: {}", e))?;

        let fetched_at = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);

        Ok(PricingCache { fetched_at, models })
    }

    /// Get cost, using pre-calculated cost_usd if available (auto mode)
    pub fn get_or_calculate_cost(&self, entry: &UsageEntry) -> f64 {
        if let Some(cost) = entry.cost_usd {
            return cost;
        }
        self.calculate_cost(entry)
    }

    /// Calculate cost from tokens (always calculates, ignores cost_usd)
    pub fn calculate_cost(&self, entry: &UsageEntry) -> f64 {
        let model = match &entry.model {
            Some(m) => m,
            None => return 0.0,
        };

        let pricing = match self.get_pricing(model) {
            Some(p) => p,
            None => return 0.0,
        };

        // Non-cached input = input_tokens - cache_read_tokens
        let non_cached_input = entry.input_tokens.saturating_sub(entry.cache_read_tokens);

        let input_cost = pricing.input_cost_per_token.unwrap_or(0.0);
        let output_cost = pricing.output_cost_per_token.unwrap_or(0.0);
        let cache_read_cost = pricing.cache_read_input_token_cost.unwrap_or(0.0);
        let cache_creation_cost = pricing.cache_creation_input_token_cost.unwrap_or(0.0);

        (non_cached_input as f64 * input_cost)
            + (entry.cache_read_tokens as f64 * cache_read_cost)
            + (entry.cache_creation_tokens as f64 * cache_creation_cost)
            + (entry.output_tokens as f64 * output_cost)
    }

    /// Get pricing for a model (exact match only)
    pub fn get_pricing(&self, model: &str) -> Option<&ModelPricing> {
        self.cache.models.get(model)
    }

    /// Force refresh pricing data
    pub fn refresh(&mut self) -> Result<()> {
        let cache = Self::fetch_pricing()
            .map_err(|e| ToktrackError::Pricing(format!("Refresh failed: {}", e)))?;
        let _ = Self::save_cache(&self.cache_path, &cache);
        self.cache = cache;
        Ok(())
    }

    /// Get the number of models in the cache
    pub fn model_count(&self) -> usize {
        self.cache.models.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use std::time::{SystemTime, UNIX_EPOCH};
    use tempfile::TempDir;

    fn make_entry(
        model: Option<&str>,
        input: u64,
        output: u64,
        cache_read: u64,
        cache_creation: u64,
        cost_usd: Option<f64>,
    ) -> UsageEntry {
        UsageEntry {
            timestamp: Utc::now(),
            model: model.map(String::from),
            input_tokens: input,
            output_tokens: output,
            cache_read_tokens: cache_read,
            cache_creation_tokens: cache_creation,
            thinking_tokens: 0,
            cost_usd,
            message_id: None,
            request_id: None,
            source: None,
        }
    }

    fn create_test_service() -> (PricingService, TempDir) {
        let temp_dir = TempDir::new().unwrap();
        let cache_path = temp_dir.path().join("pricing.json");

        // Create mock pricing data
        let mut models = HashMap::new();
        models.insert(
            "claude-sonnet-4".to_string(),
            ModelPricing {
                input_cost_per_token: Some(0.000003),         // $3 per 1M tokens
                output_cost_per_token: Some(0.000015),        // $15 per 1M tokens
                cache_read_input_token_cost: Some(0.0000003), // $0.30 per 1M tokens
                cache_creation_input_token_cost: Some(0.00000375), // $3.75 per 1M tokens
            },
        );
        models.insert(
            "claude-opus-4".to_string(),
            ModelPricing {
                input_cost_per_token: Some(0.000015),  // $15 per 1M tokens
                output_cost_per_token: Some(0.000075), // $75 per 1M tokens
                cache_read_input_token_cost: Some(0.0000015), // $1.50 per 1M tokens
                cache_creation_input_token_cost: Some(0.00001875), // $18.75 per 1M tokens
            },
        );

        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;

        let cache = PricingCache {
            fetched_at: now,
            models,
        };

        // Save mock cache
        let content = serde_json::to_string_pretty(&cache).unwrap();
        fs::create_dir_all(cache_path.parent().unwrap()).unwrap();
        fs::write(&cache_path, content).unwrap();

        let service = PricingService::with_cache_path(cache_path).unwrap();
        (service, temp_dir)
    }

    // ========== get_or_calculate_cost tests (auto mode) ==========

    #[test]
    fn test_returns_existing_cost_usd_when_present() {
        let (service, _temp) = create_test_service();
        let entry = make_entry(Some("claude-sonnet-4"), 1000, 500, 0, 0, Some(0.05));

        let cost = service.get_or_calculate_cost(&entry);

        assert!((cost - 0.05).abs() < f64::EPSILON);
    }

    #[test]
    fn test_calculates_when_cost_usd_is_none() {
        let (service, _temp) = create_test_service();
        // Using claude-sonnet-4: $3/$15 per 1M tokens
        // 1000 input * 0.000003 + 500 output * 0.000015 = 0.003 + 0.0075 = 0.0105
        let entry = make_entry(Some("claude-sonnet-4"), 1000, 500, 0, 0, None);

        let cost = service.get_or_calculate_cost(&entry);

        assert!(
            (cost - 0.0105).abs() < 1e-10,
            "Expected 0.0105, got {}",
            cost
        );
    }

    // ========== calculate_cost tests ==========

    #[test]
    fn test_calculate_cost_basic() {
        let (service, _temp) = create_test_service();
        // claude-sonnet-4: input=$3/1M, output=$15/1M
        // 1000 input * $0.000003 = $0.003
        // 500 output * $0.000015 = $0.0075
        // Total: $0.0105
        let entry = make_entry(Some("claude-sonnet-4"), 1000, 500, 0, 0, None);

        let cost = service.calculate_cost(&entry);

        assert!(
            (cost - 0.0105).abs() < 1e-10,
            "Expected 0.0105, got {}",
            cost
        );
    }

    #[test]
    fn test_calculate_cost_with_cache_tokens() {
        let (service, _temp) = create_test_service();
        // claude-sonnet-4:
        // - input=$3/1M, output=$15/1M
        // - cache_read=$0.30/1M, cache_creation=$3.75/1M
        //
        // Entry: input=1000, output=500, cache_read=200, cache_creation=100
        // non_cached_input = 1000 - 200 = 800
        //
        // Cost = (800 * 0.000003) + (200 * 0.0000003) + (100 * 0.00000375) + (500 * 0.000015)
        //      = 0.0024 + 0.00006 + 0.000375 + 0.0075
        //      = 0.010335
        let entry = make_entry(Some("claude-sonnet-4"), 1000, 500, 200, 100, None);

        let cost = service.calculate_cost(&entry);

        assert!(
            (cost - 0.010335).abs() < 1e-10,
            "Expected 0.010335, got {}",
            cost
        );
    }

    #[test]
    fn test_calculate_cost_unknown_model_returns_zero() {
        let (service, _temp) = create_test_service();
        let entry = make_entry(Some("unknown-model-xyz"), 1000, 500, 0, 0, None);

        let cost = service.calculate_cost(&entry);

        assert!((cost - 0.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_calculate_cost_none_model_returns_zero() {
        let (service, _temp) = create_test_service();
        let entry = make_entry(None, 1000, 500, 0, 0, None);

        let cost = service.calculate_cost(&entry);

        assert!((cost - 0.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_non_cached_input_calculation() {
        let (service, _temp) = create_test_service();
        // If cache_read > input_tokens, saturating_sub should give 0
        // This tests edge case: input=100, cache_read=150
        let entry = make_entry(Some("claude-sonnet-4"), 100, 500, 150, 0, None);

        // non_cached_input = 100.saturating_sub(150) = 0
        // cost = (0 * input) + (150 * cache_read) + (0 * cache_create) + (500 * output)
        //      = 0 + (150 * 0.0000003) + 0 + (500 * 0.000015)
        //      = 0.000045 + 0.0075
        //      = 0.007545
        let cost = service.calculate_cost(&entry);

        assert!(
            (cost - 0.007545).abs() < 1e-10,
            "Expected 0.007545, got {}",
            cost
        );
    }

    // ========== get_pricing tests ==========

    #[test]
    fn test_get_pricing_exact_match() {
        let (service, _temp) = create_test_service();

        let pricing = service.get_pricing("claude-sonnet-4");

        assert!(pricing.is_some());
        let p = pricing.unwrap();
        assert!((p.input_cost_per_token.unwrap() - 0.000003).abs() < 1e-10);
    }

    #[test]
    fn test_get_pricing_not_found() {
        let (service, _temp) = create_test_service();

        let pricing = service.get_pricing("nonexistent-model");

        assert!(pricing.is_none());
    }

    // ========== PricingCache tests ==========

    #[test]
    fn test_cache_is_expired_after_1h() {
        let old_timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64
            - 3601; // 1 hour + 1 second ago

        let cache = PricingCache {
            fetched_at: old_timestamp,
            models: HashMap::new(),
        };

        assert!(cache.is_expired());
    }

    #[test]
    fn test_cache_is_valid_within_1h() {
        let recent_timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64
            - 1800; // 30 minutes ago

        let cache = PricingCache {
            fetched_at: recent_timestamp,
            models: HashMap::new(),
        };

        assert!(!cache.is_expired());
    }

    #[test]
    fn test_cache_load_and_save() {
        let temp_dir = TempDir::new().unwrap();
        let cache_path = temp_dir.path().join("test_cache.json");

        let mut models = HashMap::new();
        models.insert(
            "test-model".to_string(),
            ModelPricing {
                input_cost_per_token: Some(0.001),
                output_cost_per_token: Some(0.002),
                cache_read_input_token_cost: None,
                cache_creation_input_token_cost: None,
            },
        );

        let cache = PricingCache {
            fetched_at: 12345,
            models,
        };

        // Save
        PricingService::save_cache(&cache_path, &cache).unwrap();

        // Load
        let loaded = PricingService::load_cache(&cache_path).unwrap();

        assert_eq!(loaded.fetched_at, 12345);
        assert!(loaded.models.contains_key("test-model"));
    }

    #[test]
    fn test_model_count() {
        let (service, _temp) = create_test_service();

        // We added 2 models in create_test_service
        assert_eq!(service.model_count(), 2);
    }
}
