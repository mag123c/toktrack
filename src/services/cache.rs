//! DailySummary caching service for persistent usage statistics
//!
//! Caches daily summaries to preserve historical data even after
//! original JSONL files are deleted.

use crate::services::Aggregator;
use crate::types::{CacheWarning, DailySummary, Result, ToktrackError, UsageEntry};
use chrono::{Local, NaiveDate};
use directories::BaseDirs;
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::PathBuf;

/// Cached daily summary data for a CLI
#[derive(Debug, Serialize, Deserialize)]
pub struct DailySummaryCache {
    /// CLI identifier
    pub cli: String,
    /// Unix timestamp of last update
    pub updated_at: i64,
    /// Cached daily summaries
    pub summaries: Vec<DailySummary>,
}

/// Service for caching and retrieving daily summaries
pub struct DailySummaryCacheService {
    cache_dir: PathBuf,
}

impl DailySummaryCacheService {
    /// Create a new cache service with default cache directory (~/.toktrack/cache)
    pub fn new() -> Result<Self> {
        let base_dirs = BaseDirs::new()
            .ok_or_else(|| ToktrackError::Cache("Cannot determine home directory".into()))?;
        let cache_dir = base_dirs.home_dir().join(".toktrack").join("cache");
        fs::create_dir_all(&cache_dir)?;
        Ok(Self { cache_dir })
    }

    /// Create a cache service with a custom cache directory
    pub fn with_cache_dir(cache_dir: PathBuf) -> Self {
        Self { cache_dir }
    }

    /// Get the cache file path for a CLI
    pub fn cache_path(&self, cli: &str) -> PathBuf {
        self.cache_dir.join(format!("{}_daily.json", cli))
    }

    /// Load cached summaries, compute missing dates, and merge results
    ///
    /// Algorithm:
    /// 1. Load cache if exists, filter out today's data
    /// 2. Determine which dates need computation (missing or today)
    /// 3. Compute summaries for those dates
    /// 4. Merge cached + new, deduplicate by date
    /// 5. Save updated cache
    ///
    /// Returns (summaries, optional_warning) - warning indicates cache issues
    pub fn load_or_compute(
        &self,
        cli: &str,
        entries: &[UsageEntry],
    ) -> Result<(Vec<DailySummary>, Option<CacheWarning>)> {
        let today = Local::now().date_naive();

        // Step 1: Load cache (past dates only)
        let (cached, warning) = self.load_past_summaries(cli, today);
        let cached_dates: HashSet<NaiveDate> = cached.iter().map(|s| s.date).collect();

        // Step 2: Determine dates to compute
        let entry_dates: HashSet<NaiveDate> =
            entries.iter().map(|e| e.timestamp.date_naive()).collect();

        let dates_to_compute: HashSet<NaiveDate> = entry_dates
            .iter()
            .filter(|&date| *date == today || !cached_dates.contains(date))
            .copied()
            .collect();

        // Step 3: Filter entries for dates to compute
        let entries_to_compute: Vec<&UsageEntry> = entries
            .iter()
            .filter(|e| dates_to_compute.contains(&e.timestamp.date_naive()))
            .collect();

        // Step 4: Compute new summaries
        let new_summaries = if entries_to_compute.is_empty() {
            Vec::new()
        } else {
            let owned: Vec<UsageEntry> = entries_to_compute.into_iter().cloned().collect();
            Aggregator::daily(&owned)
        };

        // Step 5: Merge and deduplicate (new takes precedence)
        let new_dates: HashSet<NaiveDate> = new_summaries.iter().map(|s| s.date).collect();
        let mut result: Vec<DailySummary> = cached
            .into_iter()
            .filter(|s| !new_dates.contains(&s.date))
            .collect();
        result.extend(new_summaries);

        // Sort by date ascending
        result.sort_by_key(|s| s.date);

        // Step 6: Save cache
        self.save_cache(cli, &result)?;

        Ok((result, warning))
    }

    /// Clear cache for a CLI
    pub fn clear(&self, cli: &str) -> Result<()> {
        let path = self.cache_path(cli);
        if path.exists() {
            fs::remove_file(&path)?;
        }
        Ok(())
    }

    /// Load cached summaries for past dates (excludes today)
    /// Uses shared (read) file lock to prevent concurrent write corruption
    /// Returns (summaries, optional_warning)
    fn load_past_summaries(
        &self,
        cli: &str,
        today: NaiveDate,
    ) -> (Vec<DailySummary>, Option<CacheWarning>) {
        let path = self.cache_path(cli);
        if !path.exists() {
            return (Vec::new(), None);
        }

        // Open file with shared lock for reading
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

        // Acquire shared (read) lock - allows multiple readers
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
                // Corrupted cache - return warning
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

        // Release lock (automatically released on drop, but explicit is clearer)
        let _ = file.unlock();

        // Filter to past dates only
        (
            cache
                .summaries
                .into_iter()
                .filter(|s| s.date < today)
                .collect(),
            None,
        )
    }

    /// Save summaries to cache using atomic write (temp file + rename)
    /// This prevents data corruption from concurrent reads during write
    fn save_cache(&self, cli: &str, summaries: &[DailySummary]) -> Result<()> {
        fs::create_dir_all(&self.cache_dir)?;

        let cache = DailySummaryCache {
            cli: cli.to_string(),
            updated_at: chrono::Utc::now().timestamp(),
            summaries: summaries.to_vec(),
        };

        let content = serde_json::to_string_pretty(&cache)
            .map_err(|e| ToktrackError::Cache(format!("Serialization failed: {}", e)))?;

        let path = self.cache_path(cli);
        let temp_path = path.with_extension("json.tmp");

        // Step 1: Write to temp file (no lock needed - unique path per write)
        {
            let mut file = File::create(&temp_path)
                .map_err(|e| ToktrackError::Cache(format!("Failed to create temp file: {}", e)))?;
            file.write_all(content.as_bytes())
                .map_err(|e| ToktrackError::Cache(format!("Failed to write temp file: {}", e)))?;
            file.sync_all()
                .map_err(|e| ToktrackError::Cache(format!("Failed to sync temp file: {}", e)))?;
        }

        // Step 2: Lock target file, then atomic rename
        // Note: truncate(false) since we only need the file descriptor for locking,
        // the actual content comes from the atomic rename
        let target = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(false)
            .open(&path)?;

        // Acquire exclusive lock BEFORE modifying target
        target
            .lock_exclusive()
            .map_err(|e| ToktrackError::Cache(format!("Failed to acquire write lock: {}", e)))?;

        // Atomic rename (on same filesystem) - replaces target atomically
        fs::rename(&temp_path, &path)
            .map_err(|e| ToktrackError::Cache(format!("Failed to rename temp file: {}", e)))?;

        // Release lock
        let _ = target.unlock();

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Utc};
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

    // Test 2: Cache hit only recomputes today
    #[test]
    fn test_cache_hit_only_recomputes_today() {
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
            total_cost_usd: 9.99,
            models: HashMap::new(),
        };
        let cache = DailySummaryCache {
            cli: "claude-code".to_string(),
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
            },
        ];

        let (result, warning) = service.load_or_compute("claude-code", &entries).unwrap();

        // Should have 2 summaries, no warning for valid cache
        assert!(warning.is_none());
        assert_eq!(result.len(), 2);

        // Yesterday should use cached value (999), not entry (100)
        let yesterday_result = result.iter().find(|s| s.date == yesterday).unwrap();
        assert_eq!(yesterday_result.total_input_tokens, 999);

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
            total_cost_usd: 9.99,
            models: HashMap::new(),
        };
        let cache = DailySummaryCache {
            cli: "claude-code".to_string(),
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
            total_cost_usd: 0.005,
            models: HashMap::new(),
        };
        let cache = DailySummaryCache {
            cli: "claude-code".to_string(),
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
}
