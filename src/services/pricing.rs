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

/// Vendored offline pricing fallback (trimmed LiteLLM snapshot — only the fields
/// `ModelPricing` reads). Used when there is no live fetch and no on-disk cache,
/// so first-run-offline still prices instead of reporting $0. Regenerate by
/// trimming `model_prices_and_context_window.json` to `ModelPricing`'s fields.
const PRICING_SNAPSHOT_JSON: &str = include_str!("../../assets/pricing_snapshot.json");

/// Cache TTL in seconds (1 hour)
const CACHE_TTL_SECS: i64 = 3600;

/// HTTP request timeout in seconds
const REQUEST_TIMEOUT_SECS: u64 = 10;

/// LiteLLM web search pricing: nested `search_context_cost_per_query` object.
/// Tiers are per-query USD; many models set all three equal (Claude = 0.01).
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SearchContextCost {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub search_context_size_low: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub search_context_size_medium: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub search_context_size_high: Option<f64>,
}

impl SearchContextCost {
    /// Per-query cost, preferring the medium tier, then low, then high.
    fn per_query_cost(&self) -> Option<f64> {
        self.search_context_size_medium
            .or(self.search_context_size_low)
            .or(self.search_context_size_high)
    }
}

/// Pricing information for a model
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ModelPricing {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_cost_per_token: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_cost_per_token: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_read_input_token_cost: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_creation_input_token_cost: Option<f64>,
    // Tiered pricing fields. A model carries at most one breakpoint per token
    // kind; LiteLLM uses 128k / 200k / 256k / 272k variants.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_cost_per_token_above_128k_tokens: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_cost_per_token_above_200k_tokens: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_cost_per_token_above_256k_tokens: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_cost_per_token_above_272k_tokens: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_cost_per_token_above_128k_tokens: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_cost_per_token_above_200k_tokens: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_cost_per_token_above_256k_tokens: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_cost_per_token_above_272k_tokens: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_read_input_token_cost_above_200k_tokens: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_read_input_token_cost_above_272k_tokens: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_creation_input_token_cost_above_200k_tokens: Option<f64>,
    // 1h ephemeral cache writes. LiteLLM prices the longer TTL under its own
    // `_above_1hr` keys (Claude Code writes 1h caches by default), with an
    // optional 200k breakpoint on top.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_creation_input_token_cost_above_1hr: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_creation_input_token_cost_above_1hr_above_200k_tokens: Option<f64>,
    // Cache TTL-specific pricing (for Bedrock/Vertex)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_creation_5m_token_cost: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_creation_1h_token_cost: Option<f64>,
    // Web search pricing (LiteLLM nested object `search_context_cost_per_query`)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub search_context_cost_per_query: Option<SearchContextCost>,
    // Reasoning/thinking token cost; falls back to the output rate when absent
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_cost_per_reasoning_token: Option<f64>,
    /// Provider-specific rate multipliers keyed by mode (LiteLLM ships `fast`
    /// and `us`). Only `fast` is applied: Claude Code records the speed per
    /// request, while `inference_geo` reads `not_available` in practice. The
    /// same LiteLLM key also carries non-numeric provider metadata on other
    /// models, so anything that is not a number is dropped.
    #[serde(
        default,
        deserialize_with = "deserialize_numeric_map",
        skip_serializing_if = "Option::is_none"
    )]
    pub provider_specific_entry: Option<HashMap<String, f64>>,
}

impl ModelPricing {
    /// Whether this entry prices anything at all. LiteLLM carries many entries
    /// that only describe capabilities, and those are dropped from the vendored
    /// snapshot.
    #[allow(dead_code)] // Used by the gen_pricing_snapshot tool and tests
    pub fn has_any_pricing(&self) -> bool {
        self.input_cost_per_token.is_some()
            || self.output_cost_per_token.is_some()
            || self.cache_read_input_token_cost.is_some()
            || self.cache_creation_input_token_cost.is_some()
            || self.cache_creation_input_token_cost_above_1hr.is_some()
            || self.output_cost_per_reasoning_token.is_some()
            || self
                .search_context_cost_per_query
                .as_ref()
                .and_then(SearchContextCost::per_query_cost)
                .is_some()
    }
}

/// Keeps only the numeric members of a LiteLLM map. `provider_specific_entry`
/// mixes rate multipliers with provider metadata such as
/// `{"bedrock_invocation_schema": "titan_v2"}`.
fn deserialize_numeric_map<'de, D>(
    deserializer: D,
) -> std::result::Result<Option<HashMap<String, f64>>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let raw: Option<HashMap<String, serde_json::Value>> = Option::deserialize(deserializer)?;
    Ok(raw.map(|m| {
        m.into_iter()
            .filter_map(|(k, v)| v.as_f64().map(|f| (k, f)))
            .collect()
    }))
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

/// Resolve the single active tiered breakpoint for a token kind. A model
/// declares at most one above-tier; pick the present one (128k→200k→256k→272k).
/// Returns `(threshold_tokens, above_rate)`.
fn tier(
    above_128k: Option<f64>,
    above_200k: Option<f64>,
    above_256k: Option<f64>,
    above_272k: Option<f64>,
) -> Option<(u64, f64)> {
    if let Some(r) = above_128k {
        Some((128_000, r))
    } else if let Some(r) = above_200k {
        Some((200_000, r))
    } else if let Some(r) = above_256k {
        Some((256_000, r))
    } else {
        above_272k.map(|r| (272_000, r))
    }
}

/// Calculate cost with tiered pricing: base rate up to the breakpoint, tiered
/// rate above. `tier` is `(threshold_tokens, above_rate)` when the model is tiered.
fn tiered_cost(tokens: u64, base_price: f64, tier: Option<(u64, f64)>) -> f64 {
    match tier {
        Some((threshold, above_rate)) if tokens > threshold => {
            let below = threshold as f64 * base_price;
            let above = (tokens - threshold) as f64 * above_rate;
            below + above
        }
        _ => tokens as f64 * base_price,
    }
}

/// Rate multiplier for the speed the request ran at. Claude Code records
/// `usage.speed`, and LiteLLM carries the matching multiplier under
/// `provider_specific_entry.fast` (2.0 for claude-opus-5, matching its $10/$50
/// fast pricing against a $5/$25 base). Anything else bills at 1x.
fn speed_multiplier(entry: &UsageEntry, rates: Option<&HashMap<String, f64>>) -> f64 {
    if !entry.fast_speed {
        return 1.0;
    }
    rates.and_then(|m| m.get("fast")).copied().unwrap_or(1.0)
}

/// Custom pricing configuration from ~/.toktrack/pricing.toml
/// All costs are in $/1M tokens (converted to per-token internally)
#[derive(Debug, Deserialize)]
struct CustomPricingConfig {
    models: Option<HashMap<String, CustomModelPricing>>,
    global: Option<GlobalPricing>,
}

#[derive(Debug, Deserialize)]
struct CustomModelPricing {
    input: Option<f64>,
    output: Option<f64>,
    cache_creation: Option<f64>,
    cache_read: Option<f64>,
    cache_creation_5m: Option<f64>,
    cache_creation_1h: Option<f64>,
    // Tiered pricing (above 200k tokens)
    input_above_200k: Option<f64>,
    output_above_200k: Option<f64>,
    cache_read_above_200k: Option<f64>,
    cache_creation_above_200k: Option<f64>,
}

#[derive(Debug, Deserialize)]
struct GlobalPricing {
    web_search_per_request: Option<f64>,
    /// Optional per-request web fetch cost. No LiteLLM source exists, so web fetch
    /// is priced only when the user sets this.
    web_fetch_per_request: Option<f64>,
}

/// Pricing service for calculating token costs
pub struct PricingService {
    cache: PricingCache,
    #[allow(dead_code)]
    cache_path: PathBuf,
    custom: Option<CustomPricingConfig>,
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
        let custom = Self::load_custom_pricing();
        Ok(Self {
            cache,
            cache_path,
            custom,
        })
    }

    /// Create a PricingService, preferring cache but refreshing if expired or corrupt.
    /// Falls back to the vendored snapshot when the cache is unreadable and the
    /// network is unavailable, so it returns None only if the home dir can't be found.
    pub fn from_cache_only() -> Option<Self> {
        let cache_path = Self::default_cache_path().ok()?;
        let custom = Self::load_custom_pricing();

        match Self::load_cache(&cache_path) {
            Ok(cache) if !cache.is_expired() => Some(Self {
                cache,
                cache_path,
                custom,
            }),
            Ok(cache) => {
                if let Ok(fresh) = Self::fetch_pricing() {
                    let _ = Self::save_cache(&cache_path, &fresh);
                    Some(Self {
                        cache: fresh,
                        cache_path,
                        custom,
                    })
                } else {
                    Some(Self {
                        cache,
                        cache_path,
                        custom,
                    })
                }
            }
            Err(_) => {
                if let Ok(fresh) = Self::fetch_pricing() {
                    let _ = Self::save_cache(&cache_path, &fresh);
                    Some(Self {
                        cache: fresh,
                        cache_path,
                        custom,
                    })
                } else {
                    eprintln!(
                        "[toktrack] Warning: pricing cache unreadable and fetch failed; \
                         using bundled offline snapshot (costs may be stale)"
                    );
                    Some(Self {
                        cache: Self::snapshot_cache(),
                        cache_path,
                        custom,
                    })
                }
            }
        }
    }

    /// Cache-only constructor with custom path (for testing)
    #[allow(dead_code)]
    pub fn from_cache_only_with_path(cache_path: &PathBuf) -> Option<Self> {
        let cache = Self::load_cache(cache_path).ok()?;
        Some(Self {
            cache,
            cache_path: cache_path.clone(),
            custom: None,
        })
    }

    /// Get the default cache path (~/.toktrack/pricing.json)
    fn default_cache_path() -> Result<PathBuf> {
        let home = directories::UserDirs::new()
            .ok_or_else(|| ToktrackError::Pricing("Failed to get home directory".into()))?
            .home_dir()
            .to_path_buf();
        Ok(home.join(".toktrack").join("pricing.json"))
    }

    /// Load custom pricing from TOKTRACK_PRICING_FILE env var or ~/.toktrack/pricing.toml
    fn load_custom_pricing() -> Option<CustomPricingConfig> {
        let path = if let Ok(env_path) = std::env::var("TOKTRACK_PRICING_FILE") {
            PathBuf::from(env_path)
        } else {
            let home = directories::UserDirs::new()?.home_dir().to_path_buf();
            home.join(".toktrack").join("pricing.toml")
        };
        let content = fs::read_to_string(&path).ok()?;
        match toml::from_str::<CustomPricingConfig>(&content) {
            Ok(config) => Some(config),
            Err(e) => {
                eprintln!(
                    "[toktrack] Warning: Failed to parse {}: {}",
                    path.display(),
                    e
                );
                None
            }
        }
    }

    /// Load custom pricing from a specific path (for testing)
    #[allow(dead_code)]
    fn with_custom_pricing(mut self, custom: Option<CustomPricingConfig>) -> Self {
        self.custom = custom;
        self
    }

    /// Get custom model pricing as ModelPricing (converting $/1M to $/token)
    fn get_custom_pricing(&self, model: &str) -> Option<ModelPricing> {
        let custom = self.custom.as_ref()?;
        let models = custom.models.as_ref()?;
        let custom_model = models.get(model)?;

        let to_per_token = |v: Option<f64>| v.map(|x| x / 1_000_000.0);

        Some(ModelPricing {
            input_cost_per_token: to_per_token(custom_model.input),
            output_cost_per_token: to_per_token(custom_model.output),
            cache_creation_input_token_cost: to_per_token(custom_model.cache_creation),
            cache_read_input_token_cost: to_per_token(custom_model.cache_read),
            cache_creation_5m_token_cost: to_per_token(custom_model.cache_creation_5m),
            cache_creation_1h_token_cost: to_per_token(custom_model.cache_creation_1h),
            // Custom pricing keeps the 200k breakpoint; other tiers stay None.
            input_cost_per_token_above_128k_tokens: None,
            input_cost_per_token_above_200k_tokens: to_per_token(custom_model.input_above_200k),
            input_cost_per_token_above_256k_tokens: None,
            input_cost_per_token_above_272k_tokens: None,
            output_cost_per_token_above_128k_tokens: None,
            output_cost_per_token_above_200k_tokens: to_per_token(custom_model.output_above_200k),
            output_cost_per_token_above_256k_tokens: None,
            output_cost_per_token_above_272k_tokens: None,
            cache_read_input_token_cost_above_200k_tokens: to_per_token(
                custom_model.cache_read_above_200k,
            ),
            cache_read_input_token_cost_above_272k_tokens: None,
            cache_creation_input_token_cost_above_200k_tokens: to_per_token(
                custom_model.cache_creation_above_200k,
            ),
            // Custom pricing expresses the 1h rate through `cache_creation_1h`,
            // so the LiteLLM-shaped `_above_1hr` keys stay unset.
            cache_creation_input_token_cost_above_1hr: None,
            cache_creation_input_token_cost_above_1hr_above_200k_tokens: None,
            provider_specific_entry: None,
            search_context_cost_per_query: None,
            output_cost_per_reasoning_token: None,
        })
    }

    /// Vendored snapshot as a `PricingCache`. `fetched_at = 0` marks it stale so
    /// it is refreshed from the network at the next opportunity.
    fn snapshot_cache() -> PricingCache {
        let models: HashMap<String, ModelPricing> = serde_json::from_str(PRICING_SNAPSHOT_JSON)
            .unwrap_or_else(|e| {
                eprintln!("[toktrack] BUG: bundled pricing snapshot failed to parse: {e}");
                HashMap::new()
            });
        PricingCache {
            fetched_at: 0,
            models,
        }
    }

    /// Load cache from disk or fetch fresh data
    fn load_or_fetch_cache(cache_path: &PathBuf) -> Result<PricingCache> {
        if let Ok(cache) = Self::load_cache(cache_path) {
            if !cache.is_expired() {
                return Ok(cache);
            }
            if let Ok(fresh_cache) = Self::fetch_pricing() {
                let _ = Self::save_cache(cache_path, &fresh_cache);
                return Ok(fresh_cache);
            }
            return Ok(cache);
        }

        // No cache exists: fetch, else fall back to the vendored snapshot so we
        // never report $0 pricing offline.
        match Self::fetch_pricing() {
            Ok(cache) => {
                let _ = Self::save_cache(cache_path, &cache);
                Ok(cache)
            }
            Err(_) => {
                eprintln!(
                    "[toktrack] Warning: no cached pricing and fetch failed; \
                     using bundled offline snapshot (costs may be stale)"
                );
                Ok(Self::snapshot_cache())
            }
        }
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
    #[allow(dead_code)]
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

        let custom = self.get_custom_pricing(model);
        let litellm = self.get_pricing(model);

        let pricing = match (&custom, litellm) {
            (Some(c), _) => c,
            (None, Some(l)) => l,
            (None, None) => return 0.0,
        };

        let input = tiered_cost(
            entry.input_tokens,
            pricing.input_cost_per_token.unwrap_or(0.0),
            tier(
                pricing.input_cost_per_token_above_128k_tokens,
                pricing.input_cost_per_token_above_200k_tokens,
                pricing.input_cost_per_token_above_256k_tokens,
                pricing.input_cost_per_token_above_272k_tokens,
            ),
        );
        let output = tiered_cost(
            entry.output_tokens,
            pricing.output_cost_per_token.unwrap_or(0.0),
            tier(
                pricing.output_cost_per_token_above_128k_tokens,
                pricing.output_cost_per_token_above_200k_tokens,
                pricing.output_cost_per_token_above_256k_tokens,
                pricing.output_cost_per_token_above_272k_tokens,
            ),
        );
        let cache_read = tiered_cost(
            entry.cache_read_tokens,
            pricing.cache_read_input_token_cost.unwrap_or(0.0),
            tier(
                None,
                pricing.cache_read_input_token_cost_above_200k_tokens,
                None,
                pricing.cache_read_input_token_cost_above_272k_tokens,
            ),
        );

        // Cache creation: use TTL-aware pricing only when the entry carries a TTL
        // breakdown. Two shapes provide it — the user's own `cache_creation_5m`/
        // `cache_creation_1h` overrides, and LiteLLM's `_above_1hr` keys — and the
        // user's override wins.
        let has_custom_ttl_pricing = pricing.cache_creation_5m_token_cost.is_some()
            || pricing.cache_creation_1h_token_cost.is_some();
        let has_litellm_ttl_pricing = pricing.cache_creation_input_token_cost_above_1hr.is_some();
        let has_ttl_tokens =
            entry.cache_creation_5m_tokens > 0 || entry.cache_creation_1h_tokens > 0;

        let base_cache_creation_rate = pricing.cache_creation_input_token_cost.unwrap_or(0.0);

        let cache_creation = if has_custom_ttl_pricing && has_ttl_tokens {
            let cost_5m = entry.cache_creation_5m_tokens as f64
                * pricing
                    .cache_creation_5m_token_cost
                    .unwrap_or(base_cache_creation_rate);
            let cost_1h = entry.cache_creation_1h_tokens as f64
                * pricing
                    .cache_creation_1h_token_cost
                    .unwrap_or(base_cache_creation_rate);
            cost_5m + cost_1h
        } else if has_litellm_ttl_pricing && has_ttl_tokens {
            let cost_5m = tiered_cost(
                entry.cache_creation_5m_tokens,
                base_cache_creation_rate,
                tier(
                    None,
                    pricing.cache_creation_input_token_cost_above_200k_tokens,
                    None,
                    None,
                ),
            );
            let cost_1h = tiered_cost(
                entry.cache_creation_1h_tokens,
                pricing
                    .cache_creation_input_token_cost_above_1hr
                    .unwrap_or(base_cache_creation_rate),
                tier(
                    None,
                    pricing.cache_creation_input_token_cost_above_1hr_above_200k_tokens,
                    None,
                    None,
                ),
            );
            cost_5m + cost_1h
        } else {
            tiered_cost(
                entry.cache_creation_tokens,
                pricing.cache_creation_input_token_cost.unwrap_or(0.0),
                tier(
                    None,
                    pricing.cache_creation_input_token_cost_above_200k_tokens,
                    None,
                    None,
                ),
            )
        };

        // Reasoning/thinking tokens: billed at the reasoning rate when the model
        // declares one, otherwise at the output rate (Claude folds thinking into
        // output and reports reasoning_tokens=0, so it is unaffected). Flat — LiteLLM
        // has no reasoning `_above_200k` tier.
        let reasoning_rate = pricing
            .output_cost_per_reasoning_token
            .or(pricing.output_cost_per_token)
            .unwrap_or(0.0);
        let reasoning = entry.reasoning_tokens as f64 * reasoning_rate;

        let web_search_cost = self.get_web_search_cost(entry, pricing);
        let web_fetch_cost = self.get_web_fetch_cost(entry);

        // Fast mode multiplies the token rates; per-request tool charges are flat.
        // Custom pricing replaces the rate table wholesale but has no way to
        // express a speed multiplier, so fall back to LiteLLM's for it — otherwise
        // overriding a base rate would silently drop fast-mode billing.
        let speed_rates = pricing
            .provider_specific_entry
            .as_ref()
            .or_else(|| litellm.and_then(|l| l.provider_specific_entry.as_ref()));
        let token_cost = (input + output + cache_read + cache_creation + reasoning)
            * speed_multiplier(entry, speed_rates);

        token_cost + web_search_cost + web_fetch_cost
    }

    /// Web fetch cost. No LiteLLM source exists, so this is priced only via the
    /// custom `global.web_fetch_per_request` override (else 0).
    fn get_web_fetch_cost(&self, entry: &UsageEntry) -> f64 {
        if entry.web_fetch_requests == 0 {
            return 0.0;
        }
        let cost_per_request = self
            .custom
            .as_ref()
            .and_then(|c| c.global.as_ref())
            .and_then(|g| g.web_fetch_per_request)
            .unwrap_or(0.0);
        entry.web_fetch_requests as f64 * cost_per_request
    }

    /// Get web search cost per request (custom global override > pricing per model)
    fn get_web_search_cost(&self, entry: &UsageEntry, pricing: &ModelPricing) -> f64 {
        if entry.web_search_requests == 0 {
            return 0.0;
        }
        let cost_per_request = self
            .custom
            .as_ref()
            .and_then(|c| c.global.as_ref())
            .and_then(|g| g.web_search_per_request)
            .or_else(|| {
                pricing
                    .search_context_cost_per_query
                    .as_ref()
                    .and_then(SearchContextCost::per_query_cost)
            })
            .unwrap_or(0.0);
        entry.web_search_requests as f64 * cost_per_request
    }

    /// Get pricing for a model (exact → normalized → fuzzy substring)
    pub fn get_pricing(&self, model: &str) -> Option<&ModelPricing> {
        if let Some(pricing) = self.cache.models.get(model) {
            return Some(pricing);
        }

        let normalized = super::normalize_model_name(model);
        if normalized != model {
            if let Some(pricing) = self.cache.models.get(&normalized) {
                return Some(pricing);
            }
        }

        let lower = normalized.to_lowercase();
        let mut best: Option<(&str, &ModelPricing)> = None;
        for (key, pricing) in &self.cache.models {
            let key_lower = key.to_lowercase();
            // Skip provider-prefixed keys (azure/, openrouter/, etc.)
            if key_lower.contains('/') {
                continue;
            }
            if !key_lower.is_empty()
                && lower.contains(&key_lower)
                && best.is_none_or(|(k, _)| key.len() > k.len())
            {
                best = Some((key, pricing));
            }
        }
        best.map(|(_, p)| p)
    }

    /// Force refresh pricing data
    #[allow(dead_code)]
    pub fn refresh(&mut self) -> Result<()> {
        let cache = Self::fetch_pricing()
            .map_err(|e| ToktrackError::Pricing(format!("Refresh failed: {}", e)))?;
        let _ = Self::save_cache(&self.cache_path, &cache);
        self.cache = cache;
        Ok(())
    }

    /// Get the number of models in the cache
    #[allow(dead_code)]
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
            fast_speed: false,
            timestamp: Utc::now(),
            model: model.map(String::from),
            input_tokens: input,
            output_tokens: output,
            cache_read_tokens: cache_read,
            cache_creation_tokens: cache_creation,
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
            provider: None,
            project: None,
        }
    }

    fn create_mock_cache(cache_path: &std::path::Path) {
        let mut models = HashMap::new();
        models.insert(
            "claude-sonnet-4".to_string(),
            ModelPricing {
                input_cost_per_token: Some(0.000003),
                output_cost_per_token: Some(0.000015),
                cache_read_input_token_cost: Some(0.0000003),
                cache_creation_input_token_cost: Some(0.00000375),
                ..Default::default()
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
        let content = serde_json::to_string_pretty(&cache).unwrap();
        fs::create_dir_all(cache_path.parent().unwrap()).unwrap();
        fs::write(cache_path, content).unwrap();
    }

    fn create_test_service() -> (PricingService, TempDir) {
        let temp_dir = TempDir::new().unwrap();
        let cache_path = temp_dir.path().join("pricing.json");

        let mut models = HashMap::new();
        models.insert(
            "claude-sonnet-4".to_string(),
            ModelPricing {
                input_cost_per_token: Some(0.000003),         // $3 per 1M tokens
                output_cost_per_token: Some(0.000015),        // $15 per 1M tokens
                cache_read_input_token_cost: Some(0.0000003), // $0.30 per 1M tokens
                cache_creation_input_token_cost: Some(0.00000375), // $3.75 per 1M tokens
                ..Default::default()
            },
        );
        models.insert(
            "claude-opus-4".to_string(),
            ModelPricing {
                input_cost_per_token: Some(0.000015),  // $15 per 1M tokens
                output_cost_per_token: Some(0.000075), // $75 per 1M tokens
                cache_read_input_token_cost: Some(0.0000015), // $1.50 per 1M tokens
                cache_creation_input_token_cost: Some(0.00001875), // $18.75 per 1M tokens
                ..Default::default()
            },
        );
        models.insert(
            "claude-opus-4-6".to_string(),
            ModelPricing {
                input_cost_per_token: Some(0.000015),  // $15 per 1M tokens
                output_cost_per_token: Some(0.000075), // $75 per 1M tokens
                cache_read_input_token_cost: Some(0.0000015), // $1.50 per 1M tokens
                cache_creation_input_token_cost: Some(0.00001875), // $18.75 per 1M tokens
                input_cost_per_token_above_200k_tokens: Some(0.00003), // $30 per 1M tokens
                output_cost_per_token_above_200k_tokens: Some(0.00015), // $150 per 1M tokens
                cache_read_input_token_cost_above_200k_tokens: Some(0.000003), // $3 per 1M tokens
                cache_creation_input_token_cost_above_200k_tokens: Some(0.0000375), // $37.50 per 1M tokens
                ..Default::default()
            },
        );
        models.insert(
            "claude-opus-5".to_string(),
            ModelPricing {
                input_cost_per_token: Some(0.000005),         // $5 per 1M tokens
                output_cost_per_token: Some(0.000025),        // $25 per 1M tokens
                cache_read_input_token_cost: Some(0.0000005), // $0.50 per 1M tokens
                cache_creation_input_token_cost: Some(0.00000625), // $6.25 per 1M tokens (5m)
                cache_creation_input_token_cost_above_1hr: Some(0.00001), // $10 per 1M tokens (1h)
                provider_specific_entry: Some(HashMap::from([
                    ("fast".to_string(), 2.0),
                    ("us".to_string(), 1.1),
                ])),
                search_context_cost_per_query: Some(SearchContextCost {
                    search_context_size_low: Some(0.01),
                    search_context_size_medium: Some(0.01),
                    search_context_size_high: Some(0.01),
                }),
                ..Default::default()
            },
        );
        models.insert(
            "claude-sonnet-4-5".to_string(),
            ModelPricing {
                input_cost_per_token: Some(0.000003),
                output_cost_per_token: Some(0.000015),
                cache_read_input_token_cost: Some(0.0000003),
                cache_creation_input_token_cost: Some(0.00000375), // $3.75 per 1M (5m)
                cache_creation_input_token_cost_above_1hr: Some(0.000006), // $6 per 1M (1h)
                cache_creation_input_token_cost_above_1hr_above_200k_tokens: Some(0.000012), // $12 per 1M
                ..Default::default()
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

        let content = serde_json::to_string_pretty(&cache).unwrap();
        fs::create_dir_all(cache_path.parent().unwrap()).unwrap();
        fs::write(&cache_path, content).unwrap();

        let service = PricingService::with_cache_path(cache_path).unwrap();
        (service, temp_dir)
    }

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
        // input_tokens is already non-cached (cache tokens are separate fields)
        //
        // Cost = (1000 * 0.000003) + (200 * 0.0000003) + (100 * 0.00000375) + (500 * 0.000015)
        //      = 0.003 + 0.00006 + 0.000375 + 0.0075
        //      = 0.010935
        let entry = make_entry(Some("claude-sonnet-4"), 1000, 500, 200, 100, None);

        let cost = service.calculate_cost(&entry);

        assert!(
            (cost - 0.010935).abs() < 1e-10,
            "Expected 0.010935, got {}",
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
    fn test_input_tokens_not_double_deducted() {
        let (service, _temp) = create_test_service();
        // input_tokens is already non-cached; cache_read is a separate field
        // input=100, cache_read=150, output=500
        let entry = make_entry(Some("claude-sonnet-4"), 100, 500, 150, 0, None);

        // cost = (100 * 0.000003) + (150 * 0.0000003) + (0 * cache_create) + (500 * 0.000015)
        //      = 0.0003 + 0.000045 + 0 + 0.0075
        //      = 0.007845
        let cost = service.calculate_cost(&entry);

        assert!(
            (cost - 0.007845).abs() < 1e-10,
            "Expected 0.007845, got {}",
            cost
        );
    }

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

    #[test]
    fn test_get_pricing_normalized_date_suffix() {
        let (service, _temp) = create_test_service();
        let pricing = service.get_pricing("claude-sonnet-4-20250514");

        assert!(pricing.is_some());
        let p = pricing.unwrap();
        assert!((p.input_cost_per_token.unwrap() - 0.000003).abs() < 1e-10);
    }

    #[test]
    fn test_get_pricing_normalized_dot_to_hyphen() {
        let (service, _temp) = create_test_service();
        let pricing = service.get_pricing("claude-opus-4");

        assert!(pricing.is_some());
    }

    fn create_fuzzy_test_service() -> (PricingService, TempDir) {
        let temp_dir = TempDir::new().unwrap();
        let cache_path = temp_dir.path().join("pricing.json");

        let mut models = HashMap::new();
        models.insert(
            "gpt-5".to_string(),
            ModelPricing {
                input_cost_per_token: Some(0.00001),
                output_cost_per_token: Some(0.00003),
                ..Default::default()
            },
        );
        models.insert(
            "gpt-5-2-codex".to_string(),
            ModelPricing {
                input_cost_per_token: Some(0.000005),
                output_cost_per_token: Some(0.000015),
                ..Default::default()
            },
        );
        models.insert(
            "o4-mini".to_string(),
            ModelPricing {
                input_cost_per_token: Some(0.000001),
                output_cost_per_token: Some(0.000004),
                ..Default::default()
            },
        );
        models.insert(
            "azure/gpt-5".to_string(),
            ModelPricing {
                input_cost_per_token: Some(0.00002),
                output_cost_per_token: Some(0.00006),
                ..Default::default()
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

        let content = serde_json::to_string_pretty(&cache).unwrap();
        fs::create_dir_all(cache_path.parent().unwrap()).unwrap();
        fs::write(&cache_path, content).unwrap();

        let service = PricingService::with_cache_path(cache_path).unwrap();
        (service, temp_dir)
    }

    #[test]
    fn test_fuzzy_pricing_fallback() {
        let (service, _temp) = create_fuzzy_test_service();
        // gpt-5.3-codex → normalized: gpt-5-3-codex
        // Not exact, not normalized match, but contains "gpt-5"
        let pricing = service.get_pricing("gpt-5.3-codex");
        assert!(pricing.is_some());
        // Should match "gpt-5" (len=5), not "gpt-5-2-codex" (doesn't match substring)
        let p = pricing.unwrap();
        assert!((p.input_cost_per_token.unwrap() - 0.00001).abs() < 1e-10);
    }

    #[test]
    fn test_fuzzy_pricing_prefers_exact() {
        let (service, _temp) = create_fuzzy_test_service();
        let pricing = service.get_pricing("gpt-5-2-codex");
        assert!(pricing.is_some());
        let p = pricing.unwrap();
        assert!((p.input_cost_per_token.unwrap() - 0.000005).abs() < 1e-10);
    }

    #[test]
    fn test_fuzzy_pricing_skips_provider_prefixed() {
        let (service, _temp) = create_fuzzy_test_service();
        // "azure/gpt-5" should be skipped in fuzzy matching
        // "gpt-5.3-codex" should match "gpt-5", not "azure/gpt-5"
        let pricing = service.get_pricing("gpt-5.3-codex");
        assert!(pricing.is_some());
        let p = pricing.unwrap();
        // Should be "gpt-5" pricing ($0.00001), not "azure/gpt-5" ($0.00002)
        assert!((p.input_cost_per_token.unwrap() - 0.00001).abs() < 1e-10);
    }

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
                ..Default::default()
            },
        );

        let cache = PricingCache {
            fetched_at: 12345,
            models,
        };

        PricingService::save_cache(&cache_path, &cache).unwrap();

        let loaded = PricingService::load_cache(&cache_path).unwrap();

        assert_eq!(loaded.fetched_at, 12345);
        assert!(loaded.models.contains_key("test-model"));
    }

    #[test]
    fn test_model_count() {
        let (service, _temp) = create_test_service();

        assert_eq!(service.model_count(), 5);
    }

    /// Guards the reason `GrokParser` sets `cost_usd` itself instead of relying
    /// on this table: LiteLLM ships Grok only under provider-prefixed keys, and
    /// `get_pricing`'s substring fallback deliberately skips keys containing
    /// `/`, so a bare `grok-4.6` resolves to nothing and would be priced at $0.
    /// If LiteLLM ever adds a bare key, this fails and the parser could go back to
    /// the pricing table — except for the per-call tier split, which it still cannot
    /// reproduce.
    #[test]
    fn test_bare_grok_id_is_unreachable_in_pricing_table() {
        let cache = PricingService::snapshot_cache();
        let service = PricingService {
            cache,
            cache_path: PathBuf::from("/dev/null"),
            custom: None,
        };

        // The provider-prefixed key exists and prices.
        assert!(
            service.get_pricing("xai/grok-4.6").is_some(),
            "snapshot should carry xai/grok-4.6"
        );
        // The id Grok actually reports does not resolve, in either spelling.
        assert!(
            service.get_pricing("grok-4.6").is_none(),
            "bare grok-4.6 unexpectedly resolved - recheck whether GrokParser still needs to set cost_usd from ticks"
        );
        assert!(
            service.get_pricing("grok-4-6").is_none(),
            "normalized grok-4-6 unexpectedly resolved - recheck whether GrokParser still needs to set cost_usd from ticks"
        );

        // ...which is why the table path would report $0.00 for a real entry.
        let entry = UsageEntry {
            timestamp: Utc::now(),
            model: Some("grok-4.6".to_string()),
            input_tokens: 21214,
            output_tokens: 1841,
            cache_read_tokens: 205824,
            cache_creation_tokens: 0,
            reasoning_tokens: 612,
            cache_creation_5m_tokens: 0,
            cache_creation_1h_tokens: 0,
            web_search_requests: 0,
            web_fetch_requests: 0,
            reported_total_tokens: Some(229491),
            cost_usd: None,
            message_id: None,
            request_id: None,
            source: Some("grok".into()),
            provider: Some("xai".into()),
            project: None,
            fast_speed: false,
        };
        assert_eq!(service.calculate_cost(&entry), 0.0);

        // The parser-supplied figure is used instead, and is the exact amount.
        let priced = UsageEntry {
            cost_usd: Some(0.160058),
            ..entry
        };
        assert!((service.get_or_calculate_cost(&priced) - 0.160058).abs() < 1e-9);
    }

    #[test]
    fn test_vendored_snapshot_parses_and_prices_known_models() {
        let cache = PricingService::snapshot_cache();
        assert!(
            cache.models.len() > 1000,
            "snapshot should carry the bulk of LiteLLM models, got {}",
            cache.models.len()
        );
        let service = PricingService {
            cache,
            cache_path: PathBuf::from("/dev/null"),
            custom: None,
        };
        // exact, dotted-key, and date-suffix-normalized resolution against the real snapshot
        assert!(service.get_pricing("claude-opus-4-5").is_some());
        assert!(service.get_pricing("gemini-2.5-pro").is_some());
        assert!(service.get_pricing("gpt-5").is_some());
        assert!(
            service.get_pricing("claude-opus-4-5-20251101").is_some(),
            "date-suffix should normalize to a snapshot key"
        );
    }

    #[test]
    fn test_from_cache_only_with_valid_cache() {
        let (_, temp_dir) = create_test_service();
        let cache_path = temp_dir.path().join("pricing.json");

        let service = PricingService::from_cache_only_with_path(&cache_path);
        assert!(service.is_some());
        assert_eq!(service.unwrap().model_count(), 5);
    }

    #[test]
    fn test_from_cache_only_no_cache_returns_none() {
        let temp_dir = TempDir::new().unwrap();
        let cache_path = temp_dir.path().join("nonexistent.json");

        let service = PricingService::from_cache_only_with_path(&cache_path);
        assert!(service.is_none());
    }

    #[test]
    fn test_from_cache_only_uses_expired_cache() {
        let temp_dir = TempDir::new().unwrap();
        let cache_path = temp_dir.path().join("pricing.json");

        let mut models = HashMap::new();
        models.insert("test-model".to_string(), ModelPricing::default());
        let cache = PricingCache {
            fetched_at: 0,
            models,
        };
        let content = serde_json::to_string_pretty(&cache).unwrap();
        fs::write(&cache_path, content).unwrap();

        let service = PricingService::from_cache_only_with_path(&cache_path);
        assert!(service.is_some());
        assert_eq!(service.unwrap().model_count(), 1);
    }

    #[test]
    fn test_from_cache_only_corrupt_cache_returns_none() {
        let temp_dir = TempDir::new().unwrap();
        let cache_path = temp_dir.path().join("pricing.json");

        fs::write(&cache_path, "not valid json{{{").unwrap();

        let service = PricingService::from_cache_only_with_path(&cache_path);
        assert!(service.is_none());
    }

    #[test]
    fn test_tiered_cost_below_threshold() {
        // 100k tokens at $15/1M → all at base rate
        let cost = tiered_cost(100_000, 0.000015, Some((200_000, 0.00003)));
        let expected = 100_000.0 * 0.000015;
        assert!(
            (cost - expected).abs() < 1e-10,
            "Expected {}, got {}",
            expected,
            cost
        );
    }

    #[test]
    fn test_tiered_cost_at_threshold() {
        // Exactly 200k tokens → all at base rate (threshold is inclusive)
        let cost = tiered_cost(200_000, 0.000015, Some((200_000, 0.00003)));
        let expected = 200_000.0 * 0.000015;
        assert!(
            (cost - expected).abs() < 1e-10,
            "Expected {}, got {}",
            expected,
            cost
        );
    }

    #[test]
    fn test_tiered_cost_above_threshold() {
        // 300k tokens: 200k at base ($15/1M), 100k at tiered ($30/1M)
        let cost = tiered_cost(300_000, 0.000015, Some((200_000, 0.00003)));
        let expected = 200_000.0 * 0.000015 + 100_000.0 * 0.00003;
        assert!(
            (cost - expected).abs() < 1e-10,
            "Expected {}, got {}",
            expected,
            cost
        );
    }

    #[test]
    fn test_tiered_cost_just_above_threshold() {
        // 200,001 tokens: 200k at base, 1 token at tiered
        let cost = tiered_cost(200_001, 0.000015, Some((200_000, 0.00003)));
        let expected = 200_000.0 * 0.000015 + 1.0 * 0.00003;
        assert!(
            (cost - expected).abs() < 1e-10,
            "Expected {}, got {}",
            expected,
            cost
        );
    }

    #[test]
    fn test_tiered_cost_none_tiered_price_uses_base() {
        // No tiered price → all tokens at base rate even above threshold
        let cost = tiered_cost(300_000, 0.000015, None);
        let expected = 300_000.0 * 0.000015;
        assert!(
            (cost - expected).abs() < 1e-10,
            "Expected {}, got {}",
            expected,
            cost
        );
    }

    #[test]
    fn test_tiered_cost_zero_tokens() {
        let cost = tiered_cost(0, 0.000015, Some((200_000, 0.00003)));
        assert!((cost - 0.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_tier_resolver_precedence() {
        assert_eq!(
            tier(Some(0.1), Some(0.2), Some(0.3), Some(0.4)),
            Some((128_000, 0.1))
        );
        assert_eq!(tier(None, None, None, Some(0.4)), Some((272_000, 0.4)));
        assert_eq!(tier(None, Some(0.2), None, None), Some((200_000, 0.2)));
        assert_eq!(tier(None, None, None, None), None);
    }

    #[test]
    fn test_tiered_cost_128k_breakpoint() {
        // 200k tokens at base $15/1M up to 128k, then $30/1M above
        let cost = tiered_cost(200_000, 0.000015, Some((128_000, 0.00003)));
        let expected = 128_000.0 * 0.000015 + 72_000.0 * 0.00003;
        assert!(
            (cost - expected).abs() < 1e-12,
            "expected {}, got {}",
            expected,
            cost
        );
    }

    #[test]
    fn test_tiered_cost_272k_breakpoint() {
        // 300k tokens: 272k at base, 28k above
        let cost = tiered_cost(300_000, 0.000015, Some((272_000, 0.00003)));
        let expected = 272_000.0 * 0.000015 + 28_000.0 * 0.00003;
        assert!(
            (cost - expected).abs() < 1e-12,
            "expected {}, got {}",
            expected,
            cost
        );
    }

    #[test]
    fn test_calculate_cost_uses_272k_breakpoint() {
        let temp_dir = TempDir::new().unwrap();
        let cache_path = temp_dir.path().join("pricing.json");
        write_cache_json(
            &cache_path,
            r#"{
                "gemini-3-pro": {
                    "input_cost_per_token": 0.000002,
                    "input_cost_per_token_above_272k_tokens": 0.000004
                }
            }"#,
        );
        let service = PricingService::from_cache_only_with_path(&cache_path).unwrap();
        let entry = make_entry(Some("gemini-3-pro"), 300_000, 0, 0, 0, None);
        let cost = service.calculate_cost(&entry);
        // 272k * $2/1M + 28k * $4/1M
        let expected = 272_000.0 * 0.000002 + 28_000.0 * 0.000004;
        assert!(
            (cost - expected).abs() < 1e-12,
            "expected {}, got {}",
            expected,
            cost
        );
    }

    #[test]
    fn test_calculate_cost_tiered_model_below_threshold() {
        let (service, _temp) = create_test_service();
        // claude-opus-4-6 with 100k input, 50k output → all below threshold, uses base rate
        let entry = make_entry(Some("claude-opus-4-6"), 100_000, 50_000, 0, 0, None);

        let cost = service.calculate_cost(&entry);

        // input: 100k * $15/1M = $1.50
        // output: 50k * $75/1M = $3.75
        let expected = 100_000.0 * 0.000015 + 50_000.0 * 0.000075;
        assert!(
            (cost - expected).abs() < 1e-10,
            "Expected {}, got {}",
            expected,
            cost
        );
    }

    #[test]
    fn test_calculate_cost_tiered_model_above_threshold() {
        let (service, _temp) = create_test_service();
        // claude-opus-4-6: 300k input, 250k output, 300k cache_read, 500k cache_creation
        let entry = make_entry(
            Some("claude-opus-4-6"),
            300_000,
            250_000,
            300_000,
            500_000,
            None,
        );

        let cost = service.calculate_cost(&entry);

        // input: 200k * $15/1M + 100k * $30/1M = $3.00 + $3.00 = $6.00
        let input = 200_000.0 * 0.000015 + 100_000.0 * 0.00003;
        // output: 200k * $75/1M + 50k * $150/1M = $15.00 + $7.50 = $22.50
        let output = 200_000.0 * 0.000075 + 50_000.0 * 0.00015;
        // cache_read: 200k * $1.50/1M + 100k * $3/1M = $0.30 + $0.30 = $0.60
        let cache_read = 200_000.0 * 0.0000015 + 100_000.0 * 0.000003;
        // cache_creation: 200k * $18.75/1M + 300k * $37.50/1M = $3.75 + $11.25 = $15.00
        let cache_creation = 200_000.0 * 0.00001875 + 300_000.0 * 0.0000375;
        let expected = input + output + cache_read + cache_creation;

        assert!(
            (cost - expected).abs() < 1e-10,
            "Expected {}, got {}",
            expected,
            cost
        );
    }

    #[test]
    fn test_calculate_cost_non_tiered_model_flat_rate() {
        let (service, _temp) = create_test_service();
        // claude-sonnet-4 has no tiered pricing → uses flat rate even above 200k
        let entry = make_entry(Some("claude-sonnet-4"), 300_000, 0, 0, 0, None);

        let cost = service.calculate_cost(&entry);

        // All 300k at base rate: 300k * $3/1M = $0.90
        let expected = 300_000.0 * 0.000003;
        assert!(
            (cost - expected).abs() < 1e-10,
            "Expected {}, got {}",
            expected,
            cost
        );
    }

    #[test]
    fn test_custom_pricing_overrides_litellm() {
        let temp_dir = TempDir::new().unwrap();
        let cache_path = temp_dir.path().join("pricing.json");
        create_mock_cache(&cache_path);

        let custom = CustomPricingConfig {
            models: Some(HashMap::from([(
                "claude-sonnet-4".to_string(),
                CustomModelPricing {
                    input: Some(5.0),     // $5/1M (vs LiteLLM $3/1M)
                    output: Some(20.0),   // $20/1M (vs LiteLLM $15/1M)
                    cache_creation: None, // fallback to LiteLLM: not used since custom takes full precedence
                    cache_read: None,
                    cache_creation_5m: None,
                    cache_creation_1h: None,
                    input_above_200k: None,
                    output_above_200k: None,
                    cache_read_above_200k: None,
                    cache_creation_above_200k: None,
                },
            )])),
            global: None,
        };

        let service = PricingService::from_cache_only_with_path(&cache_path)
            .unwrap()
            .with_custom_pricing(Some(custom));

        let entry = make_entry(Some("claude-sonnet-4"), 1_000_000, 500_000, 0, 0, None);
        let cost = service.calculate_cost(&entry);

        // Custom: input=$5/1M * 1M + output=$20/1M * 0.5M = $5 + $10 = $15
        let expected = 5.0 + 10.0;
        assert!(
            (cost - expected).abs() < 1e-6,
            "Custom pricing override failed: expected {}, got {}",
            expected,
            cost
        );
    }

    #[test]
    fn test_custom_pricing_no_model_falls_back_to_litellm() {
        let temp_dir = TempDir::new().unwrap();
        let cache_path = temp_dir.path().join("pricing.json");
        create_mock_cache(&cache_path);

        let custom = CustomPricingConfig {
            models: Some(HashMap::from([(
                "some-other-model".to_string(),
                CustomModelPricing {
                    input: Some(99.0),
                    output: None,
                    cache_creation: None,
                    cache_read: None,
                    cache_creation_5m: None,
                    cache_creation_1h: None,
                    input_above_200k: None,
                    output_above_200k: None,
                    cache_read_above_200k: None,
                    cache_creation_above_200k: None,
                },
            )])),
            global: None,
        };

        let service = PricingService::from_cache_only_with_path(&cache_path)
            .unwrap()
            .with_custom_pricing(Some(custom));

        let entry = make_entry(Some("claude-sonnet-4"), 1_000_000, 0, 0, 0, None);
        let cost = service.calculate_cost(&entry);

        // LiteLLM: $3/1M * 1M = $3
        let expected = 3.0;
        assert!(
            (cost - expected).abs() < 1e-6,
            "Fallback to LiteLLM failed: expected {}, got {}",
            expected,
            cost
        );
    }

    #[test]
    fn test_custom_pricing_cache_ttl_tiers() {
        let temp_dir = TempDir::new().unwrap();
        let cache_path = temp_dir.path().join("pricing.json");
        create_mock_cache(&cache_path);

        let custom = CustomPricingConfig {
            models: Some(HashMap::from([(
                "claude-sonnet-4".to_string(),
                CustomModelPricing {
                    input: Some(3.0),
                    output: Some(15.0),
                    cache_creation: Some(3.75),
                    cache_read: Some(0.30),
                    cache_creation_5m: Some(3.75), // same as base
                    cache_creation_1h: Some(6.0),  // 1.6x multiplier
                    input_above_200k: None,
                    output_above_200k: None,
                    cache_read_above_200k: None,
                    cache_creation_above_200k: None,
                },
            )])),
            global: None,
        };

        let service = PricingService::from_cache_only_with_path(&cache_path)
            .unwrap()
            .with_custom_pricing(Some(custom));

        let mut entry = make_entry(Some("claude-sonnet-4"), 0, 0, 0, 0, None);
        entry.cache_creation_5m_tokens = 100_000;
        entry.cache_creation_1h_tokens = 100_000;

        let cost = service.calculate_cost(&entry);

        // 5m: 100k * $3.75/1M = $0.375
        // 1h: 100k * $6.0/1M  = $0.60
        let expected = 0.375 + 0.60;
        assert!(
            (cost - expected).abs() < 1e-6,
            "TTL tier pricing failed: expected {}, got {}",
            expected,
            cost
        );
    }

    #[test]
    fn test_ttl_pricing_falls_back_to_flat_for_historical_entries() {
        // Historical entries have cache_creation_tokens but no TTL breakdown
        let temp_dir = TempDir::new().unwrap();
        let cache_path = temp_dir.path().join("pricing.json");
        create_mock_cache(&cache_path);

        let custom = CustomPricingConfig {
            models: Some(HashMap::from([(
                "claude-sonnet-4".to_string(),
                CustomModelPricing {
                    input: Some(3.0),
                    output: Some(15.0),
                    cache_creation: Some(3.75),
                    cache_read: Some(0.30),
                    cache_creation_5m: Some(3.75),
                    cache_creation_1h: Some(6.0),
                    input_above_200k: None,
                    output_above_200k: None,
                    cache_read_above_200k: None,
                    cache_creation_above_200k: None,
                },
            )])),
            global: None,
        };

        let service = PricingService::from_cache_only_with_path(&cache_path)
            .unwrap()
            .with_custom_pricing(Some(custom));

        let entry = make_entry(Some("claude-sonnet-4"), 0, 0, 0, 100_000, None);
        let cost = service.calculate_cost(&entry);

        let expected = 100_000.0 * (3.75 / 1_000_000.0);
        assert!(
            (cost - expected).abs() < 1e-6,
            "Historical entry should use flat cache_creation rate, expected {}, got {}",
            expected,
            cost
        );
    }

    #[test]
    fn test_web_search_cost_with_global_override() {
        let temp_dir = TempDir::new().unwrap();
        let cache_path = temp_dir.path().join("pricing.json");
        create_mock_cache(&cache_path);

        let custom = CustomPricingConfig {
            models: None,
            global: Some(GlobalPricing {
                web_search_per_request: Some(0.02), // $0.02 per search
                web_fetch_per_request: None,
            }),
        };

        let service = PricingService::from_cache_only_with_path(&cache_path)
            .unwrap()
            .with_custom_pricing(Some(custom));

        let mut entry = make_entry(Some("claude-sonnet-4"), 0, 0, 0, 0, None);
        entry.web_search_requests = 5;

        let cost = service.calculate_cost(&entry);

        // 5 searches * $0.02 = $0.10
        let expected = 0.10;
        assert!(
            (cost - expected).abs() < 1e-6,
            "Web search cost failed: expected {}, got {}",
            expected,
            cost
        );
    }

    #[test]
    fn test_no_custom_pricing_uses_litellm_only() {
        let temp_dir = TempDir::new().unwrap();
        let cache_path = temp_dir.path().join("pricing.json");
        create_mock_cache(&cache_path);

        let service = PricingService::from_cache_only_with_path(&cache_path).unwrap();

        let entry = make_entry(Some("claude-sonnet-4"), 1_000, 500, 200, 100, None);
        let cost = service.calculate_cost(&entry);

        // LiteLLM: input=$3/1M, output=$15/1M, cache_read=$0.30/1M, cache_creation=$3.75/1M
        let expected =
            1_000.0 * 0.000003 + 500.0 * 0.000015 + 200.0 * 0.0000003 + 100.0 * 0.00000375;

        assert!(
            (cost - expected).abs() < 1e-10,
            "LiteLLM-only cost failed: expected {}, got {}",
            expected,
            cost
        );
    }

    #[test]
    fn test_custom_pricing_toml_parsing() {
        let toml_str = r#"
[models."claude-opus-4-5-20251101"]
input = 15.0
output = 75.0
cache_creation = 18.75
cache_read = 0.30
cache_creation_5m = 18.75
cache_creation_1h = 30.0

[models."claude-sonnet-4-20250514"]
input = 3.0
output = 15.0

[global]
web_search_per_request = 0.01
"#;

        let config: CustomPricingConfig = toml::from_str(toml_str).unwrap();

        let models = config.models.unwrap();
        assert_eq!(models.len(), 2);

        let opus = &models["claude-opus-4-5-20251101"];
        assert_eq!(opus.input, Some(15.0));
        assert_eq!(opus.cache_creation_1h, Some(30.0));

        let sonnet = &models["claude-sonnet-4-20250514"];
        assert_eq!(sonnet.input, Some(3.0));
        assert_eq!(sonnet.cache_creation_5m, None);

        let global = config.global.unwrap();
        assert_eq!(global.web_search_per_request, Some(0.01));
    }

    #[test]
    fn test_load_custom_pricing_from_env_var() {
        let temp_dir = TempDir::new().unwrap();
        let toml_path = temp_dir.path().join("custom-pricing.toml");
        fs::write(
            &toml_path,
            r#"
[models."test-model"]
input = 10.0
output = 50.0
"#,
        )
        .unwrap();

        std::env::set_var("TOKTRACK_PRICING_FILE", toml_path.to_str().unwrap());
        let config = PricingService::load_custom_pricing();
        std::env::remove_var("TOKTRACK_PRICING_FILE");

        let config = config.expect("Should load from env var path");
        let models = config.models.unwrap();
        let model = &models["test-model"];
        assert_eq!(model.input, Some(10.0));
        assert_eq!(model.output, Some(50.0));
    }

    #[test]
    fn test_env_var_invalid_path_returns_none() {
        std::env::set_var("TOKTRACK_PRICING_FILE", "/nonexistent/path/pricing.toml");
        let config = PricingService::load_custom_pricing();
        std::env::remove_var("TOKTRACK_PRICING_FILE");

        let _ = config;
    }

    #[test]
    fn test_custom_tiered_pricing_above_200k() {
        let temp_dir = TempDir::new().unwrap();
        let cache_path = temp_dir.path().join("pricing.json");
        create_mock_cache(&cache_path);

        let custom = CustomPricingConfig {
            models: Some(HashMap::from([(
                "custom-tiered-model".to_string(),
                CustomModelPricing {
                    input: Some(3.0),   // $3/1M base
                    output: Some(15.0), // $15/1M base
                    cache_creation: Some(3.75),
                    cache_read: Some(0.30),
                    cache_creation_5m: None,
                    cache_creation_1h: None,
                    input_above_200k: Some(6.0),   // $6/1M above 200k
                    output_above_200k: Some(30.0), // $30/1M above 200k
                    cache_read_above_200k: Some(0.60),
                    cache_creation_above_200k: Some(7.50),
                },
            )])),
            global: None,
        };

        let service = PricingService::from_cache_only_with_path(&cache_path)
            .unwrap()
            .with_custom_pricing(Some(custom));

        // 300k input: 200k at $3/1M + 100k at $6/1M = $0.60 + $0.60 = $1.20
        let entry = make_entry(Some("custom-tiered-model"), 300_000, 0, 0, 0, None);
        let cost = service.calculate_cost(&entry);

        let expected = 200_000.0 * 0.000003 + 100_000.0 * 0.000006;
        assert!(
            (cost - expected).abs() < 1e-10,
            "Custom tiered pricing failed: expected {}, got {}",
            expected,
            cost
        );
    }

    #[test]
    fn test_custom_tiered_pricing_all_token_types() {
        let temp_dir = TempDir::new().unwrap();
        let cache_path = temp_dir.path().join("pricing.json");
        create_mock_cache(&cache_path);

        let custom = CustomPricingConfig {
            models: Some(HashMap::from([(
                "tiered-all".to_string(),
                CustomModelPricing {
                    input: Some(15.0),
                    output: Some(75.0),
                    cache_creation: Some(18.75),
                    cache_read: Some(1.50),
                    cache_creation_5m: None,
                    cache_creation_1h: None,
                    input_above_200k: Some(30.0),
                    output_above_200k: Some(150.0),
                    cache_read_above_200k: Some(3.0),
                    cache_creation_above_200k: Some(37.50),
                },
            )])),
            global: None,
        };

        let service = PricingService::from_cache_only_with_path(&cache_path)
            .unwrap()
            .with_custom_pricing(Some(custom));

        // 300k input, 250k output, 300k cache_read, 500k cache_creation
        let entry = make_entry(Some("tiered-all"), 300_000, 250_000, 300_000, 500_000, None);
        let cost = service.calculate_cost(&entry);

        let input = 200_000.0 * 0.000015 + 100_000.0 * 0.00003;
        let output = 200_000.0 * 0.000075 + 50_000.0 * 0.00015;
        let cache_read = 200_000.0 * 0.0000015 + 100_000.0 * 0.000003;
        let cache_creation = 200_000.0 * 0.00001875 + 300_000.0 * 0.0000375;
        let expected = input + output + cache_read + cache_creation;

        assert!(
            (cost - expected).abs() < 1e-6,
            "Custom tiered all types failed: expected {}, got {}",
            expected,
            cost
        );
    }

    #[test]
    fn test_custom_no_tiered_fields_uses_flat_rate() {
        let temp_dir = TempDir::new().unwrap();
        let cache_path = temp_dir.path().join("pricing.json");
        create_mock_cache(&cache_path);

        let custom = CustomPricingConfig {
            models: Some(HashMap::from([(
                "flat-only".to_string(),
                CustomModelPricing {
                    input: Some(3.0),
                    output: Some(15.0),
                    cache_creation: None,
                    cache_read: None,
                    cache_creation_5m: None,
                    cache_creation_1h: None,
                    input_above_200k: None,
                    output_above_200k: None,
                    cache_read_above_200k: None,
                    cache_creation_above_200k: None,
                },
            )])),
            global: None,
        };

        let service = PricingService::from_cache_only_with_path(&cache_path)
            .unwrap()
            .with_custom_pricing(Some(custom));

        let entry = make_entry(Some("flat-only"), 300_000, 0, 0, 0, None);
        let cost = service.calculate_cost(&entry);

        // All 300k at flat rate: $3/1M * 300k = $0.90
        let expected = 300_000.0 * 0.000003;
        assert!(
            (cost - expected).abs() < 1e-10,
            "Flat rate fallback failed: expected {}, got {}",
            expected,
            cost
        );
    }

    #[test]
    fn test_custom_pricing_toml_parsing_with_tiered() {
        let toml_str = r#"
[models."claude-sonnet-4-5-20250514"]
input = 3.0
output = 15.0
cache_creation = 3.75
cache_read = 0.30
input_above_200k = 6.0
output_above_200k = 30.0
cache_read_above_200k = 0.60
cache_creation_above_200k = 7.50

[global]
web_search_per_request = 0.01
"#;

        let config: CustomPricingConfig = toml::from_str(toml_str).unwrap();
        let models = config.models.unwrap();
        let sonnet = &models["claude-sonnet-4-5-20250514"];
        assert_eq!(sonnet.input, Some(3.0));
        assert_eq!(sonnet.input_above_200k, Some(6.0));
        assert_eq!(sonnet.output_above_200k, Some(30.0));
        assert_eq!(sonnet.cache_read_above_200k, Some(0.60));
        assert_eq!(sonnet.cache_creation_above_200k, Some(7.50));
    }

    /// Write a pricing cache file with a raw JSON `models` object (so tests can
    /// exercise LiteLLM field names without going through the typed struct).
    fn write_cache_json(cache_path: &std::path::Path, models_json: &str) {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;
        let content = format!(r#"{{"fetched_at":{now},"models":{models_json}}}"#);
        fs::create_dir_all(cache_path.parent().unwrap()).unwrap();
        fs::write(cache_path, content).unwrap();
    }

    #[test]
    fn test_web_search_cost_from_search_context_cost_per_query() {
        let temp_dir = TempDir::new().unwrap();
        let cache_path = temp_dir.path().join("pricing.json");
        write_cache_json(
            &cache_path,
            r#"{
                "claude-opus-4-1": {
                    "input_cost_per_token": 0.000015,
                    "output_cost_per_token": 0.000075,
                    "search_context_cost_per_query": {
                        "search_context_size_low": 0.01,
                        "search_context_size_medium": 0.01,
                        "search_context_size_high": 0.01
                    }
                }
            }"#,
        );
        let service = PricingService::from_cache_only_with_path(&cache_path).unwrap();
        let mut entry = make_entry(Some("claude-opus-4-1"), 0, 0, 0, 0, None);
        entry.web_search_requests = 2;

        let cost = service.calculate_cost(&entry);

        // 2 requests * $0.01 (medium tier) = $0.02
        assert!((cost - 0.02).abs() < 1e-9, "expected 0.02, got {}", cost);
    }

    #[test]
    fn test_web_search_cost_medium_falls_back_to_low_then_high() {
        let temp_dir = TempDir::new().unwrap();
        let cache_path = temp_dir.path().join("pricing.json");
        write_cache_json(
            &cache_path,
            r#"{
                "model-low-only": {
                    "search_context_cost_per_query": { "search_context_size_low": 0.05 }
                }
            }"#,
        );
        let service = PricingService::from_cache_only_with_path(&cache_path).unwrap();
        let mut entry = make_entry(Some("model-low-only"), 0, 0, 0, 0, None);
        entry.web_search_requests = 3;

        let cost = service.calculate_cost(&entry);

        // 3 * $0.05 (low fallback) = $0.15
        assert!((cost - 0.15).abs() < 1e-9, "expected 0.15, got {}", cost);
    }

    #[test]
    fn test_custom_global_web_search_overrides_search_context() {
        let temp_dir = TempDir::new().unwrap();
        let cache_path = temp_dir.path().join("pricing.json");
        write_cache_json(
            &cache_path,
            r#"{
                "claude-opus-4-1": {
                    "search_context_cost_per_query": { "search_context_size_medium": 0.01 }
                }
            }"#,
        );
        let custom = CustomPricingConfig {
            models: None,
            global: Some(GlobalPricing {
                web_search_per_request: Some(0.03),
                web_fetch_per_request: None,
            }),
        };
        let service = PricingService::from_cache_only_with_path(&cache_path)
            .unwrap()
            .with_custom_pricing(Some(custom));
        let mut entry = make_entry(Some("claude-opus-4-1"), 0, 0, 0, 0, None);
        entry.web_search_requests = 2;

        let cost = service.calculate_cost(&entry);

        // custom global $0.03 wins over LiteLLM medium $0.01 → 2 * 0.03 = 0.06
        assert!((cost - 0.06).abs() < 1e-9, "expected 0.06, got {}", cost);
    }

    #[test]
    fn test_web_fetch_zero_without_override_and_priced_with_override() {
        let temp_dir = TempDir::new().unwrap();
        let cache_path = temp_dir.path().join("pricing.json");
        create_mock_cache(&cache_path);

        let service = PricingService::from_cache_only_with_path(&cache_path).unwrap();
        let mut entry = make_entry(Some("claude-sonnet-4"), 0, 0, 0, 0, None);
        entry.web_fetch_requests = 4;
        assert!(
            service.calculate_cost(&entry).abs() < 1e-12,
            "web fetch must be free without a custom override"
        );

        let custom = CustomPricingConfig {
            models: None,
            global: Some(GlobalPricing {
                web_search_per_request: None,
                web_fetch_per_request: Some(0.01),
            }),
        };
        let service = PricingService::from_cache_only_with_path(&cache_path)
            .unwrap()
            .with_custom_pricing(Some(custom));
        let cost = service.calculate_cost(&entry);
        assert!((cost - 0.04).abs() < 1e-12, "expected 0.04, got {}", cost);
    }

    #[test]
    fn test_no_search_context_field_yields_zero_web_search_cost() {
        let temp_dir = TempDir::new().unwrap();
        let cache_path = temp_dir.path().join("pricing.json");
        write_cache_json(
            &cache_path,
            r#"{ "plain-model": { "input_cost_per_token": 0.000001 } }"#,
        );
        let service = PricingService::from_cache_only_with_path(&cache_path).unwrap();
        let mut entry = make_entry(Some("plain-model"), 0, 0, 0, 0, None);
        entry.web_search_requests = 5;

        let cost = service.calculate_cost(&entry);

        assert!(
            (cost - 0.0).abs() < f64::EPSILON,
            "expected 0.0, got {}",
            cost
        );
    }

    #[test]
    fn test_reasoning_tokens_priced_at_output_rate_by_default() {
        let temp_dir = TempDir::new().unwrap();
        let cache_path = temp_dir.path().join("pricing.json");
        write_cache_json(
            &cache_path,
            r#"{
                "gemini-2-5-pro": {
                    "input_cost_per_token": 0.00000125,
                    "output_cost_per_token": 0.00001
                }
            }"#,
        );
        let service = PricingService::from_cache_only_with_path(&cache_path).unwrap();
        let mut entry = make_entry(Some("gemini-2-5-pro"), 1000, 500, 0, 0, None);
        entry.reasoning_tokens = 2000;

        let cost = service.calculate_cost(&entry);

        // input 1000*1.25e-6 + output 500*1e-5 + thinking 2000*1e-5(output rate)
        let expected = 1000.0 * 0.00000125 + 500.0 * 0.00001 + 2000.0 * 0.00001;
        assert!(
            (cost - expected).abs() < 1e-12,
            "expected {}, got {}",
            expected,
            cost
        );
    }

    #[test]
    fn test_reasoning_tokens_priced_at_reasoning_rate_when_present() {
        let temp_dir = TempDir::new().unwrap();
        let cache_path = temp_dir.path().join("pricing.json");
        write_cache_json(
            &cache_path,
            r#"{
                "reasoner": {
                    "input_cost_per_token": 0.000001,
                    "output_cost_per_token": 0.00001,
                    "output_cost_per_reasoning_token": 0.00002
                }
            }"#,
        );
        let service = PricingService::from_cache_only_with_path(&cache_path).unwrap();
        let mut entry = make_entry(Some("reasoner"), 0, 0, 0, 0, None);
        entry.reasoning_tokens = 1000;

        let cost = service.calculate_cost(&entry);

        // thinking 1000 * reasoning rate 2e-5 = 0.02 (NOT output rate 1e-5)
        assert!((cost - 0.02).abs() < 1e-12, "expected 0.02, got {}", cost);
    }

    #[test]
    fn test_thinking_zero_leaves_cost_unchanged() {
        let temp_dir = TempDir::new().unwrap();
        let cache_path = temp_dir.path().join("pricing.json");
        write_cache_json(
            &cache_path,
            r#"{ "claude-x": { "input_cost_per_token": 0.000003, "output_cost_per_token": 0.000015 } }"#,
        );
        let service = PricingService::from_cache_only_with_path(&cache_path).unwrap();
        let entry = make_entry(Some("claude-x"), 1000, 500, 0, 0, None);

        let cost = service.calculate_cost(&entry);

        let expected = 1000.0 * 0.000003 + 500.0 * 0.000015;
        assert!(
            (cost - expected).abs() < 1e-12,
            "expected {}, got {}",
            expected,
            cost
        );
    }

    #[test]
    fn test_thinking_with_no_output_or_reasoning_rate_is_zero() {
        // Neither output_cost_per_reasoning_token nor output_cost_per_token present
        // → reasoning rate falls back to 0.0 (no panic / NaN).
        let temp_dir = TempDir::new().unwrap();
        let cache_path = temp_dir.path().join("pricing.json");
        write_cache_json(
            &cache_path,
            r#"{ "rate-less": { "input_cost_per_token": 0.000001 } }"#,
        );
        let service = PricingService::from_cache_only_with_path(&cache_path).unwrap();
        let mut entry = make_entry(Some("rate-less"), 0, 0, 0, 0, None);
        entry.reasoning_tokens = 5000;

        let cost = service.calculate_cost(&entry);

        assert!(
            (cost - 0.0).abs() < f64::EPSILON,
            "expected 0.0, got {}",
            cost
        );
    }

    #[test]
    fn test_search_context_present_but_all_tiers_empty_is_zero() {
        // search_context_cost_per_query object with no tier values → per_query_cost None → 0.
        let temp_dir = TempDir::new().unwrap();
        let cache_path = temp_dir.path().join("pricing.json");
        write_cache_json(
            &cache_path,
            r#"{ "empty-tiers": { "search_context_cost_per_query": {} } }"#,
        );
        let service = PricingService::from_cache_only_with_path(&cache_path).unwrap();
        let mut entry = make_entry(Some("empty-tiers"), 0, 0, 0, 0, None);
        entry.web_search_requests = 4;

        let cost = service.calculate_cost(&entry);

        assert!(
            (cost - 0.0).abs() < f64::EPSILON,
            "expected 0.0, got {}",
            cost
        );
    }

    #[test]
    fn test_cache_creation_1h_uses_above_1hr_rate() {
        let (service, _temp) = create_test_service();

        let mut entry = make_entry(Some("claude-opus-5"), 0, 0, 0, 100_000, None);
        entry.cache_creation_1h_tokens = 100_000;

        let cost = service.calculate_cost(&entry);

        // 100k * $10/1M = $1.00 (above_1hr), not 100k * $6.25/1M = $0.625
        let expected = 100_000.0 * 0.00001;
        assert!(
            (cost - expected).abs() < 1e-9,
            "1h cache writes must use the above_1hr rate: expected {}, got {}",
            expected,
            cost
        );
    }

    #[test]
    fn test_cache_creation_5m_uses_base_rate_when_above_1hr_present() {
        let (service, _temp) = create_test_service();

        let mut entry = make_entry(Some("claude-opus-5"), 0, 0, 0, 150_000, None);
        entry.cache_creation_5m_tokens = 100_000;
        entry.cache_creation_1h_tokens = 50_000;

        let cost = service.calculate_cost(&entry);

        // 5m: 100k * $6.25/1M = $0.625, 1h: 50k * $10/1M = $0.50
        let expected = 100_000.0 * 0.00000625 + 50_000.0 * 0.00001;
        assert!(
            (cost - expected).abs() < 1e-9,
            "Mixed TTL split mispriced: expected {}, got {}",
            expected,
            cost
        );
    }

    #[test]
    fn test_cache_creation_1h_without_above_1hr_rate_uses_base() {
        let (service, _temp) = create_test_service();

        // claude-sonnet-4 carries no above_1hr rate in the test cache
        let mut entry = make_entry(Some("claude-sonnet-4"), 0, 0, 0, 100_000, None);
        entry.cache_creation_1h_tokens = 100_000;

        let cost = service.calculate_cost(&entry);

        let expected = 100_000.0 * 0.00000375;
        assert!(
            (cost - expected).abs() < 1e-9,
            "Without an above_1hr rate the base rate applies: expected {}, got {}",
            expected,
            cost
        );
    }

    #[test]
    fn test_cache_creation_without_ttl_breakdown_uses_base_rate() {
        let (service, _temp) = create_test_service();

        // Historical entry: cache_creation_tokens with no 5m/1h split
        let entry = make_entry(Some("claude-opus-5"), 0, 0, 0, 100_000, None);

        let cost = service.calculate_cost(&entry);

        let expected = 100_000.0 * 0.00000625;
        assert!(
            (cost - expected).abs() < 1e-9,
            "Entries without a TTL split must keep the flat rate: expected {}, got {}",
            expected,
            cost
        );
    }

    #[test]
    fn test_cache_creation_1h_above_200k_tier() {
        let (service, _temp) = create_test_service();

        let mut entry = make_entry(Some("claude-sonnet-4-5"), 0, 0, 0, 300_000, None);
        entry.cache_creation_1h_tokens = 300_000;

        let cost = service.calculate_cost(&entry);

        // Split at the breakpoint: 200k at the 1h rate ($6/1M), 100k above it ($12/1M)
        let expected = 200_000.0 * 0.000006 + 100_000.0 * 0.000012;
        assert!(
            (cost - expected).abs() < 1e-9,
            "1h cache above 200k must use the tiered rate: expected {}, got {}",
            expected,
            cost
        );
    }

    #[test]
    fn test_custom_1h_pricing_overrides_litellm_above_1hr() {
        let temp_dir = TempDir::new().unwrap();
        let cache_path = temp_dir.path().join("pricing.json");
        create_mock_cache(&cache_path);

        let custom = CustomPricingConfig {
            models: Some(HashMap::from([(
                "claude-sonnet-4".to_string(),
                CustomModelPricing {
                    input: Some(3.0),
                    output: Some(15.0),
                    cache_creation: Some(3.75),
                    cache_read: Some(0.30),
                    cache_creation_5m: Some(3.75),
                    cache_creation_1h: Some(9.0),
                    input_above_200k: None,
                    output_above_200k: None,
                    cache_read_above_200k: None,
                    cache_creation_above_200k: None,
                },
            )])),
            global: None,
        };

        let service = PricingService::from_cache_only_with_path(&cache_path)
            .unwrap()
            .with_custom_pricing(Some(custom));

        let mut entry = make_entry(Some("claude-sonnet-4"), 0, 0, 0, 100_000, None);
        entry.cache_creation_1h_tokens = 100_000;

        let cost = service.calculate_cost(&entry);

        let expected = 100_000.0 * (9.0 / 1_000_000.0);
        assert!(
            (cost - expected).abs() < 1e-9,
            "Custom 1h pricing must win over LiteLLM: expected {}, got {}",
            expected,
            cost
        );
    }

    #[test]
    fn test_model_pricing_skips_none_fields_on_serialize() {
        let pricing = ModelPricing {
            input_cost_per_token: Some(0.000005),
            ..Default::default()
        };

        let json = serde_json::to_string(&pricing).unwrap();

        assert_eq!(
            json, r#"{"input_cost_per_token":5e-6}"#,
            "Unset fields must not be serialized: {}",
            json
        );
    }

    #[test]
    fn test_has_any_pricing() {
        assert!(
            !ModelPricing::default().has_any_pricing(),
            "An all-None entry carries no pricing"
        );
        assert!(ModelPricing {
            input_cost_per_token: Some(0.000005),
            ..Default::default()
        }
        .has_any_pricing());
        assert!(
            ModelPricing {
                search_context_cost_per_query: Some(SearchContextCost {
                    search_context_size_medium: Some(0.01),
                    ..Default::default()
                }),
                ..Default::default()
            }
            .has_any_pricing(),
            "Web-search-only entries still price something"
        );
    }

    #[test]
    fn test_fast_speed_applies_multiplier() {
        let (service, _temp) = create_test_service();

        let mut entry = make_entry(Some("claude-opus-5"), 1_000_000, 0, 0, 0, None);
        let standard = service.calculate_cost(&entry);
        entry.fast_speed = true;
        let fast = service.calculate_cost(&entry);

        // claude-opus-5 in the test cache carries provider_specific_entry.fast = 2.0
        assert!(
            (fast - standard * 2.0).abs() < 1e-9,
            "fast mode must bill at 2x: standard {}, fast {}",
            standard,
            fast
        );
    }

    #[test]
    fn test_fast_speed_without_multiplier_is_unchanged() {
        let (service, _temp) = create_test_service();

        // claude-sonnet-4 has no provider_specific_entry
        let mut entry = make_entry(Some("claude-sonnet-4"), 1_000_000, 0, 0, 0, None);
        let standard = service.calculate_cost(&entry);
        entry.fast_speed = true;
        let fast = service.calculate_cost(&entry);

        assert!(
            (fast - standard).abs() < 1e-9,
            "A model without a fast rate must not change price"
        );
    }

    #[test]
    fn test_fast_multiplier_excludes_web_search_cost() {
        let (service, _temp) = create_test_service();

        let mut entry = make_entry(Some("claude-opus-5"), 1_000_000, 0, 0, 0, None);
        entry.web_search_requests = 10;
        entry.fast_speed = true;

        let cost = service.calculate_cost(&entry);

        // tokens double, the $0.01/query search rate does not
        let expected = 1_000_000.0 * 0.000005 * 2.0 + 10.0 * 0.01;
        assert!(
            (cost - expected).abs() < 1e-9,
            "Per-query search cost must stay outside the speed multiplier: expected {}, got {}",
            expected,
            cost
        );
    }

    #[test]
    fn test_custom_pricing_keeps_litellm_fast_multiplier() {
        let (base, temp_dir) = create_test_service();
        drop(base);
        let cache_path = temp_dir.path().join("pricing.json");

        let custom = CustomPricingConfig {
            models: Some(HashMap::from([(
                "claude-opus-5".to_string(),
                CustomModelPricing {
                    input: Some(4.0), // user tweaks only the base input rate
                    output: None,
                    cache_creation: None,
                    cache_read: None,
                    cache_creation_5m: None,
                    cache_creation_1h: None,
                    input_above_200k: None,
                    output_above_200k: None,
                    cache_read_above_200k: None,
                    cache_creation_above_200k: None,
                },
            )])),
            global: None,
        };

        let service = PricingService::from_cache_only_with_path(&cache_path)
            .unwrap()
            .with_custom_pricing(Some(custom));

        let mut entry = make_entry(Some("claude-opus-5"), 1_000_000, 0, 0, 0, None);
        entry.fast_speed = true;

        let cost = service.calculate_cost(&entry);

        // Custom $4/1M input, still doubled by LiteLLM's fast multiplier
        let expected = 1_000_000.0 * 0.000004 * 2.0;
        assert!(
            (cost - expected).abs() < 1e-9,
            "A custom rate must not drop fast-mode billing: expected {}, got {}",
            expected,
            cost
        );
    }
}
