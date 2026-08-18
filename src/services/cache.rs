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
use std::io::Write;
use std::path::PathBuf;

/// Normalize a composite "model::provider" key by normalizing only the model
/// portion. Plain (non-composite) keys are normalized as before.
/// (Issue #134: provider strings must not be subject to model-name normalization)
fn normalize_composite_key(key: &str) -> String {
    if let Some((model, provider)) = key.split_once("::") {
        format!("{}::{}", normalize_model_name(model), provider)
    } else {
        normalize_model_name(key)
    }
}

/// Normalize model name keys in a HashMap, merging duplicates.
fn normalize_model_keys(models: HashMap<String, ModelUsage>) -> HashMap<String, ModelUsage> {
    let mut normalized: HashMap<String, ModelUsage> = HashMap::new();
    for (name, usage) in models {
        let key = normalize_composite_key(&name);
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
                existing.reasoning_tokens = existing
                    .reasoning_tokens
                    .saturating_add(usage.reasoning_tokens);
                existing.cache_creation_5m_tokens = existing
                    .cache_creation_5m_tokens
                    .saturating_add(usage.cache_creation_5m_tokens);
                existing.cache_creation_1h_tokens = existing
                    .cache_creation_1h_tokens
                    .saturating_add(usage.cache_creation_1h_tokens);
                existing.web_search_requests = existing
                    .web_search_requests
                    .saturating_add(usage.web_search_requests);
                existing.cost_usd += usage.cost_usd;
                existing.count = existing.count.saturating_add(usage.count);
            })
            .or_insert(usage);
    }
    normalized
}

/// Bump when aggregation logic changes (e.g., timezone fix).
/// Mismatched version → past summaries are kept (history is preserved) but a
/// warning is surfaced, and every date whose raw files still exist is recomputed
/// so the new shape is populated. v14 added per-project breakdown
/// (`DailySummary.projects`). v15 resolves gemini-default / missing-model
/// records by timestamp so they no longer appear as "unknown". v16 prices 1h
/// ephemeral cache writes at LiteLLM's `_above_1hr` rate.
const CACHE_VERSION: u32 = 16;

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

    fn lock_path(&self, cli: &str) -> PathBuf {
        self.cache_dir.join(format!("{}_daily.json.lock", cli))
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
        let lock = self.lock_path(cli);
        if lock.exists() {
            fs::remove_file(&lock)?;
        }
        Ok(())
    }

    /// Return the latest (max) date in the cached summaries, or None.
    pub fn latest_cached_date(&self, cli: &str) -> Option<NaiveDate> {
        let path = self.cache_path(cli);
        let content = fs::read_to_string(&path).ok()?;
        let cache: DailySummaryCache = serde_json::from_str(&content).ok()?;
        cache.summaries.iter().map(|s| s.date).max()
    }

    /// Return the set of dates present in the cached summaries.
    /// Empty when the cache file is absent or unreadable (no panic) — the audit
    /// treats an unreadable cache as "nothing preserved" rather than failing.
    pub fn cached_dates(&self, cli: &str) -> HashSet<NaiveDate> {
        let path = self.cache_path(cli);
        let Ok(content) = fs::read_to_string(&path) else {
            return HashSet::new();
        };
        match serde_json::from_str::<DailySummaryCache>(&content) {
            Ok(cache) => cache.summaries.iter().map(|s| s.date).collect(),
            Err(_) => HashSet::new(),
        }
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

        // Lock on separate .lock file for cross-process synchronization.
        // If lock file can't be opened, proceed without lock (backward compat).
        let lock_path = self.lock_path(cli);
        let lock_file = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(false)
            .open(&lock_path);
        if let Ok(ref lf) = lock_file {
            let _ = lf.lock_shared();
        }

        let content = match fs::read_to_string(&path) {
            Ok(c) => c,
            Err(e) => {
                if let Ok(ref lf) = lock_file {
                    let _ = lf.unlock();
                }
                return (
                    Vec::new(),
                    Some(CacheWarning::LoadFailed(format!(
                        "Failed to read cache: {}",
                        e
                    ))),
                );
            }
        };

        let cache: DailySummaryCache = match serde_json::from_str(&content) {
            Ok(c) => c,
            Err(e) => {
                if let Ok(ref lf) = lock_file {
                    let _ = lf.unlock();
                }
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

        if let Ok(ref lf) = lock_file {
            let _ = lf.unlock();
        }

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

        let lock_path = self.lock_path(cli);
        let lock_file = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(false)
            .open(&lock_path)
            .map_err(|e| ToktrackError::Cache(format!("Failed to open lock file: {}", e)))?;
        lock_file
            .lock_exclusive()
            .map_err(|e| ToktrackError::Cache(format!("Failed to acquire write lock: {}", e)))?;

        {
            let mut file = File::create(&temp_path)
                .map_err(|e| ToktrackError::Cache(format!("Failed to create temp file: {}", e)))?;
            file.write_all(content.as_bytes())
                .map_err(|e| ToktrackError::Cache(format!("Failed to write temp file: {}", e)))?;
            file.sync_all()
                .map_err(|e| ToktrackError::Cache(format!("Failed to sync temp file: {}", e)))?;
        }

        fs::rename(&temp_path, &path)
            .map_err(|e| ToktrackError::Cache(format!("Failed to rename temp file: {}", e)))?;

        let _ = lock_file.unlock();
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
            fast_speed: false,
            timestamp: Utc.with_ymd_and_hms(year, month, day, 12, 0, 0).unwrap(),
            model: model.map(String::from),
            input_tokens: input,
            output_tokens: output,
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
            provider: None,
            project: None,
        }
    }

    fn create_test_service() -> (DailySummaryCacheService, TempDir) {
        let temp_dir = TempDir::new().unwrap();
        let service = DailySummaryCacheService::with_cache_dir(temp_dir.path().to_path_buf());
        (service, temp_dir)
    }

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

    #[test]
    fn test_cache_recomputes_dates_with_entries() {
        let (service, _temp) = create_test_service();
        let today = Local::now().date_naive();
        let yesterday = today - chrono::Duration::days(1);

        let cached_summary = DailySummary {
            date: yesterday,
            total_input_tokens: 999,
            total_output_tokens: 999,
            total_cache_read_tokens: 0,
            total_cache_creation_tokens: 0,
            total_reasoning_tokens: 0,
            total_cache_creation_5m_tokens: 0,
            total_cache_creation_1h_tokens: 0,
            total_web_search_requests: 0,
            total_cost_usd: 9.99,
            models: HashMap::new(),
            projects: HashMap::new(),
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

        let entries = vec![
            UsageEntry {
                fast_speed: false,
                timestamp: yesterday.and_hms_opt(12, 0, 0).unwrap().and_utc(),
                model: Some("claude".to_string()),
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
                cost_usd: Some(0.01),
                message_id: None,
                request_id: None,
                source: None,
                provider: None,
                project: None,
            },
            UsageEntry {
                fast_speed: false,
                timestamp: today.and_hms_opt(12, 0, 0).unwrap().and_utc(),
                model: Some("claude".to_string()),
                input_tokens: 200,
                output_tokens: 100,
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
            },
        ];

        let (result, warning) = service.load_or_compute("claude-code", &entries).unwrap();

        assert!(warning.is_none());
        assert_eq!(result.len(), 2);

        let yesterday_result = result.iter().find(|s| s.date == yesterday).unwrap();
        assert_eq!(yesterday_result.total_input_tokens, 100);

        let today_result = result.iter().find(|s| s.date == today).unwrap();
        assert_eq!(today_result.total_input_tokens, 200);
    }

    #[test]
    fn test_corrupted_cache_falls_back() {
        let (service, _temp) = create_test_service();
        let cache_path = service.cache_path("claude-code");
        fs::create_dir_all(cache_path.parent().unwrap()).unwrap();
        fs::write(&cache_path, "not valid json {{{").unwrap();

        let entries = vec![make_entry(2024, 1, 10, Some("claude"), 100, 50, Some(0.01))];

        let (result, warning) = service.load_or_compute("claude-code", &entries).unwrap();

        assert!(matches!(warning, Some(CacheWarning::Corrupted(_))));
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].total_input_tokens, 100);
    }

    #[test]
    fn test_empty_entries_returns_empty() {
        let (service, _temp) = create_test_service();
        let entries: Vec<UsageEntry> = vec![];

        let (result, _warning) = service.load_or_compute("claude-code", &entries).unwrap();

        assert!(result.is_empty());
    }

    #[test]
    fn test_merge_deduplicates_by_date() {
        let (service, _temp) = create_test_service();
        let today = Local::now().date_naive();

        let cached_summary = DailySummary {
            date: today,
            total_input_tokens: 999,
            total_output_tokens: 999,
            total_cache_read_tokens: 0,
            total_cache_creation_tokens: 0,
            total_reasoning_tokens: 0,
            total_cache_creation_5m_tokens: 0,
            total_cache_creation_1h_tokens: 0,
            total_web_search_requests: 0,
            total_cost_usd: 9.99,
            models: HashMap::new(),
            projects: HashMap::new(),
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

        let entries = vec![UsageEntry {
            fast_speed: false,
            timestamp: today.and_hms_opt(12, 0, 0).unwrap().and_utc(),
            model: Some("claude".to_string()),
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
            cost_usd: Some(0.01),
            message_id: None,
            request_id: None,
            source: None,
            provider: None,
            project: None,
        }];

        let (result, _warning) = service.load_or_compute("claude-code", &entries).unwrap();

        assert_eq!(result.len(), 1);
        assert_eq!(result[0].date, today);
        assert_eq!(result[0].total_input_tokens, 100);
    }

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

    #[test]
    fn test_today_always_recalculated() {
        let (service, _temp) = create_test_service();
        let today = Local::now().date_naive();

        let cached_summary = DailySummary {
            date: today,
            total_input_tokens: 50,
            total_output_tokens: 25,
            total_cache_read_tokens: 0,
            total_cache_creation_tokens: 0,
            total_reasoning_tokens: 0,
            total_cache_creation_5m_tokens: 0,
            total_cache_creation_1h_tokens: 0,
            total_web_search_requests: 0,
            total_cost_usd: 0.005,
            models: HashMap::new(),
            projects: HashMap::new(),
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

        let entries = vec![UsageEntry {
            fast_speed: false,
            timestamp: today.and_hms_opt(15, 0, 0).unwrap().and_utc(),
            model: Some("claude".to_string()),
            input_tokens: 200,
            output_tokens: 100,
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
        }];

        let (result, _warning) = service.load_or_compute("claude-code", &entries).unwrap();

        assert_eq!(result.len(), 1);
        assert_eq!(result[0].total_input_tokens, 200);
    }

    #[test]
    fn test_cache_path_format() {
        let (service, temp) = create_test_service();

        let path = service.cache_path("claude-code");
        assert_eq!(path, temp.path().join("claude-code_daily.json"));

        let path2 = service.cache_path("cursor");
        assert_eq!(path2, temp.path().join("cursor_daily.json"));
    }

    #[test]
    fn test_clear_removes_cache_file() {
        let (service, _temp) = create_test_service();
        let cache_path = service.cache_path("claude-code");

        fs::create_dir_all(cache_path.parent().unwrap()).unwrap();
        fs::write(&cache_path, "{}").unwrap();
        assert!(cache_path.exists());

        service.clear("claude-code").unwrap();

        assert!(!cache_path.exists());
    }

    #[test]
    fn test_cli_isolation() {
        let (service, _temp) = create_test_service();

        let entries1 = vec![make_entry(2024, 1, 10, Some("claude"), 100, 50, Some(0.01))];
        service.load_or_compute("claude-code", &entries1).unwrap();

        let entries2 = vec![make_entry(2024, 1, 10, Some("gpt-4"), 500, 250, Some(0.05))];
        service.load_or_compute("cursor", &entries2).unwrap();

        let claude_cache = service.cache_path("claude-code");
        let cursor_cache = service.cache_path("cursor");
        assert!(claude_cache.exists());
        assert!(cursor_cache.exists());
        assert_ne!(claude_cache, cursor_cache);

        let claude_content: DailySummaryCache =
            serde_json::from_str(&fs::read_to_string(&claude_cache).unwrap()).unwrap();
        let cursor_content: DailySummaryCache =
            serde_json::from_str(&fs::read_to_string(&cursor_cache).unwrap()).unwrap();

        assert_eq!(claude_content.cli, "claude-code");
        assert_eq!(cursor_content.cli, "cursor");
        assert_eq!(claude_content.summaries[0].total_input_tokens, 100);
        assert_eq!(cursor_content.summaries[0].total_input_tokens, 500);
    }

    #[test]
    fn test_cache_migrates_model_names() {
        let (service, _temp) = create_test_service();
        let yesterday = Local::now().date_naive() - chrono::Duration::days(1);

        let mut models = HashMap::new();
        models.insert(
            "claude-opus-4-5-20251101".to_string(),
            crate::types::ModelUsage {
                input_tokens: 100,
                output_tokens: 50,
                cache_read_tokens: 0,
                cache_creation_tokens: 0,
                reasoning_tokens: 0,
                cache_creation_5m_tokens: 0,
                cache_creation_1h_tokens: 0,
                web_search_requests: 0,
                cost_usd: 0.10,
                count: 1,
            },
        );
        models.insert(
            "claude-opus-4.5".to_string(),
            crate::types::ModelUsage {
                input_tokens: 200,
                output_tokens: 100,
                cache_read_tokens: 0,
                cache_creation_tokens: 0,
                reasoning_tokens: 0,
                cache_creation_5m_tokens: 0,
                cache_creation_1h_tokens: 0,
                web_search_requests: 0,
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
            total_reasoning_tokens: 0,
            total_cache_creation_5m_tokens: 0,
            total_cache_creation_1h_tokens: 0,
            total_web_search_requests: 0,
            total_cost_usd: 0.30,
            models,
            projects: HashMap::new(),
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

        let entries: Vec<UsageEntry> = vec![];
        let (result, _warning) = service.load_or_compute("claude-code", &entries).unwrap();

        assert_eq!(result.len(), 1);
        let summary = &result[0];

        assert_eq!(summary.models.len(), 1);
        assert!(summary.models.contains_key("claude-opus-4-5"));

        let model = summary.models.get("claude-opus-4-5").unwrap();
        assert_eq!(model.input_tokens, 300);
        assert_eq!(model.output_tokens, 150);
        assert!((model.cost_usd - 0.30).abs() < f64::EPSILON);
        assert_eq!(model.count, 3);
    }

    #[test]
    fn test_old_cache_version_mismatch() {
        let (service, _temp) = create_test_service();
        let yesterday = Local::now().date_naive() - chrono::Duration::days(1);

        let json = serde_json::json!({
            "cli": "claude-code",
            "updated_at": chrono::Utc::now().timestamp(),
            "summaries": [{
                "date": yesterday.to_string(),
                "total_input_tokens": 999,
                "total_output_tokens": 999,
                "total_cache_read_tokens": 0,
                "total_cache_creation_tokens": 0,
                "total_reasoning_tokens": 0,
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

        assert!(matches!(warning, Some(CacheWarning::VersionMismatch(_))));
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].total_input_tokens, 100);
    }

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
            total_reasoning_tokens: 0,
            total_cache_creation_5m_tokens: 0,
            total_cache_creation_1h_tokens: 0,
            total_web_search_requests: 0,
            total_cost_usd: 0.50,
            models: HashMap::new(),
            projects: HashMap::new(),
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

        let entries: Vec<UsageEntry> = vec![];
        let (result, warning) = service.load_or_compute("claude-code", &entries).unwrap();

        assert!(warning.is_none());
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].total_input_tokens, 500);
    }

    #[test]
    fn test_version_mismatch_preserves_old_data_without_entries() {
        let (service, _temp) = create_test_service();
        let old_date = Local::now().date_naive() - chrono::Duration::days(30);
        let yesterday = Local::now().date_naive() - chrono::Duration::days(1);

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
                    "total_reasoning_tokens": 0,
                    "total_cost_usd": 5.00,
                    "models": {}
                },
                {
                    "date": yesterday.to_string(),
                    "total_input_tokens": 888,
                    "total_output_tokens": 444,
                    "total_cache_read_tokens": 0,
                    "total_cache_creation_tokens": 0,
                    "total_reasoning_tokens": 0,
                    "total_cost_usd": 8.88,
                    "models": {}
                }
            ]
        });
        let cache_path = service.cache_path("claude-code");
        fs::create_dir_all(cache_path.parent().unwrap()).unwrap();
        fs::write(&cache_path, json.to_string()).unwrap();

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

        let old = result.iter().find(|s| s.date == old_date).unwrap();
        assert_eq!(old.total_input_tokens, 500);

        let recent = result.iter().find(|s| s.date == yesterday).unwrap();
        assert_eq!(recent.total_input_tokens, 200);

        let saved: DailySummaryCache =
            serde_json::from_str(&fs::read_to_string(&cache_path).unwrap()).unwrap();
        assert_eq!(saved.version, CACHE_VERSION);
    }

    #[test]
    fn test_latest_cached_date_no_cache() {
        let (service, _temp) = create_test_service();
        assert_eq!(service.latest_cached_date("claude-code"), None);
    }

    #[test]
    fn test_latest_cached_date_returns_max() {
        let (service, _temp) = create_test_service();
        let cache = DailySummaryCache {
            cli: "claude-code".to_string(),
            version: CACHE_VERSION,
            updated_at: chrono::Utc::now().timestamp(),
            summaries: vec![
                DailySummary {
                    date: NaiveDate::from_ymd_opt(2026, 2, 25).unwrap(),
                    total_input_tokens: 100,
                    total_output_tokens: 50,
                    total_cache_read_tokens: 0,
                    total_cache_creation_tokens: 0,
                    total_reasoning_tokens: 0,
                    total_cache_creation_5m_tokens: 0,
                    total_cache_creation_1h_tokens: 0,
                    total_web_search_requests: 0,
                    total_cost_usd: 0.01,
                    models: HashMap::new(),
                    projects: HashMap::new(),
                },
                DailySummary {
                    date: NaiveDate::from_ymd_opt(2026, 2, 27).unwrap(),
                    total_input_tokens: 200,
                    total_output_tokens: 100,
                    total_cache_read_tokens: 0,
                    total_cache_creation_tokens: 0,
                    total_reasoning_tokens: 0,
                    total_cache_creation_5m_tokens: 0,
                    total_cache_creation_1h_tokens: 0,
                    total_web_search_requests: 0,
                    total_cost_usd: 0.02,
                    models: HashMap::new(),
                    projects: HashMap::new(),
                },
                DailySummary {
                    date: NaiveDate::from_ymd_opt(2026, 2, 26).unwrap(),
                    total_input_tokens: 150,
                    total_output_tokens: 75,
                    total_cache_read_tokens: 0,
                    total_cache_creation_tokens: 0,
                    total_reasoning_tokens: 0,
                    total_cache_creation_5m_tokens: 0,
                    total_cache_creation_1h_tokens: 0,
                    total_web_search_requests: 0,
                    total_cost_usd: 0.015,
                    models: HashMap::new(),
                    projects: HashMap::new(),
                },
            ],
        };
        let cache_path = service.cache_path("claude-code");
        fs::create_dir_all(cache_path.parent().unwrap()).unwrap();
        fs::write(&cache_path, serde_json::to_string(&cache).unwrap()).unwrap();

        assert_eq!(
            service.latest_cached_date("claude-code"),
            Some(NaiveDate::from_ymd_opt(2026, 2, 27).unwrap())
        );
    }

    #[test]
    fn test_cached_dates_empty_when_no_file() {
        let (service, _temp) = create_test_service();
        assert!(service.cached_dates("claude-code").is_empty());
    }

    #[test]
    fn test_cached_dates_collects_all_summary_dates() {
        let (service, _temp) = create_test_service();
        let summary_on = |date: NaiveDate| DailySummary {
            date,
            total_input_tokens: 0,
            total_output_tokens: 0,
            total_cache_read_tokens: 0,
            total_cache_creation_tokens: 0,
            total_reasoning_tokens: 0,
            total_cache_creation_5m_tokens: 0,
            total_cache_creation_1h_tokens: 0,
            total_web_search_requests: 0,
            total_cost_usd: 0.0,
            models: HashMap::new(),
            projects: HashMap::new(),
        };
        let cache = DailySummaryCache {
            cli: "claude-code".to_string(),
            version: CACHE_VERSION,
            updated_at: chrono::Utc::now().timestamp(),
            summaries: vec![
                summary_on(NaiveDate::from_ymd_opt(2026, 3, 1).unwrap()),
                summary_on(NaiveDate::from_ymd_opt(2026, 3, 3).unwrap()),
            ],
        };
        let cache_path = service.cache_path("claude-code");
        fs::create_dir_all(cache_path.parent().unwrap()).unwrap();
        fs::write(&cache_path, serde_json::to_string(&cache).unwrap()).unwrap();

        let dates = service.cached_dates("claude-code");
        assert_eq!(dates.len(), 2);
        assert!(dates.contains(&NaiveDate::from_ymd_opt(2026, 3, 1).unwrap()));
        assert!(dates.contains(&NaiveDate::from_ymd_opt(2026, 3, 3).unwrap()));
    }

    #[test]
    fn test_cached_dates_empty_on_corrupted_file() {
        let (service, _temp) = create_test_service();
        let cache_path = service.cache_path("claude-code");
        fs::create_dir_all(cache_path.parent().unwrap()).unwrap();
        fs::write(&cache_path, "not valid json {{{").unwrap();
        assert!(service.cached_dates("claude-code").is_empty());
    }

    #[test]
    fn test_latest_cached_date_empty_summaries() {
        let (service, _temp) = create_test_service();
        let cache = DailySummaryCache {
            cli: "claude-code".to_string(),
            version: CACHE_VERSION,
            updated_at: chrono::Utc::now().timestamp(),
            summaries: vec![],
        };
        let cache_path = service.cache_path("claude-code");
        fs::create_dir_all(cache_path.parent().unwrap()).unwrap();
        fs::write(&cache_path, serde_json::to_string(&cache).unwrap()).unwrap();

        assert_eq!(service.latest_cached_date("claude-code"), None);
    }

    #[test]
    fn test_normalize_composite_key_preserves_provider() {
        assert_eq!(
            normalize_composite_key("claude-opus-4.5::anthropic"),
            "claude-opus-4-5::anthropic"
        );
        assert_eq!(
            normalize_composite_key("claude-opus-4-5-20251101::anthropic"),
            "claude-opus-4-5::anthropic"
        );
    }

    #[test]
    fn test_normalize_composite_key_provider_with_date_like_suffix() {
        // A provider name ending in an 8-digit `20…` suffix must not be truncated
        // by normalize_model_name's date-suffix stripping logic.
        assert_eq!(
            normalize_composite_key("gpt-4::custom-20250101"),
            "gpt-4::custom-20250101"
        );
    }

    #[test]
    fn test_normalize_composite_key_plain_key_unchanged_behavior() {
        assert_eq!(
            normalize_composite_key("claude-opus-4.5"),
            "claude-opus-4-5"
        );
        assert_eq!(normalize_composite_key("gpt-4"), "gpt-4");
    }

    #[test]
    fn test_cache_roundtrip_preserves_composite_keys() {
        // Composite keys written to disk must round-trip through load_or_compute
        // unchanged.
        let (service, _temp) = create_test_service();
        let yesterday = Local::now().date_naive() - chrono::Duration::days(1);

        let mut models = HashMap::new();
        models.insert(
            "gpt-5-4::github-copilot".to_string(),
            ModelUsage {
                input_tokens: 100,
                output_tokens: 50,
                cost_usd: 0.0,
                count: 1,
                ..Default::default()
            },
        );
        models.insert(
            "gpt-5-4::openai".to_string(),
            ModelUsage {
                input_tokens: 200,
                output_tokens: 100,
                cost_usd: 0.05,
                count: 1,
                ..Default::default()
            },
        );

        let cached = DailySummary {
            date: yesterday,
            total_input_tokens: 300,
            total_output_tokens: 150,
            total_cache_read_tokens: 0,
            total_cache_creation_tokens: 0,
            total_reasoning_tokens: 0,
            total_cache_creation_5m_tokens: 0,
            total_cache_creation_1h_tokens: 0,
            total_web_search_requests: 0,
            total_cost_usd: 0.05,
            models,
            projects: HashMap::new(),
        };
        let cache = DailySummaryCache {
            cli: "codex".to_string(),
            version: CACHE_VERSION,
            updated_at: chrono::Utc::now().timestamp(),
            summaries: vec![cached],
        };
        let cache_path = service.cache_path("codex");
        fs::create_dir_all(cache_path.parent().unwrap()).unwrap();
        fs::write(&cache_path, serde_json::to_string(&cache).unwrap()).unwrap();

        let entries: Vec<UsageEntry> = vec![];
        let (result, warning) = service.load_or_compute("codex", &entries).unwrap();

        assert!(warning.is_none());
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].models.len(), 2);
        assert!(result[0].models.contains_key("gpt-5-4::github-copilot"));
        assert!(result[0].models.contains_key("gpt-5-4::openai"));
    }

    #[test]
    fn test_cache_roundtrip_normalizes_composite_with_dotted_model() {
        // When a composite key carries a dotted model, normalization touches
        // only the model part — provider remains intact.
        let (service, _temp) = create_test_service();
        let yesterday = Local::now().date_naive() - chrono::Duration::days(1);

        let mut models = HashMap::new();
        // Pre-normalization key (model contains '.') simulating a stale v0 cache,
        // forcing the version-mismatch normalization path.
        models.insert(
            "claude-opus-4.5::anthropic".to_string(),
            ModelUsage {
                input_tokens: 100,
                ..Default::default()
            },
        );

        let cached = DailySummary {
            date: yesterday,
            total_input_tokens: 100,
            total_output_tokens: 0,
            total_cache_read_tokens: 0,
            total_cache_creation_tokens: 0,
            total_reasoning_tokens: 0,
            total_cache_creation_5m_tokens: 0,
            total_cache_creation_1h_tokens: 0,
            total_web_search_requests: 0,
            total_cost_usd: 0.0,
            models,
            projects: HashMap::new(),
        };
        let cache = serde_json::json!({
            "cli": "codex",
            "version": 0,
            "updated_at": chrono::Utc::now().timestamp(),
            "summaries": [cached],
        });
        let cache_path = service.cache_path("codex");
        fs::create_dir_all(cache_path.parent().unwrap()).unwrap();
        fs::write(&cache_path, cache.to_string()).unwrap();

        let entries: Vec<UsageEntry> = vec![];
        let (result, warning) = service.load_or_compute("codex", &entries).unwrap();

        assert!(warning.is_some());
        assert_eq!(result.len(), 1);
        assert!(result[0].models.contains_key("claude-opus-4-5::anthropic"));
    }
}
