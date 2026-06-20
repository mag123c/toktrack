//! Audit view widget — per-source data-preservation coverage.
//!
//! Renders one stacked bar per source (live / preserved / no-data) and a
//! headline count of the days toktrack kept after the upstream CLI deleted
//! them. While the audit is still computing (a background full parse) the view
//! shows a "computing" message rather than blocking startup.

use ratatui::{
    buffer::Buffer,
    layout::{Alignment, Constraint, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Paragraph, Widget},
};

use super::tabs::{Tab, TabBar};
use crate::services::audit::{AuditReport, SourceAudit};
use crate::tui::theme::Theme;

/// Maximum content width (consistent with the other views).
const MAX_CONTENT_WIDTH: u16 = 170;
/// Width of the per-source coverage bar, in cells.
const BAR_WIDTH: usize = 48;

/// Split a bar of `width` cells proportionally across the three coverage
/// categories. `live` and `cache_only` are each bumped to at least one cell so
/// a single preserved day stays visible; if the bar is too narrow to fit both,
/// `cache_only` (the headline metric) is kept and `live` may fall to zero.
/// `missing` absorbs the remainder.
fn segment_widths(live: u32, cache_only: u32, missing: u32, width: usize) -> (usize, usize, usize) {
    let total = (live + cache_only + missing) as usize;
    if total == 0 || width == 0 {
        return (0, 0, 0);
    }

    let mut l = live as usize * width / total;
    let mut c = cache_only as usize * width / total;
    if live > 0 {
        l = l.max(1);
    }
    if cache_only > 0 {
        c = c.max(1);
    }
    // Bumping to at least one cell can overshoot the bar; trim the larger of the
    // two filled segments until they fit, keeping at least one cell each.
    while l + c > width {
        if l >= c && l > 1 {
            l -= 1;
        } else if c > 1 {
            c -= 1;
        } else if l > 0 {
            l -= 1;
        } else {
            c -= 1;
        }
    }
    let m = width - l - c;
    (l, c, m)
}

/// Audit view widget.
pub struct AuditView<'a> {
    report: Option<&'a AuditReport>,
    error: Option<&'a str>,
    selected_tab: Tab,
    theme: Theme,
}

impl<'a> AuditView<'a> {
    pub fn new(report: Option<&'a AuditReport>, error: Option<&'a str>, theme: Theme) -> Self {
        Self {
            report,
            error,
            selected_tab: Tab::Audit,
            theme,
        }
    }
}

impl Widget for AuditView<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let content_width = area.width.min(MAX_CONTENT_WIDTH);
        let x_offset = (area.width.saturating_sub(content_width)) / 2;
        let centered = Rect {
            x: area.x + x_offset,
            y: area.y,
            width: content_width,
            height: area.height,
        };

        let chunks = Layout::vertical([
            Constraint::Length(1), // tabs
            Constraint::Length(1), // separator
            Constraint::Length(1), // title
            Constraint::Length(1), // blank
            Constraint::Min(0),    // content
            Constraint::Length(1), // separator
            Constraint::Length(1), // keybindings
        ])
        .split(centered);

        TabBar::new(self.selected_tab, self.theme).render(chunks[0], buf);
        self.render_separator(chunks[1], buf);
        self.render_title(chunks[2], buf);
        self.render_content(chunks[4], buf);
        self.render_separator(chunks[5], buf);
        self.render_keybindings(chunks[6], buf);
    }
}

impl AuditView<'_> {
    fn render_separator(&self, area: Rect, buf: &mut Buffer) {
        let line = "─".repeat(area.width as usize);
        buf.set_string(
            area.x,
            area.y,
            &line,
            Style::default().fg(self.theme.muted()),
        );
    }

    fn render_title(&self, area: Rect, buf: &mut Buffer) {
        Paragraph::new(Line::from(Span::styled(
            "Data Preservation Audit",
            Style::default()
                .fg(self.theme.text())
                .add_modifier(Modifier::BOLD),
        )))
        .alignment(Alignment::Center)
        .render(area, buf);
    }

    fn render_content(&self, area: Rect, buf: &mut Buffer) {
        match (self.error, self.report) {
            (Some(err), _) => self.render_message(
                area,
                buf,
                &format!("Audit failed: {err}"),
                self.theme.error(),
            ),
            (None, None) => self.render_message(
                area,
                buf,
                "Computing data-preservation audit… (parsing raw session files)",
                self.theme.muted(),
            ),
            (None, Some(report)) => self.render_report(area, buf, report),
        }
    }

    fn render_message(
        &self,
        area: Rect,
        buf: &mut Buffer,
        msg: &str,
        color: ratatui::style::Color,
    ) {
        Paragraph::new(Line::from(Span::styled(
            msg.to_string(),
            Style::default().fg(color),
        )))
        .alignment(Alignment::Center)
        .render(area, buf);
    }

    fn render_report(&self, area: Rect, buf: &mut Buffer, report: &AuditReport) {
        let mut lines: Vec<Line> = Vec::new();
        lines.push(self.legend_line());
        lines.push(Line::from(""));
        for source in &report.sources {
            self.push_source_lines(&mut lines, source);
        }
        lines.push(Line::from(""));
        lines.push(self.headline_line(report));

        Paragraph::new(lines).render(area, buf);
    }

    fn legend_line(&self) -> Line<'static> {
        Line::from(vec![
            Span::styled("█ live   ", Style::default().fg(self.theme.bar())),
            Span::styled(
                "█ preserved (CLI deleted)   ",
                Style::default().fg(self.theme.cost()),
            ),
            Span::styled(
                "░ no data (unused or lost)",
                Style::default().fg(self.theme.muted()),
            ),
        ])
    }

    fn push_source_lines(&self, lines: &mut Vec<Line<'static>>, source: &SourceAudit) {
        match (source.start, source.end) {
            (Some(start), Some(end)) => {
                lines.push(Line::from(vec![
                    Span::styled(
                        format!("{:<14}", source.label),
                        Style::default().fg(self.theme.text()),
                    ),
                    Span::styled(
                        format!("{start} → {end}   "),
                        Style::default().fg(self.theme.muted()),
                    ),
                    Span::styled(
                        format!("live {}", source.live_days),
                        Style::default().fg(self.theme.bar()),
                    ),
                    Span::styled(" · ", Style::default().fg(self.theme.muted())),
                    Span::styled(
                        format!("preserved {}", source.cache_only_days),
                        Style::default().fg(self.theme.cost()),
                    ),
                    Span::styled(" · ", Style::default().fg(self.theme.muted())),
                    Span::styled(
                        format!("missing {}", source.missing_days),
                        Style::default().fg(self.theme.muted()),
                    ),
                ]));

                let (l, c, m) = segment_widths(
                    source.live_days,
                    source.cache_only_days,
                    source.missing_days,
                    BAR_WIDTH,
                );
                lines.push(Line::from(vec![
                    Span::raw("  "),
                    Span::styled("█".repeat(l), Style::default().fg(self.theme.bar())),
                    Span::styled("█".repeat(c), Style::default().fg(self.theme.cost())),
                    Span::styled("░".repeat(m), Style::default().fg(self.theme.muted())),
                ]));
                lines.push(Line::from(""));
            }
            _ => {
                lines.push(Line::from(vec![
                    Span::styled(
                        format!("{:<14}", source.label),
                        Style::default().fg(self.theme.text()),
                    ),
                    Span::styled("(no data)", Style::default().fg(self.theme.muted())),
                ]));
                lines.push(Line::from(""));
            }
        }
    }

    fn headline_line(&self, report: &AuditReport) -> Line<'static> {
        if report.total_cache_only_days > 0 {
            Line::from(Span::styled(
                format!(
                    "► {} days of cost history preserved that your CLIs already deleted.",
                    report.total_cache_only_days
                ),
                Style::default()
                    .fg(self.theme.cost())
                    .add_modifier(Modifier::BOLD),
            ))
        } else {
            Line::from(Span::styled(
                "► No cache-only history yet — all tracked data is still live on disk.",
                Style::default().fg(self.theme.muted()),
            ))
        }
    }

    fn render_keybindings(&self, area: Rect, buf: &mut Buffer) {
        Paragraph::new(Line::from(vec![
            Span::styled("Ctrl+C", Style::default().fg(self.theme.accent())),
            Span::styled(": Quit", Style::default().fg(self.theme.muted())),
            Span::raw("  "),
            Span::styled("Tab", Style::default().fg(self.theme.accent())),
            Span::styled(": Switch view", Style::default().fg(self.theme.muted())),
            Span::raw("  "),
            Span::styled("?", Style::default().fg(self.theme.accent())),
            Span::styled(": Help", Style::default().fg(self.theme.muted())),
        ]))
        .alignment(Alignment::Center)
        .render(area, buf);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::audit::{AuditReport, SourceAudit};
    use chrono::NaiveDate;

    fn date(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
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
    fn test_segment_widths_split_proportionally() {
        // 50/50 live vs missing over 48 cells.
        let (l, c, m) = segment_widths(10, 0, 10, 48);
        assert_eq!(c, 0);
        assert_eq!(l + c + m, 48);
        assert_eq!(l, 24);
    }

    #[test]
    fn test_segment_widths_keeps_single_preserved_day_visible() {
        // 1 preserved day out of 1000 must still light at least one cell.
        let (_l, c, _m) = segment_widths(999, 1, 0, 48);
        assert!(c >= 1, "preserved segment vanished");
    }

    #[test]
    fn test_segment_widths_never_exceed_width() {
        let (l, c, m) = segment_widths(1, 1, 0, 2);
        assert_eq!(l + c + m, 2);
    }

    #[test]
    fn test_segment_widths_empty_is_zero() {
        assert_eq!(segment_widths(0, 0, 0, 48), (0, 0, 0));
    }

    fn render_to_string(view: AuditView) -> String {
        let area = Rect::new(0, 0, 90, 24);
        let mut buf = Buffer::empty(area);
        view.render(area, &mut buf);
        let mut out = String::new();
        for y in 0..area.height {
            for x in 0..area.width {
                out.push_str(buf[(x, y)].symbol());
            }
            out.push('\n');
        }
        out
    }

    #[test]
    fn test_renders_loading_state_without_panic() {
        let text = render_to_string(AuditView::new(None, None, Theme::Dark));
        assert!(text.contains("Computing"), "got:\n{text}");
    }

    #[test]
    fn test_renders_error_state_without_panic() {
        let text = render_to_string(AuditView::new(None, Some("boom"), Theme::Dark));
        assert!(text.contains("Audit failed"), "got:\n{text}");
    }

    #[test]
    fn test_renders_report_with_headline() {
        let report = AuditReport {
            generated_for: date(2026, 6, 8),
            total_cache_only_days: 28,
            sources: vec![source("claude-code", Some(date(2026, 3, 1)), 72, 28, 0)],
        };
        let text = render_to_string(AuditView::new(Some(&report), None, Theme::Dark));
        assert!(text.contains("preserved"), "got:\n{text}");
        assert!(text.contains("28 days"), "got:\n{text}");
    }
}
