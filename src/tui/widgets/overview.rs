//! Overview layout widget

use chrono::{Duration, NaiveDate};
use ratatui::{
    buffer::Buffer,
    layout::{Constraint, Layout, Rect},
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Paragraph, Widget},
};

use super::heatmap::Heatmap;
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

/// Data for the overview display
#[derive(Debug, Clone)]
pub struct OverviewData {
    pub total: TotalSummary,
    pub today_tokens: u64,
    pub week_summary: PeriodSummary,
    pub daily_tokens: Vec<(NaiveDate, u64)>,
}

/// Overview widget combining all elements
pub struct Overview<'a> {
    data: &'a OverviewData,
    today: NaiveDate,
}

impl<'a> Overview<'a> {
    pub fn new(data: &'a OverviewData, today: NaiveDate) -> Self {
        Self { data, today }
    }
}

impl Widget for Overview<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        // Main layout: header, heatmap, footer
        let chunks = Layout::vertical([
            Constraint::Length(2), // Header
            Constraint::Min(7),    // Heatmap (7 rows for weekdays)
            Constraint::Length(2), // Footer
        ])
        .split(area);

        // Render header
        self.render_header(chunks[0], buf);

        // Render heatmap
        self.render_heatmap(chunks[1], buf);

        // Render footer
        self.render_footer(chunks[2], buf);
    }
}

impl Overview<'_> {
    fn render_header(&self, area: Rect, buf: &mut Buffer) {
        let total_tokens = self.data.total.total_input_tokens + self.data.total.total_output_tokens;
        let cost = self.data.total.total_cost_usd;

        let title = format!(
            "toktrack - Total: {} tokens (${:.2})",
            format_number(total_tokens),
            cost
        );

        let subtitle = format!(
            "Input: {}K | Output: {}K | Days: {}",
            format_number(self.data.total.total_input_tokens / 1000),
            format_number(self.data.total.total_output_tokens / 1000),
            self.data.total.day_count
        );

        let header = Paragraph::new(vec![
            Line::from(Span::styled(title, Style::default().fg(Color::Cyan))),
            Line::from(Span::styled(subtitle, Style::default().fg(Color::DarkGray))),
        ]);

        header.render(area, buf);
    }

    fn render_heatmap(&self, area: Rect, buf: &mut Buffer) {
        let heatmap = Heatmap::new(&self.data.daily_tokens, self.today);
        heatmap.render(area, buf);
    }

    fn render_footer(&self, area: Rect, buf: &mut Buffer) {
        let today_str = format!("Today: {} tokens", format_number(self.data.today_tokens));
        let week_str = format!(
            "Week: {} tokens",
            format_number(self.data.week_summary.total_tokens)
        );

        let footer = Paragraph::new(vec![
            Line::from(vec![
                Span::styled(today_str, Style::default().fg(Color::Green)),
                Span::raw(" | "),
                Span::styled(week_str, Style::default().fg(Color::Yellow)),
            ]),
            Line::from(Span::styled(
                "Press 'q' to quit",
                Style::default().fg(Color::DarkGray),
            )),
        ]);

        footer.render(area, buf);
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
