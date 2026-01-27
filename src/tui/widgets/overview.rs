//! Overview layout widget

use chrono::{Duration, NaiveDate};
use ratatui::{
    buffer::Buffer,
    layout::{Alignment, Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Paragraph, Widget},
};

use super::heatmap::Heatmap;
use super::legend::Legend;
use super::tabs::{Tab, TabBar};
use crate::types::TotalSummary;

/// Format a number with thousand separators (e.g., 1234567 -> "1,234,567")
pub fn format_number(n: u64) -> String {
    if n == 0 {
        return "0".to_string();
    }

    let s = n.to_string();
    let mut result = String::with_capacity(s.len() + s.len() / 3);
    let chars: Vec<char> = s.chars().collect();

    for (i, ch) in chars.iter().enumerate() {
        if i > 0 && (chars.len() - i).is_multiple_of(3) {
            result.push(',');
        }
        result.push(*ch);
    }

    result
}

/// Format a number in compact form with K/M suffix (e.g., 1234567 -> "1.2M")
pub fn format_compact(n: u64) -> String {
    if n >= 1_000_000 {
        format!("{:.1}M", n as f64 / 1_000_000.0)
    } else if n >= 1_000 {
        format!("{:.0}K", n as f64 / 1_000.0)
    } else {
        n.to_string()
    }
}

/// Summary data for a time period
#[derive(Debug, Clone, Default)]
pub struct PeriodSummary {
    pub total_tokens: u64,
    pub input_tokens: u64,
    pub output_tokens: u64,
}

/// Compute summary for a date range (inclusive)
pub fn compute_period_summary(
    daily_summaries: &[(NaiveDate, u64, u64, u64)], // (date, total, input, output)
    start: NaiveDate,
    end: NaiveDate,
) -> PeriodSummary {
    let mut summary = PeriodSummary::default();

    for (date, total, input, output) in daily_summaries {
        if *date >= start && *date <= end {
            summary.total_tokens += total;
            summary.input_tokens += input;
            summary.output_tokens += output;
        }
    }

    summary
}

/// Compute week summary (last 7 days including today)
pub fn compute_week_summary(
    daily_summaries: &[(NaiveDate, u64, u64, u64)],
    today: NaiveDate,
) -> PeriodSummary {
    let week_start = today - Duration::days(6);
    compute_period_summary(daily_summaries, week_start, today)
}

/// Compute month summary (last 30 days including today)
pub fn compute_month_summary(
    daily_summaries: &[(NaiveDate, u64, u64, u64)],
    today: NaiveDate,
) -> PeriodSummary {
    let month_start = today - Duration::days(29);
    compute_period_summary(daily_summaries, month_start, today)
}

/// Data for the overview display
#[derive(Debug, Clone)]
pub struct OverviewData {
    pub total: TotalSummary,
    pub today_tokens: u64,
    pub week_summary: PeriodSummary,
    pub month_summary: PeriodSummary,
    pub daily_tokens: Vec<(NaiveDate, u64)>,
}

/// Maximum content width for Overview (keeps layout clean on wide terminals)
/// 52 weeks * 2-char cells + 4 label = 108, so 120 gives some padding
const MAX_CONTENT_WIDTH: u16 = 120;

/// Overview widget combining all elements
pub struct Overview<'a> {
    data: &'a OverviewData,
    today: NaiveDate,
    selected_tab: Tab,
}

impl<'a> Overview<'a> {
    pub fn new(data: &'a OverviewData, today: NaiveDate) -> Self {
        Self {
            data,
            today,
            selected_tab: Tab::Overview,
        }
    }

    pub fn with_tab(mut self, tab: Tab) -> Self {
        self.selected_tab = tab;
        self
    }
}

impl Widget for Overview<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        // Apply max width constraint and center the content
        let content_width = area.width.min(MAX_CONTENT_WIDTH);
        let x_offset = (area.width.saturating_sub(content_width)) / 2;
        let centered_area = Rect {
            x: area.x + x_offset,
            y: area.y,
            width: content_width,
            height: area.height,
        };

        // Fixed-height layout (no expansion, keybindings stay with content):
        // - Top padding (1) + Tabs (1) + Separator (1) + Hero (3) + Sub-stats (1) + Blank (1)
        // - Heatmap (16: 7×2 rows + month labels + legend) + Separator (1) + Keybindings (1) = 26 total
        let chunks = Layout::vertical([
            Constraint::Length(1),  // Top padding (NEW)
            Constraint::Length(1),  // Tabs
            Constraint::Length(1),  // Separator
            Constraint::Length(3),  // Hero stat
            Constraint::Length(1),  // Sub-stats
            Constraint::Length(1),  // Blank
            Constraint::Length(16), // Heatmap (7×2=14 rows) + month labels + legend
            Constraint::Length(1),  // Separator
            Constraint::Length(1),  // Keybindings
        ])
        .split(centered_area);

        // Top padding (chunks[0]) - nothing to render

        // Render tabs
        self.render_tabs(chunks[1], buf);

        // Render separator
        self.render_separator(chunks[2], buf);

        // Render hero stat
        self.render_hero_stat(chunks[3], buf);

        // Render sub-stats
        self.render_sub_stats(chunks[4], buf);

        // Blank line (chunks[5]) - nothing to render

        // Render heatmap with legend
        self.render_heatmap_section(chunks[6], buf);

        // Render separator
        self.render_separator(chunks[7], buf);

        // Render keybindings
        self.render_keybindings(chunks[8], buf);
    }
}

impl Overview<'_> {
    fn render_tabs(&self, area: Rect, buf: &mut Buffer) {
        let tab_bar = TabBar::new(self.selected_tab);
        tab_bar.render(area, buf);
    }

    fn render_separator(&self, area: Rect, buf: &mut Buffer) {
        let line = "─".repeat(area.width as usize);
        buf.set_string(area.x, area.y, &line, Style::default().fg(Color::DarkGray));
    }

    fn render_hero_stat(&self, area: Rect, buf: &mut Buffer) {
        let total_tokens = self.data.total.total_input_tokens + self.data.total.total_output_tokens;
        let formatted = format_number(total_tokens);

        // Center the hero number
        let hero = Paragraph::new(vec![
            Line::from(Span::styled(
                &formatted,
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            )),
            Line::from(Span::styled("tokens", Style::default().fg(Color::DarkGray))),
        ])
        .alignment(Alignment::Center);

        hero.render(area, buf);
    }

    fn render_sub_stats(&self, area: Rect, buf: &mut Buffer) {
        // Format: Today (1d), Week (7d), Month (30d), Cost
        let today_str = format!("Today: {}", format_compact(self.data.today_tokens));
        let week_str = format!(
            "Week: {}",
            format_compact(self.data.week_summary.total_tokens)
        );
        let month_str = format!(
            "Month: {}",
            format_compact(self.data.month_summary.total_tokens)
        );
        let cost_str = format!("Cost: ${:.2}", self.data.total.total_cost_usd);

        let stats = Paragraph::new(Line::from(vec![
            Span::raw(" "),
            Span::styled(today_str, Style::default().fg(Color::Green)),
            Span::raw("    "),
            Span::styled(week_str, Style::default().fg(Color::Yellow)),
            Span::raw("    "),
            Span::styled(month_str, Style::default().fg(Color::Cyan)),
            Span::raw("    "),
            Span::styled(cost_str, Style::default().fg(Color::Magenta)),
        ]));

        stats.render(area, buf);
    }

    fn render_heatmap_section(&self, area: Rect, buf: &mut Buffer) {
        // Heatmap takes 7×2=14 rows (Mon-Sun with 2-row cells) + 1 row month labels + 1 row legend = 16 rows
        let weeks = Heatmap::weeks_for_width(area.width);
        let heatmap = Heatmap::new(&self.data.daily_tokens, self.today, weeks);
        heatmap.render(area, buf);

        // Legend on last row - aligned to heatmap grid right edge
        if area.height >= 16 {
            // Calculate actual heatmap width: label (4) + weeks * cell_width (2)
            let heatmap_width = 4 + (weeks as u16 * 2);
            let legend_area = Rect {
                x: area.x,
                y: area.y + 15, // 14 rows for heatmap + 1 row for month labels
                width: heatmap_width.min(area.width),
                height: 1,
            };
            Legend::new().render(legend_area, buf);
        }
    }

    fn render_keybindings(&self, area: Rect, buf: &mut Buffer) {
        let bindings = Paragraph::new(Line::from(vec![
            Span::styled("q", Style::default().fg(Color::Cyan)),
            Span::styled(": Quit", Style::default().fg(Color::DarkGray)),
            Span::raw("  "),
            Span::styled("Tab", Style::default().fg(Color::Cyan)),
            Span::styled(": Switch view", Style::default().fg(Color::DarkGray)),
            Span::raw("  "),
            Span::styled("?", Style::default().fg(Color::Cyan)),
            Span::styled(": Help", Style::default().fg(Color::DarkGray)),
        ]))
        .alignment(Alignment::Center);

        bindings.render(area, buf);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ========== format_number tests ==========

    #[test]
    fn test_format_number_zero() {
        assert_eq!(format_number(0), "0");
    }

    #[test]
    fn test_format_number_small() {
        assert_eq!(format_number(999), "999");
    }

    #[test]
    fn test_format_number_thousand() {
        assert_eq!(format_number(1000), "1,000");
    }

    #[test]
    fn test_format_number_large() {
        assert_eq!(format_number(1234567), "1,234,567");
    }

    #[test]
    fn test_format_number_million() {
        assert_eq!(format_number(1000000), "1,000,000");
    }

    // ========== compute_period_summary tests ==========

    #[test]
    fn test_compute_period_summary_empty() {
        let result = compute_period_summary(
            &[],
            NaiveDate::from_ymd_opt(2024, 1, 1).unwrap(),
            NaiveDate::from_ymd_opt(2024, 1, 7).unwrap(),
        );
        assert_eq!(result.total_tokens, 0);
    }

    #[test]
    fn test_compute_period_summary_filters_by_date() {
        let data = vec![
            (NaiveDate::from_ymd_opt(2024, 1, 1).unwrap(), 100, 60, 40),
            (NaiveDate::from_ymd_opt(2024, 1, 5).unwrap(), 200, 120, 80),
            (NaiveDate::from_ymd_opt(2024, 1, 10).unwrap(), 300, 180, 120),
        ];

        // Only Jan 1 and Jan 5 should be included
        let result = compute_period_summary(
            &data,
            NaiveDate::from_ymd_opt(2024, 1, 1).unwrap(),
            NaiveDate::from_ymd_opt(2024, 1, 7).unwrap(),
        );

        assert_eq!(result.total_tokens, 300); // 100 + 200
        assert_eq!(result.input_tokens, 180); // 60 + 120
        assert_eq!(result.output_tokens, 120); // 40 + 80
    }

    // ========== compute_week_summary tests ==========

    #[test]
    fn test_compute_week_summary() {
        let today = NaiveDate::from_ymd_opt(2024, 1, 15).unwrap();
        let data = vec![
            (NaiveDate::from_ymd_opt(2024, 1, 9).unwrap(), 100, 60, 40), // 6 days ago
            (NaiveDate::from_ymd_opt(2024, 1, 10).unwrap(), 200, 120, 80), // 5 days ago
            (NaiveDate::from_ymd_opt(2024, 1, 15).unwrap(), 300, 180, 120), // today
            (NaiveDate::from_ymd_opt(2024, 1, 1).unwrap(), 1000, 600, 400), // too old
        ];

        let result = compute_week_summary(&data, today);

        assert_eq!(result.total_tokens, 600); // 100 + 200 + 300
        assert_eq!(result.input_tokens, 360); // 60 + 120 + 180
        assert_eq!(result.output_tokens, 240); // 40 + 80 + 120
    }
}
