//! Source detail view - displays per-source daily breakdown

use ratatui::{
    buffer::Buffer,
    layout::{Alignment, Constraint, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Paragraph, Widget},
};

use super::daily::{DailyData, DailyView, DailyViewMode};
use super::overview::format_number;
use crate::tui::theme::Theme;
use crate::types::StatsData;

/// Maximum content width (consistent with other views)
const MAX_CONTENT_WIDTH: u16 = 170;

/// Source detail view combining daily table for a single source
pub struct SourceDetailView<'a> {
    source_name: &'a str,
    daily_data: &'a DailyData,
    stats_data: &'a StatsData,
    scroll_offset: usize,
    view_mode: DailyViewMode,
    selected_index: Option<usize>,
    theme: Theme,
}

impl<'a> SourceDetailView<'a> {
    pub fn new(
        source_name: &'a str,
        daily_data: &'a DailyData,
        stats_data: &'a StatsData,
        scroll_offset: usize,
        view_mode: DailyViewMode,
        selected_index: Option<usize>,
        theme: Theme,
    ) -> Self {
        Self {
            source_name,
            daily_data,
            stats_data,
            scroll_offset,
            view_mode,
            selected_index,
            theme,
        }
    }
}

impl Widget for SourceDetailView<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let content_width = area.width.min(MAX_CONTENT_WIDTH);
        let x_offset = (area.width.saturating_sub(content_width)) / 2;
        let centered_area = Rect {
            x: area.x + x_offset,
            y: area.y,
            width: content_width,
            height: area.height,
        };

        let chunks = Layout::vertical([
            Constraint::Length(1), // 0: Top padding
            Constraint::Length(1), // 1: Source header
            Constraint::Length(1), // 2: Stats inline
            Constraint::Length(1), // 3: Separator
            Constraint::Length(1), // 4: Mode indicator
            Constraint::Length(1), // 5: Daily table header
            Constraint::Fill(1),   // 6: Daily rows (fill remaining)
            Constraint::Length(1), // 7: Separator
            Constraint::Length(1), // 8: Keybindings
        ])
        .split(centered_area);

        self.render_source_header(chunks[1], buf);
        self.render_stats_inline(chunks[2], buf);
        self.render_separator(chunks[3], buf);
        self.render_mode_indicator(chunks[4], buf);

        // Render daily table (header + rows)
        let daily_view = DailyView::new(
            self.daily_data,
            self.scroll_offset,
            self.view_mode,
            self.theme,
            self.stats_data.daily_avg_cost,
        )
        .with_selected_index(self.selected_index);

        daily_view.render_header(chunks[5], buf, &daily_view_visible_columns(chunks[5].width));
        daily_view.render_daily_rows(chunks[6], buf, &daily_view_visible_columns(chunks[6].width));

        self.render_separator(chunks[7], buf);
        self.render_keybindings(chunks[8], buf);
    }
}

/// Get visible columns for the daily table at a given width
fn daily_view_visible_columns(width: u16) -> Vec<usize> {
    super::daily::visible_columns(width)
}

impl SourceDetailView<'_> {
    fn render_source_header(&self, area: Rect, buf: &mut Buffer) {
        let total_tokens = self.stats_data.total_tokens;
        let total_cost = self.stats_data.total_cost;

        let header = Paragraph::new(Line::from(vec![
            Span::styled(
                self.source_name.to_string(),
                Style::default()
                    .fg(self.theme.accent())
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!("  {}  ", format_number(total_tokens)),
                Style::default().fg(self.theme.text()),
            ),
            Span::styled(
                format!("${:.2}", total_cost),
                Style::default().fg(self.theme.cost()),
            ),
        ]))
        .alignment(Alignment::Center);

        header.render(area, buf);
    }

    fn render_stats_inline(&self, area: Rect, buf: &mut Buffer) {
        let active_str = format!("Active: {}d", self.stats_data.active_days);
        let avg_str = format!(
            "Avg: {}/day",
            format_number(self.stats_data.daily_avg_tokens)
        );
        let peak_str = self
            .stats_data
            .peak_day
            .map(|(d, _)| format!("Peak: {}", d.format("%b %d")))
            .unwrap_or_default();
        let avg_cost_str = format!("Avg Cost: ${:.2}/day", self.stats_data.daily_avg_cost);

        let stats = Paragraph::new(Line::from(vec![
            Span::styled(&active_str, Style::default().fg(self.theme.date())),
            Span::raw("  "),
            Span::styled(&avg_str, Style::default().fg(self.theme.date())),
            Span::raw("  "),
            Span::styled(&peak_str, Style::default().fg(self.theme.date())),
            Span::raw("  "),
            Span::styled(&avg_cost_str, Style::default().fg(self.theme.date())),
        ]))
        .alignment(Alignment::Center);

        stats.render(area, buf);
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
            Span::styled("Esc", Style::default().fg(self.theme.accent())),
            Span::styled(": Back", Style::default().fg(self.theme.muted())),
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
    use crate::types::DailySummary;
    use chrono::NaiveDate;
    use std::collections::HashMap;

    fn make_stats_data() -> StatsData {
        StatsData {
            total_tokens: 1_500_000,
            daily_avg_tokens: 500_000,
            peak_day: Some((NaiveDate::from_ymd_opt(2024, 1, 15).unwrap(), 800_000)),
            total_cost: 12.50,
            daily_avg_cost: 4.17,
            active_days: 3,
        }
    }

    fn make_daily_data() -> DailyData {
        let summaries = vec![DailySummary {
            date: NaiveDate::from_ymd_opt(2024, 1, 15).unwrap(),
            total_input_tokens: 500_000,
            total_output_tokens: 200_000,
            total_cache_read_tokens: 100_000,
            total_cache_creation_tokens: 50_000,
            total_reasoning_tokens: 0,
            total_cache_creation_5m_tokens: 0,
            total_cache_creation_1h_tokens: 0,
            total_web_search_requests: 0,
            total_cost_usd: 12.50,
            models: HashMap::new(),
            projects: HashMap::new(),
        }];
        DailyData::from_daily_summaries(summaries)
    }

    #[test]
    fn test_source_detail_renders_without_panic() {
        let stats = make_stats_data();
        let daily = make_daily_data();
        let view = SourceDetailView::new(
            "claude-code",
            &daily,
            &stats,
            0,
            DailyViewMode::Daily,
            None,
            Theme::Dark,
        );
        let area = Rect::new(0, 0, 120, 20);
        let mut buf = Buffer::empty(area);
        view.render(area, &mut buf);

        let content = buffer_to_string(&buf);
        assert!(content.contains("claude-code"));
    }

    #[test]
    fn test_source_detail_renders_stats_inline() {
        let stats = make_stats_data();
        let daily = make_daily_data();
        let view = SourceDetailView::new(
            "test-source",
            &daily,
            &stats,
            0,
            DailyViewMode::Daily,
            None,
            Theme::Dark,
        );
        let area = Rect::new(0, 0, 120, 20);
        let mut buf = Buffer::empty(area);
        view.render(area, &mut buf);

        let content = buffer_to_string(&buf);
        assert!(content.contains("Active: 3d"));
        assert!(content.contains("$12.50"));
    }

    #[test]
    fn test_source_detail_mode_indicator_daily() {
        let stats = make_stats_data();
        let daily = make_daily_data();
        let view = SourceDetailView::new(
            "src",
            &daily,
            &stats,
            0,
            DailyViewMode::Daily,
            None,
            Theme::Dark,
        );
        let area = Rect::new(0, 0, 120, 20);
        let mut buf = Buffer::empty(area);
        view.render(area, &mut buf);

        let content = buffer_to_string(&buf);
        assert!(content.contains("d:Daily"));
        assert!(content.contains("w:Weekly"));
        assert!(content.contains("m:Monthly"));
    }

    #[test]
    fn test_source_detail_keybindings_rendered() {
        let stats = make_stats_data();
        let daily = make_daily_data();
        let view = SourceDetailView::new(
            "src",
            &daily,
            &stats,
            0,
            DailyViewMode::Daily,
            None,
            Theme::Dark,
        );
        let area = Rect::new(0, 0, 120, 20);
        let mut buf = Buffer::empty(area);
        view.render(area, &mut buf);

        let content = buffer_to_string(&buf);
        assert!(content.contains(": Select"));
        assert!(content.contains("Esc"));
        assert!(content.contains(": Help"));
    }

    #[test]
    fn test_source_detail_narrow_terminal() {
        let stats = make_stats_data();
        let daily = make_daily_data();
        let view = SourceDetailView::new(
            "src",
            &daily,
            &stats,
            0,
            DailyViewMode::Daily,
            None,
            Theme::Dark,
        );
        let area = Rect::new(0, 0, 60, 15);
        let mut buf = Buffer::empty(area);
        view.render(area, &mut buf);
        // Should not panic even on narrow terminal
    }

    #[test]
    fn test_source_detail_with_selection() {
        let stats = make_stats_data();
        let daily = make_daily_data();
        let view = SourceDetailView::new(
            "src",
            &daily,
            &stats,
            0,
            DailyViewMode::Daily,
            Some(0),
            Theme::Dark,
        );
        let area = Rect::new(0, 0, 120, 20);
        let mut buf = Buffer::empty(area);
        view.render(area, &mut buf);
        // Should render with selected row without panic
    }

    #[test]
    fn test_source_detail_weekly_mode() {
        let stats = make_stats_data();
        let daily = make_daily_data();
        let view = SourceDetailView::new(
            "src",
            &daily,
            &stats,
            0,
            DailyViewMode::Weekly,
            None,
            Theme::Dark,
        );
        let area = Rect::new(0, 0, 120, 20);
        let mut buf = Buffer::empty(area);
        view.render(area, &mut buf);

        let content = buffer_to_string(&buf);
        assert!(content.contains("w:Weekly"));
    }

    #[test]
    fn test_source_detail_empty_data() {
        let stats = StatsData {
            total_tokens: 0,
            daily_avg_tokens: 0,
            peak_day: None,
            total_cost: 0.0,
            daily_avg_cost: 0.0,
            active_days: 0,
        };
        let daily = DailyData::from_daily_summaries(vec![]);
        let view = SourceDetailView::new(
            "empty-source",
            &daily,
            &stats,
            0,
            DailyViewMode::Daily,
            None,
            Theme::Dark,
        );
        let area = Rect::new(0, 0, 120, 20);
        let mut buf = Buffer::empty(area);
        view.render(area, &mut buf);

        let content = buffer_to_string(&buf);
        assert!(content.contains("empty-source"));
        assert!(content.contains("$0.00"));
    }

    #[test]
    fn test_source_detail_light_theme() {
        let stats = make_stats_data();
        let daily = make_daily_data();
        let view = SourceDetailView::new(
            "src",
            &daily,
            &stats,
            0,
            DailyViewMode::Daily,
            None,
            Theme::Light,
        );
        let area = Rect::new(0, 0, 120, 20);
        let mut buf = Buffer::empty(area);
        view.render(area, &mut buf);
        // Should render with light theme without panic
    }

    #[test]
    fn test_daily_view_visible_columns_delegates() {
        let cols = daily_view_visible_columns(200);
        assert!(!cols.is_empty());

        let narrow_cols = daily_view_visible_columns(40);
        assert!(narrow_cols.len() <= cols.len());
    }

    fn buffer_to_string(buf: &Buffer) -> String {
        let area = buf.area;
        let mut result = String::new();
        for y in area.y..area.y + area.height {
            for x in area.x..area.x + area.width {
                let cell = &buf[(x, y)];
                result.push_str(cell.symbol());
            }
            result.push('\n');
        }
        result
    }
}
