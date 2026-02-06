//! DailySummary caching service for persistent usage statistics
//!
//! Caches daily summaries to preserve historical data even after
//! original JSONL files are deleted.

use crate::services::{normalize_model_name, Aggregator};
use crate::types::{CacheWarning, DailySummary, ModelUsage, Result, ToktrackError, UsageEntry};
use chrono::{Local, NaiveDate};
use directories::BaseDirs;
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::PathBuf;

/// Normalize model name keys in a HashMap, merging duplicates.
fn normalize_model_keys(models: HashMap<String, ModelUsage>) -> HashMap<String, ModelUsage> {
    let mut normalized: HashMap<String, ModelUsage> = HashMap::new();
    for (name, usage) in models {
        let key = normalize_model_name(&name);
        normalized
            .entry(key)
            .and_modify(|existing| {
                existing.input_tokens = existing.input_tokens.saturating_add(usage.input_tokens);
                existing.output_tokens = existing.output_tokens.saturating_add(usage.output_tokens);
                existing.cache_read_tokens = existing
                    .cache_read_tokens
                    .saturating_add(usage.cache_read_tokens);
                existing.cache_creation_tokens = existing
                    .cache_creation_tokens
                    .saturating_add(usage.cache_creation_tokens);
                existing.thinking_tokens = existing
                    .thinking_tokens
                    .saturating_add(usage.thinking_tokens);
                existing.cost_usd += usage.cost_usd;
                existing.count = existing.count.saturating_add(usage.count);
            })
            .or_insert(usage);
    }
    normalized
}

/// Bump when aggregation logic changes (e.g., timezone fix).
/// Mismatched version → full cache invalidation.
const CACHE_VERSION: u32 = 5;

#[derive(Debug, Serialize, Deserialize)]
pub struct DailySummaryCache {
    pub cli: String,
    #[serde(default)]
    pub version: u32,
    pub updated_at: i64,
    pub summaries: Vec<DailySummary>,
}

pub struct DailySummaryCacheService {
    cache_dir: PathBuf,
}

impl DailySummaryCacheService {
    pub fn new() -> Result<Self> {
        let base_dirs = BaseDirs::new()
            .ok_or_else(|| ToktrackError::Cache("Cannot determine home directory".into()))?;
        let cache_dir = base_dirs.home_dir().join(".toktrack").join("cache");
        fs::create_dir_all(&cache_dir)?;
        Ok(Self { cache_dir })
    }

    #[allow(dead_code)]
    pub fn with_cache_dir(cache_dir: PathBuf) -> Self {
        Self { cache_dir }
    }

    pub fn cache_path(&self, cli: &str) -> PathBuf {
        self.cache_dir.join(format!("{}_daily.json", cli))
    }

    /// Check if cached version matches current CACHE_VERSION.
    /// Returns false if cache doesn't exist or version mismatches.
    pub fn is_version_current(&self, cli: &str) -> bool {
        let path = self.cache_path(cli);
        if !path.exists() {
            return false;
        }
        let content = match fs::read_to_string(&path) {
            Ok(c) => c,
            Err(_) => return false,
        };
        let cache: DailySummaryCache = match serde_json::from_str(&content) {
            Ok(c) => c,
            Err(_) => return false,
        };
        cache.version == CACHE_VERSION
    }

    /// Load cached summaries, compute missing dates, merge and deduplicate.
    /// Today is always recomputed. Returns (summaries, optional_warning).
    pub fn load_or_compute(
        &self,
        cli: &str,
        entries: &[UsageEntry],
    ) -> Result<(Vec<DailySummary>, Option<CacheWarning>)> {
        let today = Local::now().date_naive();

        let (cached, warning) = self.load_past_summaries(cli, today);

        let entry_dates: HashSet<NaiveDate> = entries.iter().map(|e| e.local_date()).collect();

        // Recompute: today (always), uncached dates, and cached dates with new entries.
        // Since we iterate entry_dates, any date with entries is recomputed.
        let dates_to_compute: HashSet<NaiveDate> = entry_dates.clone();

        let entries_to_compute: Vec<&UsageEntry> = entries
            .iter()
            .filter(|e| dates_to_compute.contains(&e.local_date()))
            .collect();

        let new_summaries = if entries_to_compute.is_empty() {
            Vec::new()
        } else {
            let owned: Vec<UsageEntry> = entries_to_compute.into_iter().cloned().collect();
            Aggregator::daily(&owned)
        };

        let new_dates: HashSet<NaiveDate> = new_summaries.iter().map(|s| s.date).collect();
        let mut result: Vec<DailySummary> = cached
            .into_iter()
            .filter(|s| !new_dates.contains(&s.date))
            .collect();
        result.extend(new_summaries);
        result.sort_by_key(|s| s.date);

        self.save_cache(cli, &result)?;

        Ok((result, warning))
    }

    #[allow(dead_code)]
    pub fn clear(&self, cli: &str) -> Result<()> {
        let path = self.cache_path(cli);
        if path.exists() {
            fs::remove_file(&path)?;
        }
        Ok(())
    }

    /// Load cached summaries for past dates (excludes today).
    /// Uses shared file lock for concurrent read safety.
    fn load_past_summaries(
        &self,
        cli: &str,
        today: NaiveDate,
    ) -> (Vec<DailySummary>, Option<CacheWarning>) {
        let path = self.cache_path(cli);
        if !path.exists() {
            return (Vec::new(), None);
        }

        let file = match File::open(&path) {
            Ok(f) => f,
            Err(e) => {
                return (
                    Vec::new(),
                    Some(CacheWarning::LoadFailed(format!(
                        "Failed to open cache: {}",
                        e
                    ))),
                );
            }
        };

        if let Err(e) = file.lock_shared() {
            return (
                Vec::new(),
                Some(CacheWarning::LoadFailed(format!(
                    "Failed to acquire read lock: {}",
                    e
                ))),
            );
        }

        let mut content = String::new();
        let mut reader = std::io::BufReader::new(&file);
        if let Err(e) = reader.read_to_string(&mut content) {
            let _ = file.unlock();
            return (
                Vec::new(),
                Some(CacheWarning::LoadFailed(format!(
                    "Failed to read cache: {}",
                    e
                ))),
            );
        }

        let cache: DailySummaryCache = match serde_json::from_str(&content) {
            Ok(c) => c,
            Err(e) => {
                let _ = file.unlock();
                return (
                    Vec::new(),
                    Some(CacheWarning::Corrupted(format!(
                        "Corrupted cache file: {}",
                        e
                    ))),
                );
            }
        };

        let warning = if cache.version != CACHE_VERSION {
            Some(CacheWarning::VersionMismatch(format!(
                "Cache version {} != {}, recomputing available dates",
                cache.version, CACHE_VERSION
            )))
        } else {
            None
        };

        let _ = file.unlock();

        // Migrate model names: normalize keys in the models HashMap
        let summaries: Vec<DailySummary> = cache
            .summaries
            .into_iter()
            .filter(|s| s.date < today)
            .map(|mut s| {
                s.models = normalize_model_keys(s.models);
                s
            })
            .collect();

        (summaries, warning)
    }

    /// Save using atomic write (temp file + rename) with exclusive lock.
    fn save_cache(&self, cli: &str, summaries: &[DailySummary]) -> Result<()> {
        fs::create_dir_all(&self.cache_dir)?;

        let cache = DailySummaryCache {
            cli: cli.to_string(),
            version: CACHE_VERSION,
            updated_at: chrono::Utc::now().timestamp(),
            summaries: summaries.to_vec(),
        };

        let content = serde_json::to_string_pretty(&cache)
            .map_err(|e| ToktrackError::Cache(format!("Serialization failed: {}", e)))?;

        let path = self.cache_path(cli);
        let temp_path = path.with_extension("json.tmp");

        {
            let mut file = File::create(&temp_path)
                .map_err(|e| ToktrackError::Cache(format!("Failed to create temp file: {}", e)))?;
            file.write_all(content.as_bytes())
                .map_err(|e| ToktrackError::Cache(format!("Failed to write temp file: {}", e)))?;
            file.sync_all()
                .map_err(|e| ToktrackError::Cache(format!("Failed to sync temp file: {}", e)))?;
        }

        let target = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(false)
            .open(&path)?;

        target
            .lock_exclusive()
            .map_err(|e| ToktrackError::Cache(format!("Failed to acquire write lock: {}", e)))?;

        fs::rename(&temp_path, &path)
            .map_err(|e| ToktrackError::Cache(format!("Failed to rename temp file: {}", e)))?;

        let _ = target.unlock();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Datelike, TimeZone, Utc};
    use std::collections::HashMap;
    use tempfile::TempDir;

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

    fn create_test_service() -> (DailySummaryCacheService, TempDir) {
        let temp_dir = TempDir::new().unwrap();
        let service = DailySummaryCacheService::with_cache_dir(temp_dir.path().to_path_buf());
        (service, temp_dir)
    }

    // Test 1: No cache computes all entries
    #[test]
    fn test_no_cache_computes_all_entries() {
        let (service, _temp) = create_test_service();
        let entries = vec![
            make_entry(2024, 1, 10, Some("claude"), 100, 50, Some(0.01)),
            make_entry(2024, 1, 11, Some("claude"), 200, 100, Some(0.02)),
        ];

        let (result, warning) = service.load_or_compute("claude-code", &entries).unwrap();

        assert!(warning.is_none());
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].date.to_string(), "2024-01-10");
        assert_eq!(result[1].date.to_string(), "2024-01-11");
        assert_eq!(result[0].total_input_tokens, 100);
        assert_eq!(result[1].total_input_tokens, 200);
    }

    // Test 2: Cache hit recomputes dates with new entries
    #[test]
    fn test_cache_recomputes_dates_with_entries() {
        let (service, _temp) = create_test_service();
        let today = Local::now().date_naive();
        let yesterday = today - chrono::Duration::days(1);

        // Pre-populate cache with yesterday's data
        let cached_summary = DailySummary {
            date: yesterday,
            total_input_tokens: 999, // Different from entries
            total_output_tokens: 999,
            total_cache_read_tokens: 0,
            total_cache_creation_tokens: 0,
            total_thinking_tokens: 0,
            total_cost_usd: 9.99,
            models: HashMap::new(),
        };
        let cache = DailySummaryCache {
            cli: "claude-code".to_string(),
            version: CACHE_VERSION,
            updated_at: chrono::Utc::now().timestamp(),
            summaries: vec![cached_summary],
        };
        let cache_path = service.cache_path("claude-code");
        fs::create_dir_all(cache_path.parent().unwrap()).unwrap();
        fs::write(&cache_path, serde_json::to_string(&cache).unwrap()).unwrap();

        // Entries for yesterday and today
        let entries = vec![
            UsageEntry {
                timestamp: yesterday.and_hms_opt(12, 0, 0).unwrap().and_utc(),
                model: Some("claude".to_string()),
                input_tokens: 100,
                output_tokens: 50,
                cache_read_tokens: 0,
                cache_creation_tokens: 0,
                thinking_tokens: 0,
                cost_usd: Some(0.01),
                message_id: None,
                request_id: None,
                source: None,
                provider: None,
            },
            UsageEntry {
                timestamp: today.and_hms_opt(12, 0, 0).unwrap().and_utc(),
                model: Some("claude".to_string()),
                input_tokens: 200,
                output_tokens: 100,
                cache_read_tokens: 0,
                cache_creation_tokens: 0,
                thinking_tokens: 0,
                cost_usd: Some(0.02),
                message_id: None,
                request_id: None,
                source: None,
                provider: None,
            },
        ];

        let (result, warning) = service.load_or_compute("claude-code", &entries).unwrap();

        // Should have 2 summaries, no warning for valid cache
        assert!(warning.is_none());
        assert_eq!(result.len(), 2);

        // Yesterday should be recomputed from entries (100), not cached (999)
        let yesterday_result = result.iter().find(|s| s.date == yesterday).unwrap();
        assert_eq!(yesterday_result.total_input_tokens, 100);

        // Today should be recomputed (200)
        let today_result = result.iter().find(|s| s.date == today).unwrap();
        assert_eq!(today_result.total_input_tokens, 200);
    }

    // Test 3: Corrupted cache falls back to full recomputation with warning
    #[test]
    fn test_corrupted_cache_falls_back() {
        let (service, _temp) = create_test_service();
        let cache_path = service.cache_path("claude-code");
        fs::create_dir_all(cache_path.parent().unwrap()).unwrap();
        fs::write(&cache_path, "not valid json {{{").unwrap();

        let entries = vec![make_entry(2024, 1, 10, Some("claude"), 100, 50, Some(0.01))];

        let (result, warning) = service.load_or_compute("claude-code", &entries).unwrap();

        // Should return warning for corrupted cache
        assert!(matches!(warning, Some(CacheWarning::Corrupted(_))));
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].total_input_tokens, 100);
    }

    // Test 4: Empty entries returns empty result
    #[test]
    fn test_empty_entries_returns_empty() {
        let (service, _temp) = create_test_service();
        let entries: Vec<UsageEntry> = vec![];

        let (result, _warning) = service.load_or_compute("claude-code", &entries).unwrap();

        assert!(result.is_empty());
    }

    // Test 5: Merge deduplicates by date (new takes precedence)
    #[test]
    fn test_merge_deduplicates_by_date() {
        let (service, _temp) = create_test_service();
        let today = Local::now().date_naive();

        // Pre-populate cache with today's old data
        let cached_summary = DailySummary {
            date: today,
            total_input_tokens: 999,
            total_output_tokens: 999,
            total_cache_read_tokens: 0,
            total_cache_creation_tokens: 0,
            total_thinking_tokens: 0,
            total_cost_usd: 9.99,
            models: HashMap::new(),
        };
        let cache = DailySummaryCache {
            cli: "claude-code".to_string(),
            version: CACHE_VERSION,
            updated_at: chrono::Utc::now().timestamp(),
            summaries: vec![cached_summary],
        };
        let cache_path = service.cache_path("claude-code");
        fs::create_dir_all(cache_path.parent().unwrap()).unwrap();
        fs::write(&cache_path, serde_json::to_string(&cache).unwrap()).unwrap();

        // New entry for today
        let entries = vec![UsageEntry {
            timestamp: today.and_hms_opt(12, 0, 0).unwrap().and_utc(),
            model: Some("claude".to_string()),
            input_tokens: 100,
            output_tokens: 50,
            cache_read_tokens: 0,
            cache_creation_tokens: 0,
            thinking_tokens: 0,
            cost_usd: Some(0.01),
            message_id: None,
            request_id: None,
            source: None,
            provider: None,
        }];

        let (result, _warning) = service.load_or_compute("claude-code", &entries).unwrap();

        // Should only have one entry for today with the new value
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].date, today);
        assert_eq!(result[0].total_input_tokens, 100); // New value, not 999
    }

    // Test 6: Results are sorted ascending by date
    #[test]
    fn test_results_sorted_ascending() {
        let (service, _temp) = create_test_service();
        let entries = vec![
            make_entry(2024, 1, 20, Some("claude"), 300, 150, Some(0.03)),
            make_entry(2024, 1, 10, Some("claude"), 100, 50, Some(0.01)),
            make_entry(2024, 1, 15, Some("claude"), 200, 100, Some(0.02)),
        ];

        let (result, _warning) = service.load_or_compute("claude-code", &entries).unwrap();

        assert_eq!(result.len(), 3);
        assert_eq!(result[0].date.to_string(), "2024-01-10");
        assert_eq!(result[1].date.to_string(), "2024-01-15");
        assert_eq!(result[2].date.to_string(), "2024-01-20");
    }

    // Test 7: Today is always recalculated even if in cache
    #[test]
    fn test_today_always_recalculated() {
        let (service, _temp) = create_test_service();
        let today = Local::now().date_naive();

        // Pre-populate cache with today
        let cached_summary = DailySummary {
            date: today,
            total_input_tokens: 50, // Old value
            total_output_tokens: 25,
            total_cache_read_tokens: 0,
            total_cache_creation_tokens: 0,
            total_thinking_tokens: 0,
            total_cost_usd: 0.005,
            models: HashMap::new(),
        };
        let cache = DailySummaryCache {
            cli: "claude-code".to_string(),
            version: CACHE_VERSION,
            updated_at: chrono::Utc::now().timestamp(),
            summaries: vec![cached_summary],
        };
        let cache_path = service.cache_path("claude-code");
        fs::create_dir_all(cache_path.parent().unwrap()).unwrap();
        fs::write(&cache_path, serde_json::to_string(&cache).unwrap()).unwrap();

        // New entry for today with different values
        let entries = vec![UsageEntry {
            timestamp: today.and_hms_opt(15, 0, 0).unwrap().and_utc(),
            model: Some("claude".to_string()),
            input_tokens: 200,
            output_tokens: 100,
            cache_read_tokens: 0,
            cache_creation_tokens: 0,
            thinking_tokens: 0,
            cost_usd: Some(0.02),
            message_id: None,
            request_id: None,
            source: None,
            provider: None,
        }];

        let (result, _warning) = service.load_or_compute("claude-code", &entries).unwrap();

        assert_eq!(result.len(), 1);
        assert_eq!(result[0].total_input_tokens, 200); // New value, not 50
    }

    // Test 8: Cache path format is correct
    #[test]
    fn test_cache_path_format() {
        let (service, temp) = create_test_service();

        let path = service.cache_path("claude-code");
        assert_eq!(path, temp.path().join("claude-code_daily.json"));

        let path2 = service.cache_path("cursor");
        assert_eq!(path2, temp.path().join("cursor_daily.json"));
    }

    // Test 9: Clear removes cache file
    #[test]
    fn test_clear_removes_cache_file() {
        let (service, _temp) = create_test_service();
        let cache_path = service.cache_path("claude-code");

        // Create cache file
        fs::create_dir_all(cache_path.parent().unwrap()).unwrap();
        fs::write(&cache_path, "{}").unwrap();
        assert!(cache_path.exists());

        // Clear it
        service.clear("claude-code").unwrap();

        assert!(!cache_path.exists());
    }

    // Test 10: CLI isolation - different CLIs have separate caches
    #[test]
    fn test_cli_isolation() {
        let (service, _temp) = create_test_service();

        // Store data for claude-code
        let entries1 = vec![make_entry(2024, 1, 10, Some("claude"), 100, 50, Some(0.01))];
        service.load_or_compute("claude-code", &entries1).unwrap();

        // Store data for cursor
        let entries2 = vec![make_entry(2024, 1, 10, Some("gpt-4"), 500, 250, Some(0.05))];
        service.load_or_compute("cursor", &entries2).unwrap();

        // Verify separate cache files exist
        let claude_cache = service.cache_path("claude-code");
        let cursor_cache = service.cache_path("cursor");
        assert!(claude_cache.exists());
        assert!(cursor_cache.exists());
        assert_ne!(claude_cache, cursor_cache);

        // Verify data is isolated
        let claude_content: DailySummaryCache =
            serde_json::from_str(&fs::read_to_string(&claude_cache).unwrap()).unwrap();
        let cursor_content: DailySummaryCache =
            serde_json::from_str(&fs::read_to_string(&cursor_cache).unwrap()).unwrap();

        assert_eq!(claude_content.cli, "claude-code");
        assert_eq!(cursor_content.cli, "cursor");
        assert_eq!(claude_content.summaries[0].total_input_tokens, 100);
        assert_eq!(cursor_content.summaries[0].total_input_tokens, 500);
    }

    // Test 11: Cache migrates model names (normalizes keys)
    #[test]
    fn test_cache_migrates_model_names() {
        let (service, _temp) = create_test_service();
        let yesterday = Local::now().date_naive() - chrono::Duration::days(1);

        // Create cache with non-normalized model names (with date suffixes)
        let mut models = HashMap::new();
        models.insert(
            "claude-opus-4-5-20251101".to_string(),
            crate::types::ModelUsage {
                input_tokens: 100,
                output_tokens: 50,
                cache_read_tokens: 0,
                cache_creation_tokens: 0,
                thinking_tokens: 0,
                cost_usd: 0.10,
                count: 1,
            },
        );
        models.insert(
            "claude-opus-4.5".to_string(), // Dot version, same model
            crate::types::ModelUsage {
                input_tokens: 200,
                output_tokens: 100,
                cache_read_tokens: 0,
                cache_creation_tokens: 0,
                thinking_tokens: 0,
                cost_usd: 0.20,
                count: 2,
            },
        );

        let cached_summary = DailySummary {
            date: yesterday,
            total_input_tokens: 300,
            total_output_tokens: 150,
            total_cache_read_tokens: 0,
            total_cache_creation_tokens: 0,
            total_thinking_tokens: 0,
            total_cost_usd: 0.30,
            models,
        };
        let cache = DailySummaryCache {
            cli: "claude-code".to_string(),
            version: CACHE_VERSION,
            updated_at: chrono::Utc::now().timestamp(),
            summaries: vec![cached_summary],
        };
        let cache_path = service.cache_path("claude-code");
        fs::create_dir_all(cache_path.parent().unwrap()).unwrap();
        fs::write(&cache_path, serde_json::to_string(&cache).unwrap()).unwrap();

        // Load and verify normalization + merging
        let entries: Vec<UsageEntry> = vec![];
        let (result, _warning) = service.load_or_compute("claude-code", &entries).unwrap();

        assert_eq!(result.len(), 1);
        let summary = &result[0];

        // Should have only one model key after normalization (merged)
        assert_eq!(summary.models.len(), 1);
        assert!(summary.models.contains_key("claude-opus-4-5"));

        // Values should be merged
        let model = summary.models.get("claude-opus-4-5").unwrap();
        assert_eq!(model.input_tokens, 300); // 100 + 200
        assert_eq!(model.output_tokens, 150); // 50 + 100
        assert!((model.cost_usd - 0.30).abs() < f64::EPSILON); // 0.10 + 0.20
        assert_eq!(model.count, 3); // 1 + 2
    }

    // Test 12: Old cache without version (deserialized as 0) triggers VersionMismatch
    #[test]
    fn test_old_cache_version_mismatch() {
        let (service, _temp) = create_test_service();
        let yesterday = Local::now().date_naive() - chrono::Duration::days(1);

        // Write cache JSON without "version" field (simulates pre-versioning cache)
        let json = serde_json::json!({
            "cli": "claude-code",
            "updated_at": chrono::Utc::now().timestamp(),
            "summaries": [{
                "date": yesterday.to_string(),
                "total_input_tokens": 999,
                "total_output_tokens": 999,
                "total_cache_read_tokens": 0,
                "total_cache_creation_tokens": 0,
                "total_thinking_tokens": 0,
                "total_cost_usd": 9.99,
                "models": {}
            }]
        });
        let cache_path = service.cache_path("claude-code");
        fs::create_dir_all(cache_path.parent().unwrap()).unwrap();
        fs::write(&cache_path, json.to_string()).unwrap();

        let entries = vec![make_entry(
            yesterday.year(),
            yesterday.month(),
            yesterday.day(),
            Some("claude"),
            100,
            50,
            Some(0.01),
        )];

        let (result, warning) = service.load_or_compute("claude-code", &entries).unwrap();

        // Should return VersionMismatch warning
        assert!(matches!(warning, Some(CacheWarning::VersionMismatch(_))));
        // Old cached value (999) should be discarded; recomputed from entries
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].total_input_tokens, 100);
    }

    // Test 13: Matching version loads cache normally
    #[test]
    fn test_matching_version_loads_normally() {
        let (service, _temp) = create_test_service();
        let yesterday = Local::now().date_naive() - chrono::Duration::days(1);

        let cached_summary = DailySummary {
            date: yesterday,
            total_input_tokens: 500,
            total_output_tokens: 250,
            total_cache_read_tokens: 0,
            total_cache_creation_tokens: 0,
            total_thinking_tokens: 0,
            total_cost_usd: 0.50,
            models: HashMap::new(),
        };
        let cache = DailySummaryCache {
            cli: "claude-code".to_string(),
            version: CACHE_VERSION,
            updated_at: chrono::Utc::now().timestamp(),
            summaries: vec![cached_summary],
        };
        let cache_path = service.cache_path("claude-code");
        fs::create_dir_all(cache_path.parent().unwrap()).unwrap();
        fs::write(&cache_path, serde_json::to_string(&cache).unwrap()).unwrap();

        // No entries — should rely entirely on cache
        let entries: Vec<UsageEntry> = vec![];
        let (result, warning) = service.load_or_compute("claude-code", &entries).unwrap();

        assert!(warning.is_none());
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].total_input_tokens, 500);
    }

    // Test 14: Version mismatch preserves cached dates without entries
    #[test]
    fn test_version_mismatch_preserves_old_data_without_entries() {
        let (service, _temp) = create_test_service();
        let old_date = Local::now().date_naive() - chrono::Duration::days(30);
        let yesterday = Local::now().date_naive() - chrono::Duration::days(1);

        // Old cache with two dates: old_date (no entries) + yesterday (has entries)
        let json = serde_json::json!({
            "cli": "claude-code",
            "version": 0,
            "updated_at": chrono::Utc::now().timestamp(),
            "summaries": [
                {
                    "date": old_date.to_string(),
                    "total_input_tokens": 500,
                    "total_output_tokens": 250,
                    "total_cache_read_tokens": 0,
                    "total_cache_creation_tokens": 0,
                    "total_thinking_tokens": 0,
                    "total_cost_usd": 5.00,
                    "models": {}
                },
                {
                    "date": yesterday.to_string(),
                    "total_input_tokens": 888,
                    "total_output_tokens": 444,
                    "total_cache_read_tokens": 0,
                    "total_cache_creation_tokens": 0,
                    "total_thinking_tokens": 0,
                    "total_cost_usd": 8.88,
                    "models": {}
                }
            ]
        });
        let cache_path = service.cache_path("claude-code");
        fs::create_dir_all(cache_path.parent().unwrap()).unwrap();
        fs::write(&cache_path, json.to_string()).unwrap();

        // Only provide entries for yesterday (old_date JSONL is gone)
        let entries = vec![make_entry(
            yesterday.year(),
            yesterday.month(),
            yesterday.day(),
            Some("claude"),
            200,
            100,
            Some(0.02),
        )];

        let (result, warning) = service.load_or_compute("claude-code", &entries).unwrap();

        assert!(matches!(warning, Some(CacheWarning::VersionMismatch(_))));
        assert_eq!(result.len(), 2);

        // old_date: preserved from cache (no entries to recompute)
        let old = result.iter().find(|s| s.date == old_date).unwrap();
        assert_eq!(old.total_input_tokens, 500);

        // yesterday: recomputed from entries
        let recent = result.iter().find(|s| s.date == yesterday).unwrap();
        assert_eq!(recent.total_input_tokens, 200);

        // Saved cache should now have CACHE_VERSION
        let saved: DailySummaryCache =
            serde_json::from_str(&fs::read_to_string(&cache_path).unwrap()).unwrap();
        assert_eq!(saved.version, CACHE_VERSION);
    }
}
