//! Data-preservation audit: per-source, per-day coverage classification.
//!
//! For each source we compare the dates that still have raw session files on
//! disk (`live`) against the dates kept in toktrack's persistent cache. A day
//! that exists only in the cache (`cache_only`) is cost history the upstream CLI
//! has already deleted — the concrete proof of toktrack's persistence wedge. A
//! day inside the covered range with neither raw nor cached data is `missing`
//! ("no data" — could be an unused day or one lost before toktrack saw it; we do
//! not claim it as a loss).

use crate::parsers::SourceInstance;
use crate::services::DailySummaryCacheService;
use chrono::NaiveDate;
use serde::Serialize;
use std::collections::{BTreeSet, HashSet};

/// Coverage state for a single (source, day).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DayCoverage {
    /// Raw session file for this day is still on disk.
    Live,
    /// Day is in the cache but the raw file is gone — preserved by toktrack.
    CacheOnly,
    /// Day inside the covered range with neither raw nor cached data.
    Missing,
}

/// Coverage of one calendar day.
#[derive(Debug, Clone, Serialize)]
pub struct DayStatus {
    pub date: NaiveDate,
    pub coverage: DayCoverage,
}

/// Per-source audit result over the covered date range.
#[derive(Debug, Clone, Serialize)]
pub struct SourceAudit {
    pub id: String,
    pub label: String,
    pub start: Option<NaiveDate>,
    pub end: Option<NaiveDate>,
    pub total_days: u32,
    pub live_days: u32,
    pub cache_only_days: u32,
    pub missing_days: u32,
    pub days: Vec<DayStatus>,
}

/// Full audit report across all sources.
#[derive(Debug, Clone, Serialize)]
pub struct AuditReport {
    pub generated_for: NaiveDate,
    /// Headline metric: total days preserved in cache after the CLI deleted them.
    pub total_cache_only_days: u32,
    pub sources: Vec<SourceAudit>,
}

/// Classify every day in `[min(live ∪ cache) ..= max(live ∪ cache)]`.
///
/// Pure: given the live and cached date sets it produces the per-day coverage
/// and counts. `live` takes precedence over `cache` (a day with a raw file is
/// still recoverable, so it is reported as `live` even if also cached).
pub fn audit_source(
    id: impl Into<String>,
    label: impl Into<String>,
    live: &HashSet<NaiveDate>,
    cache: &HashSet<NaiveDate>,
) -> SourceAudit {
    let id = id.into();
    let label = label.into();

    // Union of all dates that carry data anywhere; bounds the covered range.
    let all: BTreeSet<NaiveDate> = live.iter().chain(cache.iter()).copied().collect();
    let start = all.iter().next().copied();
    let end = all.iter().next_back().copied();

    let mut days = Vec::new();
    let mut live_days = 0u32;
    let mut cache_only_days = 0u32;
    let mut missing_days = 0u32;

    if let (Some(start), Some(end)) = (start, end) {
        let mut date = start;
        loop {
            let coverage = if live.contains(&date) {
                live_days += 1;
                DayCoverage::Live
            } else if cache.contains(&date) {
                cache_only_days += 1;
                DayCoverage::CacheOnly
            } else {
                missing_days += 1;
                DayCoverage::Missing
            };
            days.push(DayStatus { date, coverage });

            if date == end {
                break;
            }
            // The `date == end` break above guarantees `date < end` here, so
            // `date` is strictly below the max representable NaiveDate.
            date = date.succ_opt().expect("date is strictly before end");
        }
    }

    SourceAudit {
        id,
        label,
        start,
        end,
        total_days: days.len() as u32,
        live_days,
        cache_only_days,
        missing_days,
        days,
    }
}

/// Build the full audit report across every source.
///
/// For each source `live` is the set of dates with raw session files still on
/// disk, and `cache` is the set of dates kept in toktrack's persistent cache.
/// Sources are processed independently; a source whose raw data fails to parse
/// degrades to "no live data" rather than failing the whole report.
pub fn build_report(
    sources: &[SourceInstance],
    cache: &DailySummaryCacheService,
    today: NaiveDate,
) -> AuditReport {
    let sources: Vec<SourceAudit> = sources
        .iter()
        .map(|source| {
            let live = collect_live_dates(source);
            let cached = cache.cached_dates(&source.id);
            audit_source(source.id.as_str(), source.label.as_str(), &live, &cached)
        })
        .collect();

    let total_cache_only_days = sources.iter().map(|s| s.cache_only_days).sum();

    AuditReport {
        generated_for: today,
        total_cache_only_days,
        sources,
    }
}

/// Collect the local dates that currently have raw session files for `source`.
///
/// Uses a full `parse_all` so multi-day session files are classified correctly
/// (peeking a single line per file would misattribute files that span
/// midnight). On parse failure we warn and return an empty set — the audit
/// never hard-fails on one bad source.
fn collect_live_dates(source: &SourceInstance) -> HashSet<NaiveDate> {
    match source.parser.parse_all() {
        Ok(entries) => entries.iter().map(|e| e.local_date()).collect(),
        Err(e) => {
            eprintln!(
                "[toktrack] Warning: failed to read raw data for '{}': {}",
                source.id, e
            );
            HashSet::new()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }

    fn set(dates: &[NaiveDate]) -> HashSet<NaiveDate> {
        dates.iter().copied().collect()
    }

    #[test]
    fn test_live_takes_precedence_over_cache() {
        // A day present in BOTH live and cache is reported as live (recoverable).
        let day = d(2026, 3, 1);
        let audit = audit_source("claude-code", "claude-code", &set(&[day]), &set(&[day]));

        assert_eq!(audit.days.len(), 1);
        assert_eq!(audit.days[0].coverage, DayCoverage::Live);
        assert_eq!(audit.live_days, 1);
        assert_eq!(audit.cache_only_days, 0);
    }

    #[test]
    fn test_cache_only_when_raw_gone() {
        // In cache but not on disk → preserved-by-toktrack.
        let day = d(2026, 3, 1);
        let audit = audit_source("codex", "codex", &HashSet::new(), &set(&[day]));

        assert_eq!(audit.days[0].coverage, DayCoverage::CacheOnly);
        assert_eq!(audit.cache_only_days, 1);
        assert_eq!(audit.live_days, 0);
    }

    #[test]
    fn test_missing_for_gap_days_inside_range() {
        // Range is 03-01..=03-03; only the endpoints have data, the middle is a gap.
        let live = set(&[d(2026, 3, 1)]);
        let cache = set(&[d(2026, 3, 3)]);
        let audit = audit_source("gemini", "gemini", &live, &cache);

        assert_eq!(audit.start, Some(d(2026, 3, 1)));
        assert_eq!(audit.end, Some(d(2026, 3, 3)));
        assert_eq!(audit.total_days, 3);
        assert_eq!(audit.days[1].date, d(2026, 3, 2));
        assert_eq!(audit.days[1].coverage, DayCoverage::Missing);
        assert_eq!(audit.live_days, 1);
        assert_eq!(audit.cache_only_days, 1);
        assert_eq!(audit.missing_days, 1);
    }

    #[test]
    fn test_empty_source_yields_none_range() {
        let audit = audit_source("pi-agent", "pi-agent", &HashSet::new(), &HashSet::new());

        assert_eq!(audit.start, None);
        assert_eq!(audit.end, None);
        assert_eq!(audit.total_days, 0);
        assert!(audit.days.is_empty());
    }

    #[test]
    fn test_single_day_source() {
        let day = d(2026, 6, 8);
        let audit = audit_source("opencode", "opencode", &set(&[day]), &HashSet::new());

        assert_eq!(audit.start, Some(day));
        assert_eq!(audit.end, Some(day));
        assert_eq!(audit.total_days, 1);
        assert_eq!(audit.live_days, 1);
    }

    #[test]
    fn test_counts_sum_to_total_days() {
        // 03-01 live, 03-02 missing, 03-03 cache_only, 03-04 missing, 03-05 live.
        let live = set(&[d(2026, 3, 1), d(2026, 3, 5)]);
        let cache = set(&[d(2026, 3, 3)]);
        let audit = audit_source("claude-code", "claude-code", &live, &cache);

        assert_eq!(audit.total_days, 5);
        assert_eq!(
            audit.live_days + audit.cache_only_days + audit.missing_days,
            audit.total_days
        );
        assert_eq!(audit.live_days, 2);
        assert_eq!(audit.cache_only_days, 1);
        assert_eq!(audit.missing_days, 2);
    }

    #[test]
    fn test_coverage_serializes_to_snake_case() {
        let day = d(2026, 3, 1);
        let audit = audit_source("codex", "codex", &HashSet::new(), &set(&[day]));
        let json = serde_json::to_string(&audit.days[0]).unwrap();
        assert!(json.contains("\"cache_only\""), "got: {json}");
    }

    #[test]
    fn test_build_report_counts_cached_dates_as_cache_only_when_no_raw() {
        use crate::parsers::ClaudeCodeParser;
        use crate::services::cache::DailySummaryCache;
        use crate::types::DailySummary;
        use std::collections::HashMap;
        use std::path::PathBuf;
        use tempfile::TempDir;

        let temp = TempDir::new().unwrap();
        let cache = DailySummaryCacheService::with_cache_dir(temp.path().to_path_buf());

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
        let cache_file = DailySummaryCache {
            cli: "auditsrc".to_string(),
            version: 0, // irrelevant for cached_dates
            updated_at: 0,
            summaries: vec![summary_on(d(2024, 1, 10)), summary_on(d(2024, 1, 12))],
        };
        std::fs::write(
            cache.cache_path("auditsrc"),
            serde_json::to_string(&cache_file).unwrap(),
        )
        .unwrap();

        // Parser points at a nonexistent dir → no live dates, so every cached
        // date must be classified cache_only.
        let source = SourceInstance::new(
            "auditsrc",
            "auditsrc",
            "claude-code",
            Box::new(ClaudeCodeParser::with_data_dir(PathBuf::from(
                "tests/fixtures/nonexistent",
            ))),
        );

        let report = build_report(std::slice::from_ref(&source), &cache, d(2024, 6, 1));

        assert_eq!(report.sources.len(), 1);
        let audited = &report.sources[0];
        assert_eq!(audited.live_days, 0);
        assert_eq!(audited.cache_only_days, 2);
        assert_eq!(report.total_cache_only_days, 2);
        assert_eq!(report.generated_for, d(2024, 6, 1));
    }
}
