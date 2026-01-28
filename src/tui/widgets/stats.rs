//! Stats view widget - displays usage statistics in a card grid

use ratatui::{
    buffer::Buffer,
    layout::{Alignment, Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Widget},
};

use super::overview::format_number;
use super::tabs::{Tab, TabBar};
use crate::types::StatsData;

/// Maximum content width for Stats view (consistent with other views)
const MAX_CONTENT_WIDTH: u16 = 170;

/// Card dimensions
const CARD_WIDTH: u16 = 28;
const CARD_HEIGHT: u16 = 5;

/// Fixed number of columns for balanced 2x3 grid
const FIXED_COLS: usize = 3;

/// Calculate number of cards per row based on available width (max 3 for balanced grid)
fn cards_per_row(width: u16) -> usize {
    let usable_width = width.saturating_sub(4); // padding
    let cards = (usable_width / (CARD_WIDTH + 2)) as usize; // +2 for spacing
    cards.clamp(1, FIXED_COLS)
}

/// Stats view widget
pub struct StatsView<'a> {
    data: &'a StatsData,
    selected_tab: Tab,
}

impl<'a> StatsView<'a> {
    pub fn new(data: &'a StatsData) -> Self {
        Self {
            data,
            selected_tab: Tab::Stats,
        }
    }

    pub fn with_tab(mut self, tab: Tab) -> Self {
        self.selected_tab = tab;
        self
    }
}

impl Widget for StatsView<'_> {
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

        // Calculate grid layout
        let cols = cards_per_row(centered_area.width);
        let rows = 6_usize.div_ceil(cols); // 6 cards total
        let grid_height = (rows as u16) * (CARD_HEIGHT + 1); // +1 for spacing

        let chunks = Layout::vertical([
            Constraint::Length(1),           // Top padding
            Constraint::Length(1),           // Tabs
            Constraint::Length(1),           // Separator
            Constraint::Length(1),           // Title
            Constraint::Length(1),           // Blank
            Constraint::Length(grid_height), // Card grid
            Constraint::Length(1),           // Separator
            Constraint::Length(1),           // Keybindings
            Constraint::Min(0),              // Remaining space
        ])
        .split(centered_area);

        // Render tabs
        self.render_tabs(chunks[1], buf);

        // Render separator
        self.render_separator(chunks[2], buf);

        // Render title
        self.render_title(chunks[3], buf);

        // Render card grid
        self.render_card_grid(chunks[5], buf, cols);

        // Render separator
        self.render_separator(chunks[6], buf);

        // Render keybindings
        self.render_keybindings(chunks[7], buf);
    }
}

impl StatsView<'_> {
    fn render_tabs(&self, area: Rect, buf: &mut Buffer) {
        let tab_bar = TabBar::new(self.selected_tab);
        tab_bar.render(area, buf);
    }

    fn render_separator(&self, area: Rect, buf: &mut Buffer) {
        let line = "─".repeat(area.width as usize);
        buf.set_string(area.x, area.y, &line, Style::default().fg(Color::DarkGray));
    }

    fn render_title(&self, area: Rect, buf: &mut Buffer) {
        let title = Paragraph::new(Line::from(Span::styled(
            "Usage Statistics",
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        )))
        .alignment(Alignment::Center);
        title.render(area, buf);
    }

    fn render_card_grid(&self, area: Rect, buf: &mut Buffer, cols: usize) {
        let cards = self.build_cards();

        // Calculate grid positioning
        let total_cards_width = (cols as u16) * CARD_WIDTH + ((cols - 1) as u16) * 2; // 2 = spacing
        let start_x = area.x + (area.width.saturating_sub(total_cards_width)) / 2;

        for (i, card) in cards.iter().enumerate() {
            let row = i / cols;
            let col = i % cols;

            let card_x = start_x + (col as u16) * (CARD_WIDTH + 2);
            let card_y = area.y + (row as u16) * (CARD_HEIGHT + 1);

            // Skip if card is outside area
            if card_y + CARD_HEIGHT > area.y + area.height {
                continue;
            }

            let card_area = Rect {
                x: card_x,
                y: card_y,
                width: CARD_WIDTH,
                height: CARD_HEIGHT,
            };

            self.render_card(card_area, buf, card);
        }
    }

    fn build_cards(&self) -> Vec<StatCard> {
        vec![
            StatCard {
                title: "Total Tokens".to_string(),
                value: format_number(self.data.total_tokens),
                value_color: Color::Cyan,
                border_color: Color::Cyan,
            },
            StatCard {
                title: "Daily Average".to_string(),
                value: format_number(self.data.daily_avg_tokens),
                value_color: Color::Blue,
                border_color: Color::Blue,
            },
            StatCard {
                title: "Peak Day".to_string(),
                value: self
                    .data
                    .peak_day
                    .map(|(date, tokens)| {
                        format!("{} ({})", date.format("%m/%d"), format_number(tokens))
                    })
                    .unwrap_or_else(|| "N/A".to_string()),
                value_color: Color::Yellow,
                border_color: Color::Yellow,
            },
            StatCard {
                title: "Total Cost".to_string(),
                value: format!("${:.2}", self.data.total_cost),
                value_color: Color::LightRed,
                border_color: Color::Red,
            },
            StatCard {
                title: "Daily Avg Cost".to_string(),
                value: format!("${:.2}", self.data.daily_avg_cost),
                value_color: Color::Magenta,
                border_color: Color::Magenta,
            },
            StatCard {
                title: "Active Days".to_string(),
                value: self.data.active_days.to_string(),
                value_color: Color::Green,
                border_color: Color::Green,
            },
        ]
    }

    fn render_card(&self, area: Rect, buf: &mut Buffer, card: &StatCard) {
        // Draw card border with card-specific color
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(card.border_color));
        block.render(area, buf);

        // Render title (centered, line 1 inside border) with matching border color
        if area.height > 2 {
            let title_y = area.y + 1;
            let title = &card.title;
            let title_x = area.x + (area.width.saturating_sub(title.len() as u16)) / 2;
            buf.set_string(
                title_x,
                title_y,
                title,
                Style::default().fg(card.border_color),
            );
        }

        // Render value (centered, line 2-3 inside border)
        if area.height > 3 {
            let value_y = area.y + 3;
            let value = &card.value;
            let value_x = area.x + (area.width.saturating_sub(value.len() as u16)) / 2;
            buf.set_string(
                value_x,
                value_y,
                value,
                Style::default()
                    .fg(card.value_color)
                    .add_modifier(Modifier::BOLD),
            );
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

/// Internal card representation
struct StatCard {
    title: String,
    value: String,
    value_color: Color,
    border_color: Color,
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    #[test]
    fn test_stats_view_builds_six_cards() {
        let data = StatsData {
            total_tokens: 1000,
            daily_avg_tokens: 500,
            peak_day: Some((NaiveDate::from_ymd_opt(2024, 1, 15).unwrap(), 1000)),
            total_cost: 1.50,
            daily_avg_cost: 0.75,
            active_days: 2,
        };
        let view = StatsView::new(&data);
        let cards = view.build_cards();

        assert_eq!(cards.len(), 6);
    }

    #[test]
    fn test_cards_per_row_narrow() {
        // Width 60 should fit 1-2 cards
        let cols = cards_per_row(60);
        assert!((1..=2).contains(&cols));
    }

    #[test]
    fn test_cards_per_row_wide() {
        // Width 170 should fit 5 cards: (170-4) / 30 = 5
        let cols = cards_per_row(170);
        assert!(cols >= 3);
    }

    #[test]
    fn test_cards_per_row_minimum() {
        // Even with very narrow width, should return at least 1
        assert_eq!(cards_per_row(20), 1);
        assert_eq!(cards_per_row(10), 1);
    }
}
