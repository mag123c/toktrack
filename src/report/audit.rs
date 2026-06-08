//! Human-readable rendering of the data-preservation audit.
//!
//! Leads each source with the headline-worthy figure — days preserved in cache
//! after the upstream CLI deleted them — while labelling gap days honestly as
//! "no data (unused or lost)" so the report never overstates a loss.

use crate::services::audit::{AuditReport, SourceAudit};
use std::fmt::Write as _;

/// Render the audit report as plain text suitable for a terminal.
pub fn render(report: &AuditReport) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "Data preservation audit ({})", report.generated_for);
    let _ = writeln!(out);

    for source in &report.sources {
        render_source(&mut out, source);
    }

    if report.total_cache_only_days > 0 {
        let _ = writeln!(
            out,
            "  \u{25ba} {} days of cost history preserved that your CLIs already deleted.",
            report.total_cache_only_days
        );
    } else {
        let _ = writeln!(
            out,
            "  \u{25ba} No cache-only history yet — all tracked data is still live on disk."
        );
    }

    out
}

fn render_source(out: &mut String, source: &SourceAudit) {
    match (source.start, source.end) {
        (Some(start), Some(end)) => {
            let _ = writeln!(out, "  {}   {} \u{2192} {}", source.label, start, end);
            let _ = writeln!(
                out,
                "    live (raw on disk):       {:>4} days",
                source.live_days
            );
            let _ = writeln!(
                out,
                "    preserved (CLI deleted):  {:>4} days",
                source.cache_only_days
            );
            let _ = writeln!(
                out,
                "    no data (unused or lost): {:>4} days",
                source.missing_days
            );
            let _ = writeln!(out);
        }
        _ => {
            let _ = writeln!(out, "  {}   (no data)", source.label);
            let _ = writeln!(out);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::audit::SourceAudit;
    use chrono::NaiveDate;

    fn date(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    fn report_with(sources: Vec<SourceAudit>, total_cache_only: u32) -> AuditReport {
        AuditReport {
            generated_for: date(2026, 6, 8),
            total_cache_only_days: total_cache_only,
            sources,
        }
    }

    fn source(
        label: &str,
        start: Option<NaiveDate>,
        live: u32,
        cache_only: u32,
        missing: u32,
    ) -> SourceAudit {
        SourceAudit {
            id: label.to_string(),
            label: label.to_string(),
            start,
            end: start,
            total_days: live + cache_only + missing,
            live_days: live,
            cache_only_days: cache_only,
            missing_days: missing,
            days: Vec::new(),
        }
    }

    #[test]
    fn test_render_includes_headline_count() {
        let r = report_with(
            vec![source("claude-code", Some(date(2026, 3, 1)), 72, 28, 0)],
            28,
        );
        let text = render(&r);
        assert!(
            text.contains("28 days of cost history preserved"),
            "got:\n{text}"
        );
    }

    #[test]
    fn test_render_labels_missing_honestly() {
        let r = report_with(vec![source("codex", Some(date(2026, 3, 1)), 1, 1, 1)], 1);
        let text = render(&r);
        assert!(text.contains("no data (unused or lost)"), "got:\n{text}");
    }

    #[test]
    fn test_render_empty_source_shows_no_data() {
        let r = report_with(vec![source("pi-agent", None, 0, 0, 0)], 0);
        let text = render(&r);
        assert!(text.contains("pi-agent   (no data)"), "got:\n{text}");
    }

    #[test]
    fn test_render_zero_preserved_uses_live_headline() {
        let r = report_with(vec![source("gemini", Some(date(2026, 3, 1)), 5, 0, 0)], 0);
        let text = render(&r);
        assert!(text.contains("still live on disk"), "got:\n{text}");
    }
}
