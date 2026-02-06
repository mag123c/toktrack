//! Daily view widget - displays per-day usage statistics with sparklines

use ratatui::{
    buffer::Buffer,
    layout::{Alignment, Constraint, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Paragraph, Widget},
};

use super::overview::format_number;
use super::tabs::{Tab, TabBar};
use crate::services::{display_name, Aggregator};
use crate::tui::theme::{spike_level, Theme};
use crate::types::DailySummary;

/// View mode within the Daily tab
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DailyViewMode {
    #[default]
    Daily,
    Weekly,
    Monthly,
}

impl DailyViewMode {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Daily => "Daily",
            Self::Weekly => "Weekly",
            Self::Monthly => "Monthly",
        }
    }

    pub fn date_column_label(&self) -> &'static str {
        match self {
            Self::Daily => "Date",
            Self::Weekly => "Week",
            Self::Monthly => "Month",
        }
    }
}

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

/// Data for the daily view (holds daily, weekly, and monthly aggregations)
#[derive(Debug)]
pub struct DailyData {
    /// Daily summaries sorted by date ascending (oldest first)
    pub daily_summaries: Vec<DailySummary>,
    pub daily_max_tokens: u64,
    pub weekly_summaries: Vec<DailySummary>,
    pub weekly_max_tokens: u64,
    pub monthly_summaries: Vec<DailySummary>,
    pub monthly_max_tokens: u64,
}

impl DailyData {
    /// Create DailyData from aggregated daily summaries
    /// Expects summaries in ascending order (from Aggregator::daily)
    pub fn from_daily_summaries(summaries: Vec<DailySummary>) -> Self {
        let calc_max = |s: &[DailySummary]| -> u64 {
            s.iter()
                .map(|d| {
                    d.total_input_tokens
                        + d.total_output_tokens
                        + d.total_cache_read_tokens
                        + d.total_cache_creation_tokens
                        + d.total_thinking_tokens
                })
                .max()
                .unwrap_or(0)
        };

        let weekly_summaries = Aggregator::weekly(&summaries);
        let monthly_summaries = Aggregator::monthly(&summaries);

        let daily_max_tokens = calc_max(&summaries);
        let weekly_max_tokens = calc_max(&weekly_summaries);
        let monthly_max_tokens = calc_max(&monthly_summaries);

        Self {
            daily_summaries: summaries,
            daily_max_tokens,
            weekly_summaries,
            weekly_max_tokens,
            monthly_summaries,
            monthly_max_tokens,
        }
    }

    /// Get summaries and max_tokens for the given view mode
    pub fn for_mode(&self, mode: DailyViewMode) -> (&[DailySummary], u64) {
        match mode {
            DailyViewMode::Daily => (&self.daily_summaries, self.daily_max_tokens),
            DailyViewMode::Weekly => (&self.weekly_summaries, self.weekly_max_tokens),
            DailyViewMode::Monthly => (&self.monthly_summaries, self.monthly_max_tokens),
        }
    }

    /// Calculate maximum scroll offset for a given item count
    pub fn max_scroll_offset_for(count: usize) -> usize {
        count.saturating_sub(VISIBLE_ROWS)
    }
}

/// Maximum content width for Daily view (consistent with Overview/Models)
const MAX_CONTENT_WIDTH: u16 = 170;

/// Visible rows for scrolling (excluding header)
pub const VISIBLE_ROWS: usize = 15;

/// Column index constants for clarity
const COL_DATE: usize = 0;
const COL_MODEL: usize = 1;
const COL_TOTAL: usize = 2;
const COL_COST: usize = 3;
const COL_INPUT: usize = 4;
const COL_OUTPUT: usize = 5;
const COL_CACHE: usize = 6;
const COL_USAGE: usize = 7;

/// Column definition: (label, width). Core columns (0-3) are never hidden.
/// Date width includes 2 chars for selection marker (▸ )
const COLUMNS: [(&str, u16); 8] = [
    ("Date", 14),   // 0: COL_DATE (12 date + 2 marker)
    ("Model", 25),  // 1: COL_MODEL
    ("Total", 18),  // 2: COL_TOTAL
    ("Cost", 12),   // 3: COL_COST
    ("Input", 18),  // 4: COL_INPUT
    ("Output", 18), // 5: COL_OUTPUT
    ("Cache", 18),  // 6: COL_CACHE
    ("Usage", 18),  // 7: COL_USAGE
];

/// Determine which column indices are visible for a given terminal width.
/// Columns are hidden in priority order: Input first, then Output, Cache, Usage.
/// This prioritizes showing Usage (visual bar) in narrow views.
fn visible_columns(width: u16) -> Vec<usize> {
    // Ordered by hide priority: first element is hidden first
    const HIDE_ORDER: [usize; 4] = [COL_INPUT, COL_OUTPUT, COL_CACHE, COL_USAGE];

    let mut visible: Vec<usize> = (0..COLUMNS.len()).collect();

    for &col_idx in &HIDE_ORDER {
        let total: u16 = visible.iter().map(|&i| COLUMNS[i].1).sum();
        if total <= width {
            return visible;
        }
        visible.retain(|&i| i != col_idx);
    }

    visible
}

/// Calculate total table width for a set of visible column indices.
fn table_width_for(visible: &[usize]) -> u16 {
    visible.iter().map(|&i| COLUMNS[i].1).sum()
}

/// Daily view widget
pub struct DailyView<'a> {
    data: &'a DailyData,
    scroll_offset: usize,
    selected_index: Option<usize>,
    selected_tab: Tab,
    view_mode: DailyViewMode,
    theme: Theme,
    avg_cost: f64,
}

impl<'a> DailyView<'a> {
    pub fn new(
        data: &'a DailyData,
        scroll_offset: usize,
        view_mode: DailyViewMode,
        theme: Theme,
        avg_cost: f64,
    ) -> Self {
        Self {
            data,
            scroll_offset,
            selected_index: None,
            selected_tab: Tab::Daily,
            view_mode,
            theme,
            avg_cost,
        }
    }

    pub fn with_tab(mut self, tab: Tab) -> Self {
        self.selected_tab = tab;
        self
    }

    pub fn with_selected_index(mut self, selected_index: Option<usize>) -> Self {
        self.selected_index = selected_index;
        self
    }

    /// Calculate the maximum valid scroll offset for the given mode
    pub fn max_scroll_offset(data: &DailyData, mode: DailyViewMode) -> usize {
        let (summaries, _) = data.for_mode(mode);
        DailyData::max_scroll_offset_for(summaries.len())
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

        // Determine visible columns based on available width
        let visible = visible_columns(centered_area.width);

        let (summaries, _) = self.data.for_mode(self.view_mode);

        // Calculate layout
        let visible_rows = summaries.len().min(VISIBLE_ROWS) as u16;
        let chunks = Layout::vertical([
            Constraint::Length(1),            // Top padding
            Constraint::Length(1),            // Tabs
            Constraint::Length(1),            // Separator
            Constraint::Length(1),            // Mode indicator
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

        // Render mode indicator
        self.render_mode_indicator(chunks[3], buf);

        // Render header
        self.render_header(chunks[4], buf, &visible);

        // Render daily rows
        self.render_daily_rows(chunks[5], buf, &visible);

        // Render separator
        self.render_separator(chunks[6], buf);

        // Render keybindings
        self.render_keybindings(chunks[7], buf);
    }
}

impl DailyView<'_> {
    /// Calculate horizontal offset to center the table
    fn calculate_table_offset(area_width: u16, tw: u16) -> u16 {
        area_width.saturating_sub(tw) / 2
    }

    fn render_tabs(&self, area: Rect, buf: &mut Buffer) {
        let tab_bar = TabBar::new(self.selected_tab, self.theme);
        tab_bar.render(area, buf);
    }

    fn render_separator(&self, area: Rect, buf: &mut Buffer) {
        let line = "─".repeat(area.width as usize);
        buf.set_string(
            area.x,
            area.y,
            &line,
            Style::default().fg(self.theme.muted()),
        );
    }

    fn render_mode_indicator(&self, area: Rect, buf: &mut Buffer) {
        let modes = [
            ('d', DailyViewMode::Daily),
            ('w', DailyViewMode::Weekly),
            ('m', DailyViewMode::Monthly),
        ];

        let mut spans = Vec::new();
        for (i, (key, mode)) in modes.iter().enumerate() {
            if i > 0 {
                spans.push(Span::raw("  "));
            }
            let is_active = *mode == self.view_mode;
            let style = if is_active {
                Style::default()
                    .fg(self.theme.accent())
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(self.theme.text())
            };
            spans.push(Span::styled(format!("{}:{}", key, mode.label()), style));
        }

        let indicator = Paragraph::new(Line::from(spans)).alignment(Alignment::Center);
        indicator.render(area, buf);
    }

    fn render_header(&self, area: Rect, buf: &mut Buffer, visible: &[usize]) {
        let tw = table_width_for(visible);
        let offset = Self::calculate_table_offset(area.width, tw);
        let date_label = self.view_mode.date_column_label();
        let header_style = Style::default()
            .fg(self.theme.text())
            .add_modifier(Modifier::BOLD);

        let mut spans = Vec::new();
        for &col in visible {
            let (label, width) = COLUMNS[col];
            let label = if col == COL_DATE { date_label } else { label };
            let formatted = if col == COL_DATE {
                // Add 2-space prefix to align with selection marker in rows
                format!("  {:<width$}", label, width = (width as usize) - 2)
            } else if col == COL_MODEL {
                format!("{:<width$}", label, width = width as usize)
            } else {
                format!("{:>width$}", label, width = width as usize)
            };
            spans.push(Span::styled(formatted, header_style));
        }

        let header = Line::from(spans);
        let paragraph = Paragraph::new(header).alignment(Alignment::Left);
        paragraph.render(
            Rect {
                x: area.x + offset,
                y: area.y,
                width: tw.min(area.width),
                height: area.height,
            },
            buf,
        );
    }

    fn render_daily_rows(&self, area: Rect, buf: &mut Buffer, visible: &[usize]) {
        let tw = table_width_for(visible);
        let offset = Self::calculate_table_offset(area.width, tw);
        let (summaries, max_tokens) = self.data.for_mode(self.view_mode);
        let start = self.scroll_offset;
        let end = (start + area.height as usize).min(summaries.len());

        for (i, summary) in summaries[start..end].iter().enumerate() {
            let y = area.y + i as u16;
            if y >= area.y + area.height {
                break;
            }

            let data_index = start + i;
            let is_selected = self.selected_index == Some(data_index);

            self.render_daily_row(
                Rect {
                    x: area.x + offset,
                    y,
                    width: tw.min(area.width),
                    height: 1,
                },
                buf,
                summary,
                max_tokens,
                visible,
                is_selected,
            );
        }
    }

    fn render_daily_row(
        &self,
        area: Rect,
        buf: &mut Buffer,
        summary: &DailySummary,
        max_tokens: u64,
        visible: &[usize],
        is_selected: bool,
    ) {
        let total_tokens = summary.total_input_tokens
            + summary.total_output_tokens
            + summary.total_cache_read_tokens
            + summary.total_cache_creation_tokens
            + summary.total_thinking_tokens;

        let cache_tokens = summary.total_cache_read_tokens + summary.total_cache_creation_tokens;

        // Get primary model (highest cost) + count of others, filtering out zero-token models
        let non_zero_models: Vec<_> = summary
            .models
            .iter()
            .filter(|(_, usage)| {
                let total = usage.input_tokens
                    + usage.output_tokens
                    + usage.cache_read_tokens
                    + usage.cache_creation_tokens;
                total > 0
            })
            .collect();

        // Separate primary model name and count suffix for different coloring
        let (primary_model, count_suffix) = if non_zero_models.len() == 1 {
            (display_name(non_zero_models[0].0), None)
        } else if non_zero_models.is_empty() {
            ("unknown".to_string(), None)
        } else {
            // Find model with highest cost among non-zero models
            let primary = non_zero_models
                .iter()
                .max_by(|a, b| {
                    a.1.cost_usd
                        .partial_cmp(&b.1.cost_usd)
                        .unwrap_or(std::cmp::Ordering::Equal)
                })
                .map(|(name, _)| display_name(name))
                .unwrap_or_else(|| "unknown".to_string());
            let others = non_zero_models.len() - 1;
            (primary, Some(format!(" +{}", others)))
        };

        // Truncate primary model name if too long (UTF-8 safe)
        // Reserve space for count suffix if present
        let max_primary_len = if count_suffix.is_some() { 20 } else { 23 };
        let primary_display = if primary_model.chars().count() > max_primary_len {
            format!(
                "{}…",
                primary_model
                    .chars()
                    .take(max_primary_len - 1)
                    .collect::<String>()
            )
        } else {
            primary_model
        };

        let sparkline = format_sparkline(total_tokens, max_tokens, 14);

        // Format date based on view mode
        let date_str = match self.view_mode {
            DailyViewMode::Daily | DailyViewMode::Weekly => {
                summary.date.format("%Y-%m-%d").to_string()
            }
            DailyViewMode::Monthly => summary.date.format("%Y-%m").to_string(),
        };

        // Selection marker and style modifier
        let selection_modifier = if is_selected {
            Modifier::BOLD | Modifier::REVERSED
        } else {
            Modifier::empty()
        };

        let mut spans = Vec::new();

        // Add selection marker for first column
        for (col_idx, &col) in visible.iter().enumerate() {
            // COL_MODEL is special: renders primary model (accent) + count (muted)
            if col == COL_MODEL {
                let accent_style = if is_selected && col_idx > 0 {
                    Style::default()
                        .fg(self.theme.accent())
                        .add_modifier(selection_modifier)
                } else {
                    Style::default().fg(self.theme.accent())
                };
                let muted_style = if is_selected && col_idx > 0 {
                    Style::default()
                        .fg(self.theme.muted())
                        .add_modifier(selection_modifier)
                } else {
                    Style::default().fg(self.theme.muted())
                };

                // Calculate padding: total column width is 25
                let suffix = count_suffix.as_deref().unwrap_or("");
                let content_len = primary_display.chars().count() + suffix.chars().count();
                let padding = 25usize.saturating_sub(content_len);

                spans.push(Span::styled(primary_display.clone(), accent_style));
                if !suffix.is_empty() {
                    spans.push(Span::styled(suffix.to_string(), muted_style));
                }
                spans.push(Span::styled(" ".repeat(padding), Style::default()));
                continue;
            }

            let (text, base_style) = match col {
                COL_DATE => {
                    // Prepend marker to date column
                    let marker = if is_selected { "▸ " } else { "  " };
                    // Adjust width: marker takes 2 chars, so date field is 12
                    (
                        format!("{}{:<12}", marker, date_str),
                        Style::default().fg(self.theme.date()),
                    )
                }
                COL_INPUT => (
                    format!("{:>18}", format_number(summary.total_input_tokens)),
                    Style::default().fg(self.theme.text()),
                ),
                COL_OUTPUT => (
                    format!("{:>18}", format_number(summary.total_output_tokens)),
                    Style::default().fg(self.theme.text()),
                ),
                COL_CACHE => (
                    format!("{:>18}", format_number(cache_tokens)),
                    Style::default().fg(self.theme.text()),
                ),
                COL_TOTAL => (
                    format!("{:>18}", format_number(total_tokens)),
                    Style::default().fg(self.theme.text()),
                ),
                COL_COST => {
                    let cost_color = if self.view_mode == DailyViewMode::Daily {
                        self.theme
                            .spike_color(spike_level(summary.total_cost_usd, self.avg_cost))
                    } else {
                        self.theme.text()
                    };
                    (
                        format!("{:>12}", format!("${:.2}", summary.total_cost_usd)),
                        Style::default().fg(cost_color),
                    )
                }
                COL_USAGE => (
                    format!("{:>18}", sparkline),
                    Style::default().fg(self.theme.bar()),
                ),
                _ => unreachable!(),
            };

            // Apply selection highlight to all columns except first (which has marker)
            let style = if is_selected && col_idx > 0 {
                base_style.add_modifier(selection_modifier)
            } else if is_selected && col_idx == 0 {
                // First column already has marker, just make it bold
                base_style.add_modifier(Modifier::BOLD)
            } else {
                base_style
            };

            spans.push(Span::styled(text, style));
        }

        let row = Line::from(spans);
        let paragraph = Paragraph::new(row).alignment(Alignment::Left);
        paragraph.render(area, buf);
    }

    fn render_keybindings(&self, area: Rect, buf: &mut Buffer) {
        let bindings = Paragraph::new(Line::from(vec![
            Span::styled("↑↓", Style::default().fg(self.theme.accent())),
            Span::styled(": Select", Style::default().fg(self.theme.muted())),
            Span::raw("  "),
            Span::styled("Enter", Style::default().fg(self.theme.accent())),
            Span::styled(": Details", Style::default().fg(self.theme.muted())),
            Span::raw("  "),
            Span::styled("d/w/m", Style::default().fg(self.theme.accent())),
            Span::styled(": View mode", Style::default().fg(self.theme.muted())),
            Span::raw("  "),
            Span::styled("Tab", Style::default().fg(self.theme.accent())),
            Span::styled(": Switch view", Style::default().fg(self.theme.muted())),
            Span::raw("  "),
            Span::styled("?", Style::default().fg(self.theme.accent())),
            Span::styled(": Help", Style::default().fg(self.theme.muted())),
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
            total_thinking_tokens: 0,
            total_cost_usd: cost,
            models: HashMap::new(),
        }
    }

    #[test]
    fn test_daily_data_empty() {
        let data = DailyData::from_daily_summaries(vec![]);
        assert!(data.daily_summaries.is_empty());
        assert_eq!(data.daily_max_tokens, 0);
    }

    #[test]
    fn test_daily_data_sorted_asc() {
        // Input is ascending (as from Aggregator::daily)
        let summaries = vec![
            make_daily_summary(2024, 1, 10, 100, 50, 10, 5, 0.01),
            make_daily_summary(2024, 1, 15, 200, 100, 20, 10, 0.02),
            make_daily_summary(2024, 1, 20, 300, 150, 30, 15, 0.03),
        ];

        let data = DailyData::from_daily_summaries(summaries);

        assert_eq!(data.daily_summaries.len(), 3);
        // Should remain ascending (oldest first)
        assert_eq!(data.daily_summaries[0].date.to_string(), "2024-01-10");
        assert_eq!(data.daily_summaries[1].date.to_string(), "2024-01-15");
        assert_eq!(data.daily_summaries[2].date.to_string(), "2024-01-20");
    }

    #[test]
    fn test_daily_data_max_tokens() {
        let summaries = vec![
            make_daily_summary(2024, 1, 10, 100, 50, 10, 5, 0.01), // total: 165
            make_daily_summary(2024, 1, 15, 200, 100, 20, 10, 0.02), // total: 330
            make_daily_summary(2024, 1, 20, 300, 150, 30, 15, 0.03), // total: 495
        ];

        let data = DailyData::from_daily_summaries(summaries);

        assert_eq!(data.daily_max_tokens, 495);
    }

    // ========== DailyView scroll tests ==========

    #[test]
    fn test_daily_view_scroll_bounds_empty() {
        let data = DailyData::from_daily_summaries(vec![]);
        assert_eq!(DailyView::max_scroll_offset(&data, DailyViewMode::Daily), 0);
    }

    #[test]
    fn test_daily_view_scroll_bounds_less_than_visible() {
        let summaries = vec![
            make_daily_summary(2024, 1, 10, 100, 50, 10, 5, 0.01),
            make_daily_summary(2024, 1, 15, 200, 100, 20, 10, 0.02),
        ];
        let data = DailyData::from_daily_summaries(summaries);
        // 2 items < VISIBLE_ROWS (15), so max offset is 0
        assert_eq!(DailyView::max_scroll_offset(&data, DailyViewMode::Daily), 0);
    }

    #[test]
    fn test_daily_view_scroll_bounds_more_than_visible() {
        let summaries: Vec<DailySummary> = (1..=20)
            .map(|d| make_daily_summary(2024, 1, d, 100, 50, 10, 5, 0.01))
            .collect();
        let data = DailyData::from_daily_summaries(summaries);
        // 20 items, VISIBLE_ROWS = 15, so max offset = 5
        assert_eq!(DailyView::max_scroll_offset(&data, DailyViewMode::Daily), 5);
    }

    // ========== DailyData multi-mode tests ==========

    #[test]
    fn test_daily_data_has_all_modes() {
        // 3 days across 2 weeks, 1 month
        let summaries = vec![
            make_daily_summary(2025, 1, 13, 100, 50, 0, 0, 0.01), // Mon, week of Jan 12
            make_daily_summary(2025, 1, 15, 200, 100, 0, 0, 0.02), // Wed, week of Jan 12
            make_daily_summary(2025, 1, 20, 300, 150, 0, 0, 0.03), // Mon, week of Jan 19
        ];
        let data = DailyData::from_daily_summaries(summaries);

        assert_eq!(data.daily_summaries.len(), 3);
        assert_eq!(data.weekly_summaries.len(), 2);
        assert_eq!(data.monthly_summaries.len(), 1);
    }

    #[test]
    fn test_for_mode_returns_correct_data() {
        let summaries = vec![
            make_daily_summary(2025, 1, 13, 100, 50, 0, 0, 0.01),
            make_daily_summary(2025, 1, 20, 200, 100, 0, 0, 0.02),
            make_daily_summary(2025, 2, 3, 300, 150, 0, 0, 0.03),
        ];
        let data = DailyData::from_daily_summaries(summaries);

        let (daily, _) = data.for_mode(DailyViewMode::Daily);
        assert_eq!(daily.len(), 3);

        let (weekly, _) = data.for_mode(DailyViewMode::Weekly);
        assert_eq!(weekly.len(), 3); // 3 different weeks

        let (monthly, _) = data.for_mode(DailyViewMode::Monthly);
        assert_eq!(monthly.len(), 2); // Jan and Feb
    }

    #[test]
    fn test_view_mode_labels() {
        assert_eq!(DailyViewMode::Daily.label(), "Daily");
        assert_eq!(DailyViewMode::Weekly.label(), "Weekly");
        assert_eq!(DailyViewMode::Monthly.label(), "Monthly");
    }

    #[test]
    fn test_view_mode_date_column_labels() {
        assert_eq!(DailyViewMode::Daily.date_column_label(), "Date");
        assert_eq!(DailyViewMode::Weekly.date_column_label(), "Week");
        assert_eq!(DailyViewMode::Monthly.date_column_label(), "Month");
    }

    // ========== Responsive column tests ==========
    // Hide order: Input → Output → Cache → Usage (keeps Usage visible longest)
    // Full: 141, -Input: 123, -Output: 105, -Cache: 87, -Usage: 69

    #[test]
    fn test_visible_columns_full_width() {
        // >= 141: all 8 columns visible
        let cols = visible_columns(141);
        assert_eq!(cols.len(), 8);
        assert_eq!(cols, vec![0, 1, 2, 3, 4, 5, 6, 7]);
    }

    #[test]
    fn test_visible_columns_hide_input() {
        // 123..140: 7 columns (Input hidden first)
        let cols = visible_columns(123);
        assert_eq!(cols.len(), 7);
        assert!(!cols.contains(&COL_INPUT));
        assert!(cols.contains(&COL_USAGE)); // Usage still visible
    }

    #[test]
    fn test_visible_columns_hide_input_and_output() {
        // 105..122: 6 columns (Input + Output hidden)
        let cols = visible_columns(105);
        assert_eq!(cols.len(), 6);
        assert!(!cols.contains(&COL_INPUT));
        assert!(!cols.contains(&COL_OUTPUT));
        assert!(cols.contains(&COL_USAGE)); // Usage still visible
    }

    #[test]
    fn test_visible_columns_hide_three() {
        // 87..104: 5 columns (Input + Output + Cache hidden)
        let cols = visible_columns(87);
        assert_eq!(cols.len(), 5);
        assert!(!cols.contains(&COL_INPUT));
        assert!(!cols.contains(&COL_OUTPUT));
        assert!(!cols.contains(&COL_CACHE));
        assert!(cols.contains(&COL_USAGE)); // Usage still visible
    }

    #[test]
    fn test_visible_columns_minimum() {
        // < 87: 4 columns (Date + Model + Total + Cost)
        let cols = visible_columns(69);
        assert_eq!(cols.len(), 4);
        assert_eq!(cols, vec![COL_DATE, COL_MODEL, COL_TOTAL, COL_COST]);
    }

    #[test]
    fn test_table_width_for_all_columns() {
        let all: Vec<usize> = (0..8).collect();
        assert_eq!(table_width_for(&all), 141);
    }

    #[test]
    fn test_table_width_for_minimum_columns() {
        let min = vec![COL_DATE, COL_MODEL, COL_TOTAL, COL_COST];
        assert_eq!(table_width_for(&min), 69);
    }

    #[test]
    fn test_visible_columns_wide_terminal() {
        // Very wide terminal should still show all 8
        let cols = visible_columns(200);
        assert_eq!(cols.len(), 8);
    }
}
