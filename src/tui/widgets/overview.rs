//! Overview layout widget

use chrono::NaiveDate;
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

/// Data for the overview display
#[derive(Debug, Clone)]
pub struct OverviewData {
    pub total: TotalSummary,
    pub daily_tokens: Vec<(NaiveDate, u64)>,
}

/// Maximum content width for Overview (keeps layout clean on wide terminals)
/// 52 weeks * 3-char cells + 4 label = 160, so 170 gives some padding
const MAX_CONTENT_WIDTH: u16 = 170;

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
        // - Heatmap (18: 15 rows grid + month labels + blank + legend) + Separator (1) + Keybindings (1) = 28 total
        let chunks = Layout::vertical([
            Constraint::Length(1),  // Top padding
            Constraint::Length(1),  // Tabs
            Constraint::Length(1),  // Separator
            Constraint::Length(3),  // Hero stat
            Constraint::Length(1),  // Sub-stats (Cost only)
            Constraint::Length(1),  // Blank
            Constraint::Length(18), // Heatmap (15 rows grid) + month labels + blank + legend
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

        // Render sub-stats (Cost only)
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
        // Include all token types: input + output + cache_read + cache_creation
        let total_tokens = self.data.total.total_input_tokens
            + self.data.total.total_output_tokens
            + self.data.total.total_cache_read_tokens
            + self.data.total.total_cache_creation_tokens;
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
        let cost_str = format!("Cost: ${:.2}", self.data.total.total_cost_usd);

        let stats = Paragraph::new(Line::from(vec![Span::styled(
            cost_str,
            Style::default().fg(Color::Magenta),
        )]))
        .alignment(Alignment::Center);

        stats.render(area, buf);
    }

    fn render_heatmap_section(&self, area: Rect, buf: &mut Buffer) {
        // Layout constants for heatmap section
        const HEATMAP_GRID_ROWS: u16 = 15; // 7 weekdays * 2 (content + separator) + 1 top border
        const MONTH_LABEL_ROWS: u16 = 1;
        const BLANK_ROWS: u16 = 1;
        const LEGEND_ROWS: u16 = 1;
        const LEGEND_Y_OFFSET: u16 = HEATMAP_GRID_ROWS + MONTH_LABEL_ROWS + BLANK_ROWS;
        const REQUIRED_HEIGHT: u16 = LEGEND_Y_OFFSET + LEGEND_ROWS;

        let weeks = Heatmap::weeks_for_width(area.width);
        let heatmap = Heatmap::new(&self.data.daily_tokens, self.today, weeks);
        heatmap.render(area, buf);

        // Legend on last row - aligned to heatmap grid right edge (with 1 blank row gap)
        if area.height >= REQUIRED_HEIGHT {
            // Calculate actual heatmap width: label (4) + 1 (left border) + weeks * cell_width (3)
            let heatmap_width = 4 + 1 + (weeks as u16 * 3);
            let legend_area = Rect {
                x: area.x,
                y: area.y + LEGEND_Y_OFFSET,
                width: heatmap_width.min(area.width),
                height: LEGEND_ROWS,
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
}
