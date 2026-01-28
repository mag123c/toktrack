//! Daily view widget - displays per-day usage statistics with sparklines

use ratatui::{
    buffer::Buffer,
    layout::{Alignment, Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Paragraph, Widget},
};

use super::overview::format_number;
use super::tabs::{Tab, TabBar};
use crate::types::DailySummary;

/// Format a sparkline bar based on token ratio
/// Example: tokens=500, max=1000, width=8 → "████░░░░"
pub fn format_sparkline(tokens: u64, max: u64, width: usize) -> String {
    if max == 0 || width == 0 {
        return "░".repeat(width);
    }
    let ratio = tokens as f64 / max as f64;
    let filled = (ratio * width as f64).round() as usize;
    let filled = filled.min(width); // Clamp to prevent overflow when ratio > 1.0
    let empty = width.saturating_sub(filled);
    format!("{}{}", "▓".repeat(filled), "░".repeat(empty))
}

/// Data for the daily view
#[derive(Debug)]
pub struct DailyData {
    /// Daily summaries sorted by date descending (newest first)
    pub summaries: Vec<DailySummary>,
    /// Maximum total tokens across all days (for sparkline scaling)
    pub max_tokens: u64,
}

impl DailyData {
    /// Create DailyData from aggregated daily summaries
    /// Expects summaries in ascending order (from Aggregator::daily), reverses to descending
    pub fn from_daily_summaries(mut summaries: Vec<DailySummary>) -> Self {
        // Calculate max tokens before reversing
        let max_tokens = summaries
            .iter()
            .map(|s| {
                s.total_input_tokens
                    + s.total_output_tokens
                    + s.total_cache_read_tokens
                    + s.total_cache_creation_tokens
            })
            .max()
            .unwrap_or(0);

        // Reverse to get descending order (newest first)
        summaries.reverse();

        Self {
            summaries,
            max_tokens,
        }
    }
}

/// Maximum content width for Daily view (consistent with Overview/Models)
const MAX_CONTENT_WIDTH: u16 = 170;

/// Table width: Date(12) + Model(25) + Input(18) + Output(18) + Cache(18) + Total(18) + Cost(12) + Usage(18) = 139
const TABLE_WIDTH: u16 = 139;

/// Visible rows for scrolling (excluding header)
const VISIBLE_ROWS: usize = 15;

/// Daily view widget
pub struct DailyView<'a> {
    data: &'a DailyData,
    scroll_offset: usize,
    selected_tab: Tab,
}

impl<'a> DailyView<'a> {
    pub fn new(data: &'a DailyData, scroll_offset: usize) -> Self {
        Self {
            data,
            scroll_offset,
            selected_tab: Tab::Daily,
        }
    }

    pub fn with_tab(mut self, tab: Tab) -> Self {
        self.selected_tab = tab;
        self
    }

    /// Calculate the maximum valid scroll offset
    pub fn max_scroll_offset(data: &DailyData) -> usize {
        data.summaries.len().saturating_sub(VISIBLE_ROWS)
    }
}

impl Widget for DailyView<'_> {
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

        // Calculate layout
        let visible_rows = self.data.summaries.len().min(VISIBLE_ROWS) as u16;
        let chunks = Layout::vertical([
            Constraint::Length(1),            // Top padding
            Constraint::Length(1),            // Tabs
            Constraint::Length(1),            // Separator
            Constraint::Length(1),            // Header
            Constraint::Length(visible_rows), // Daily rows
            Constraint::Length(1),            // Separator
            Constraint::Length(1),            // Keybindings
            Constraint::Min(0),               // Remaining space
        ])
        .split(centered_area);

        // Render tabs
        self.render_tabs(chunks[1], buf);

        // Render separator
        self.render_separator(chunks[2], buf);

        // Render header
        self.render_header(chunks[3], buf);

        // Render daily rows
        self.render_daily_rows(chunks[4], buf);

        // Render separator
        self.render_separator(chunks[5], buf);

        // Render keybindings
        self.render_keybindings(chunks[6], buf);
    }
}

impl DailyView<'_> {
    /// Calculate horizontal offset to center the table
    fn calculate_table_offset(&self, area_width: u16) -> u16 {
        area_width.saturating_sub(TABLE_WIDTH) / 2
    }

    fn render_tabs(&self, area: Rect, buf: &mut Buffer) {
        let tab_bar = TabBar::new(self.selected_tab);
        tab_bar.render(area, buf);
    }

    fn render_separator(&self, area: Rect, buf: &mut Buffer) {
        let line = "─".repeat(area.width as usize);
        buf.set_string(area.x, area.y, &line, Style::default().fg(Color::DarkGray));
    }

    fn render_header(&self, area: Rect, buf: &mut Buffer) {
        let offset = self.calculate_table_offset(area.width);

        // Column widths: Date(12), Model(25), Input(18), Output(18), Cache(18), Total(18), Cost(12), Usage(18)
        let header = Line::from(vec![
            Span::styled(
                format!("{:<12}", "Date"),
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!("{:<25}", "Model"),
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!("{:>18}", "Input"),
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!("{:>18}", "Output"),
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!("{:>18}", "Cache"),
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!("{:>18}", "Total"),
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!("{:>12}", "Cost"),
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!("{:>18}", "Usage"),
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            ),
        ]);

        let paragraph = Paragraph::new(header).alignment(Alignment::Left);
        paragraph.render(
            Rect {
                x: area.x + offset,
                y: area.y,
                width: TABLE_WIDTH.min(area.width),
                height: area.height,
            },
            buf,
        );
    }

    fn render_daily_rows(&self, area: Rect, buf: &mut Buffer) {
        let offset = self.calculate_table_offset(area.width);
        let start = self.scroll_offset;
        let end = (start + area.height as usize).min(self.data.summaries.len());

        for (i, summary) in self.data.summaries[start..end].iter().enumerate() {
            let y = area.y + i as u16;
            if y >= area.y + area.height {
                break;
            }

            self.render_daily_row(
                Rect {
                    x: area.x + offset,
                    y,
                    width: TABLE_WIDTH.min(area.width),
                    height: 1,
                },
                buf,
                summary,
            );
        }
    }

    fn render_daily_row(&self, area: Rect, buf: &mut Buffer, summary: &DailySummary) {
        let total_tokens = summary.total_input_tokens
            + summary.total_output_tokens
            + summary.total_cache_read_tokens
            + summary.total_cache_creation_tokens;

        let cache_tokens = summary.total_cache_read_tokens + summary.total_cache_creation_tokens;

        // Get primary model (first one, or "mixed" if multiple)
        let model_name = if summary.models.len() == 1 {
            summary
                .models
                .keys()
                .next()
                .cloned()
                .unwrap_or_else(|| "unknown".to_string())
        } else if summary.models.is_empty() {
            "unknown".to_string()
        } else {
            format!("{} models", summary.models.len())
        };

        // Truncate model name if too long (UTF-8 safe)
        let model_display = if model_name.chars().count() > 23 {
            format!("{}…", model_name.chars().take(22).collect::<String>())
        } else {
            model_name
        };

        let sparkline = format_sparkline(total_tokens, self.data.max_tokens, 14);

        let row = Line::from(vec![
            Span::styled(
                format!("{:<12}", summary.date.format("%Y-%m-%d")),
                Style::default().fg(Color::Yellow),
            ),
            Span::styled(
                format!("{:<25}", model_display),
                Style::default().fg(Color::Cyan),
            ),
            Span::styled(
                format!("{:>18}", format_number(summary.total_input_tokens)),
                Style::default().fg(Color::White),
            ),
            Span::styled(
                format!("{:>18}", format_number(summary.total_output_tokens)),
                Style::default().fg(Color::White),
            ),
            Span::styled(
                format!("{:>18}", format_number(cache_tokens)),
                Style::default().fg(Color::White),
            ),
            Span::styled(
                format!("{:>18}", format_number(total_tokens)),
                Style::default().fg(Color::White),
            ),
            Span::styled(
                format!("{:>12}", format!("${:.2}", summary.total_cost_usd)),
                Style::default().fg(Color::Magenta),
            ),
            Span::styled(
                format!("{:>18}", sparkline),
                Style::default().fg(Color::Green),
            ),
        ]);

        let paragraph = Paragraph::new(row).alignment(Alignment::Left);
        paragraph.render(area, buf);
    }

    fn render_keybindings(&self, area: Rect, buf: &mut Buffer) {
        let bindings = Paragraph::new(Line::from(vec![
            Span::styled("↑↓", Style::default().fg(Color::Cyan)),
            Span::styled(": Scroll", Style::default().fg(Color::DarkGray)),
            Span::raw("  "),
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
    use chrono::NaiveDate;
    use std::collections::HashMap;

    // ========== format_sparkline tests ==========

    #[test]
    fn test_format_sparkline_zero() {
        assert_eq!(format_sparkline(0, 1000, 8), "░░░░░░░░");
    }

    #[test]
    fn test_format_sparkline_max() {
        assert_eq!(format_sparkline(1000, 1000, 8), "▓▓▓▓▓▓▓▓");
    }

    #[test]
    fn test_format_sparkline_half() {
        assert_eq!(format_sparkline(500, 1000, 8), "▓▓▓▓░░░░");
    }

    #[test]
    fn test_format_sparkline_zero_max() {
        // When max is 0, should return all empty
        assert_eq!(format_sparkline(100, 0, 8), "░░░░░░░░");
    }

    #[test]
    fn test_format_sparkline_zero_width() {
        assert_eq!(format_sparkline(500, 1000, 0), "");
    }

    #[test]
    fn test_format_sparkline_overflow_ratio() {
        // When tokens > max (ratio > 1.0), should clamp to width
        assert_eq!(format_sparkline(2000, 1000, 8), "▓▓▓▓▓▓▓▓");
    }

    // ========== DailyData tests ==========

    #[allow(clippy::too_many_arguments)]
    fn make_daily_summary(
        year: i32,
        month: u32,
        day: u32,
        input: u64,
        output: u64,
        cache_read: u64,
        cache_creation: u64,
        cost: f64,
    ) -> DailySummary {
        DailySummary {
            date: NaiveDate::from_ymd_opt(year, month, day).unwrap(),
            total_input_tokens: input,
            total_output_tokens: output,
            total_cache_read_tokens: cache_read,
            total_cache_creation_tokens: cache_creation,
            total_cost_usd: cost,
            models: HashMap::new(),
        }
    }

    #[test]
    fn test_daily_data_empty() {
        let data = DailyData::from_daily_summaries(vec![]);
        assert!(data.summaries.is_empty());
        assert_eq!(data.max_tokens, 0);
    }

    #[test]
    fn test_daily_data_sorted_desc() {
        // Input is ascending (as from Aggregator::daily)
        let summaries = vec![
            make_daily_summary(2024, 1, 10, 100, 50, 10, 5, 0.01),
            make_daily_summary(2024, 1, 15, 200, 100, 20, 10, 0.02),
            make_daily_summary(2024, 1, 20, 300, 150, 30, 15, 0.03),
        ];

        let data = DailyData::from_daily_summaries(summaries);

        assert_eq!(data.summaries.len(), 3);
        // Should be reversed (newest first)
        assert_eq!(data.summaries[0].date.to_string(), "2024-01-20");
        assert_eq!(data.summaries[1].date.to_string(), "2024-01-15");
        assert_eq!(data.summaries[2].date.to_string(), "2024-01-10");
    }

    #[test]
    fn test_daily_data_max_tokens() {
        let summaries = vec![
            make_daily_summary(2024, 1, 10, 100, 50, 10, 5, 0.01), // total: 165
            make_daily_summary(2024, 1, 15, 200, 100, 20, 10, 0.02), // total: 330
            make_daily_summary(2024, 1, 20, 300, 150, 30, 15, 0.03), // total: 495
        ];

        let data = DailyData::from_daily_summaries(summaries);

        assert_eq!(data.max_tokens, 495);
    }

    // ========== DailyView scroll tests ==========

    #[test]
    fn test_daily_view_scroll_bounds_empty() {
        let data = DailyData::from_daily_summaries(vec![]);
        assert_eq!(DailyView::max_scroll_offset(&data), 0);
    }

    #[test]
    fn test_daily_view_scroll_bounds_less_than_visible() {
        let summaries = vec![
            make_daily_summary(2024, 1, 10, 100, 50, 10, 5, 0.01),
            make_daily_summary(2024, 1, 15, 200, 100, 20, 10, 0.02),
        ];
        let data = DailyData::from_daily_summaries(summaries);
        // 2 items < VISIBLE_ROWS (15), so max offset is 0
        assert_eq!(DailyView::max_scroll_offset(&data), 0);
    }

    #[test]
    fn test_daily_view_scroll_bounds_more_than_visible() {
        let summaries: Vec<DailySummary> = (1..=20)
            .map(|d| make_daily_summary(2024, 1, d, 100, 50, 10, 5, 0.01))
            .collect();
        let data = DailyData::from_daily_summaries(summaries);
        // 20 items, VISIBLE_ROWS = 15, so max offset = 5
        assert_eq!(DailyView::max_scroll_offset(&data), 5);
    }
}
