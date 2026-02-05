//! Model breakdown popup widget - displays per-model usage details for a selected date

use ratatui::{
    buffer::Buffer,
    layout::{Alignment, Constraint, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph, Widget},
};

use crate::services::display_name;
use crate::tui::theme::Theme;
use crate::types::ModelUsage;

use super::overview::format_number;

/// Width and height of the model breakdown popup
const POPUP_WIDTH: u16 = 50;
const POPUP_MIN_HEIGHT: u16 = 8;
const POPUP_MAX_HEIGHT: u16 = 20;

/// State for model breakdown popup
#[derive(Debug, Clone)]
pub struct ModelBreakdownState {
    /// Date label to display in title (e.g., "2026-02-05")
    pub date_label: String,
    /// Models sorted by cost descending
    pub models: Vec<(String, ModelUsage)>,
}

impl ModelBreakdownState {
    /// Create a new state from date label and model map
    pub fn new(date_label: String, models: Vec<(String, ModelUsage)>) -> Self {
        // Sort by cost descending
        let mut models = models;
        models.sort_by(|a, b| {
            b.1.cost_usd
                .partial_cmp(&a.1.cost_usd)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        Self { date_label, models }
    }
}

/// Model breakdown popup overlay
pub struct ModelBreakdownPopup<'a> {
    state: &'a ModelBreakdownState,
    theme: Theme,
}

impl<'a> ModelBreakdownPopup<'a> {
    pub fn new(state: &'a ModelBreakdownState, theme: Theme) -> Self {
        Self { state, theme }
    }

    /// Calculate centered popup area with dynamic height based on model count
    pub fn centered_area(area: Rect, model_count: usize) -> Rect {
        // Height = header (3) + models + footer (2)
        let content_height = 5 + model_count as u16;
        let height = content_height.clamp(POPUP_MIN_HEIGHT, POPUP_MAX_HEIGHT);

        let x = area.x + (area.width.saturating_sub(POPUP_WIDTH)) / 2;
        let y = area.y + (area.height.saturating_sub(height)) / 2;
        Rect {
            x,
            y,
            width: POPUP_WIDTH.min(area.width),
            height: height.min(area.height),
        }
    }
}

impl Widget for ModelBreakdownPopup<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        // Clear the area first (for overlay effect)
        Clear.render(area, buf);

        // Create block with border and date title
        let title = format!(" {} ", self.state.date_label);
        let block = Block::default()
            .title(title)
            .title_alignment(Alignment::Center)
            .borders(Borders::ALL)
            .border_style(Style::default().fg(self.theme.date()));

        let inner = block.inner(area);
        block.render(area, buf);

        // Calculate visible rows (minus header and footer)
        let available_rows = inner.height.saturating_sub(4) as usize;
        let models_to_show = self.state.models.len().min(available_rows);

        // Build layout
        let mut constraints = vec![
            Constraint::Length(1), // Header
            Constraint::Length(1), // Separator
        ];
        for _ in 0..models_to_show {
            constraints.push(Constraint::Length(1));
        }
        constraints.push(Constraint::Length(1)); // Padding
        constraints.push(Constraint::Length(1)); // Footer

        let chunks = Layout::vertical(constraints).split(inner);

        // Header
        let header_style = Style::default()
            .fg(self.theme.text())
            .add_modifier(Modifier::BOLD);
        let header = Line::from(vec![
            Span::styled(format!("{:<22}", "Model"), header_style),
            Span::styled(format!("{:>12}", "Total"), header_style),
            Span::styled(format!("{:>12}", "Cost"), header_style),
        ]);
        Paragraph::new(header)
            .alignment(Alignment::Left)
            .render(chunks[0], buf);

        // Separator
        let sep = "─".repeat(inner.width as usize);
        buf.set_string(
            inner.x,
            chunks[1].y,
            &sep,
            Style::default().fg(self.theme.muted()),
        );

        // Model rows
        for (i, (model_name, usage)) in self.state.models.iter().take(models_to_show).enumerate() {
            let chunk_idx = i + 2;
            let display = display_name(model_name);
            let truncated = if display.chars().count() > 20 {
                format!("{}…", display.chars().take(19).collect::<String>())
            } else {
                display
            };

            let total_tokens = usage.input_tokens
                + usage.output_tokens
                + usage.cache_read_tokens
                + usage.cache_creation_tokens;

            let row = Line::from(vec![
                Span::styled(
                    format!("{:<22}", truncated),
                    Style::default().fg(self.theme.accent()),
                ),
                Span::styled(
                    format!("{:>12}", format_number(total_tokens)),
                    Style::default().fg(self.theme.text()),
                ),
                Span::styled(
                    format!("{:>12}", format!("${:.2}", usage.cost_usd)),
                    Style::default().fg(self.theme.cost()),
                ),
            ]);
            Paragraph::new(row)
                .alignment(Alignment::Left)
                .render(chunks[chunk_idx], buf);
        }

        // Footer hint
        let footer_idx = chunks.len() - 1;
        let footer = Line::from(Span::styled(
            "Press Esc to close",
            Style::default().fg(self.theme.muted()),
        ));
        Paragraph::new(footer)
            .alignment(Alignment::Center)
            .render(chunks[footer_idx], buf);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_usage(input: u64, output: u64, cost: f64) -> ModelUsage {
        ModelUsage {
            input_tokens: input,
            output_tokens: output,
            cache_read_tokens: 0,
            cache_creation_tokens: 0,
            cost_usd: cost,
            count: 1,
        }
    }

    #[test]
    fn test_state_sorts_by_cost_descending() {
        let models = vec![
            ("cheap".to_string(), make_usage(100, 50, 0.50)),
            ("expensive".to_string(), make_usage(200, 100, 2.00)),
            ("mid".to_string(), make_usage(150, 75, 1.00)),
        ];
        let state = ModelBreakdownState::new("2026-02-05".to_string(), models);

        assert_eq!(state.models[0].0, "expensive");
        assert_eq!(state.models[1].0, "mid");
        assert_eq!(state.models[2].0, "cheap");
    }

    #[test]
    fn test_state_empty_models() {
        let state = ModelBreakdownState::new("2026-02-05".to_string(), vec![]);
        assert!(state.models.is_empty());
        assert_eq!(state.date_label, "2026-02-05");
    }

    #[test]
    fn test_centered_area_basic() {
        let area = Rect::new(0, 0, 100, 50);
        let popup_area = ModelBreakdownPopup::centered_area(area, 3);

        assert_eq!(popup_area.width, POPUP_WIDTH);
        assert!(popup_area.height >= POPUP_MIN_HEIGHT);
        assert!(popup_area.height <= POPUP_MAX_HEIGHT);
        // Should be centered
        assert_eq!(popup_area.x, (100 - POPUP_WIDTH) / 2);
    }

    #[test]
    fn test_centered_area_many_models() {
        let area = Rect::new(0, 0, 100, 50);
        let popup_area = ModelBreakdownPopup::centered_area(area, 20);

        // Should cap at max height
        assert!(popup_area.height <= POPUP_MAX_HEIGHT);
    }

    #[test]
    fn test_centered_area_small_terminal() {
        let area = Rect::new(0, 0, 40, 10);
        let popup_area = ModelBreakdownPopup::centered_area(area, 5);

        // Should fit within terminal bounds
        assert!(popup_area.width <= area.width);
        assert!(popup_area.height <= area.height);
    }

    #[test]
    fn test_popup_renders_without_panic() {
        let models = vec![
            (
                "claude-sonnet-4-20250514".to_string(),
                make_usage(1000, 500, 1.50),
            ),
            (
                "claude-opus-4-20250514".to_string(),
                make_usage(500, 250, 5.00),
            ),
        ];
        let state = ModelBreakdownState::new("2026-02-05".to_string(), models);

        let area = Rect::new(0, 0, 80, 30);
        let popup_area = ModelBreakdownPopup::centered_area(area, state.models.len());
        let mut buf = Buffer::empty(area);
        ModelBreakdownPopup::new(&state, Theme::Dark).render(popup_area, &mut buf);

        // Verify content rendered
        let content: String = buf.content().iter().map(|c| c.symbol()).collect();
        assert!(content.contains("2026-02-05"));
        assert!(content.contains("Model"));
        assert!(content.contains("Total"));
        assert!(content.contains("Cost"));
        assert!(content.contains("Esc"));
    }

    #[test]
    fn test_popup_renders_display_names() {
        let models = vec![(
            "claude-opus-4-5-20251101".to_string(),
            make_usage(1000, 500, 15.00),
        )];
        let state = ModelBreakdownState::new("2026-02-05".to_string(), models);

        let area = Rect::new(0, 0, 80, 30);
        let popup_area = ModelBreakdownPopup::centered_area(area, state.models.len());
        let mut buf = Buffer::empty(area);
        ModelBreakdownPopup::new(&state, Theme::Dark).render(popup_area, &mut buf);

        let content: String = buf.content().iter().map(|c| c.symbol()).collect();
        // display_name converts claude-opus-4-5-20251101 to "Opus 4.5"
        assert!(content.contains("Opus 4.5"));
    }

    #[test]
    fn test_popup_truncates_long_model_names() {
        let models = vec![(
            "super-long-model-name-that-exceeds-twenty-chars".to_string(),
            make_usage(100, 50, 1.00),
        )];
        let state = ModelBreakdownState::new("2026-02-05".to_string(), models);

        let area = Rect::new(0, 0, 80, 30);
        let popup_area = ModelBreakdownPopup::centered_area(area, state.models.len());
        let mut buf = Buffer::empty(area);
        ModelBreakdownPopup::new(&state, Theme::Dark).render(popup_area, &mut buf);

        let content: String = buf.content().iter().map(|c| c.symbol()).collect();
        // Should contain truncation marker
        assert!(content.contains('…'));
    }
}
