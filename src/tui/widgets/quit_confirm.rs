//! Quit confirmation popup widget

use ratatui::{
    buffer::Buffer,
    layout::{Alignment, Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph, Widget},
};

use crate::tui::theme::Theme;

/// Width and height of the quit confirm popup
const POPUP_WIDTH: u16 = 36;
const POPUP_HEIGHT: u16 = 9;

/// State for quit confirmation dialog
#[derive(Debug, Clone, Default)]
pub struct QuitConfirmState {
    /// 0 = Yes (default), 1 = No
    pub selection: u8,
}

impl QuitConfirmState {
    /// Create a new state with default selection (Yes)
    pub fn new() -> Self {
        Self::default()
    }
}

/// Quit confirmation popup overlay
pub struct QuitConfirmPopup {
    selection: u8,
    theme: Theme,
}

impl QuitConfirmPopup {
    pub fn new(selection: u8, theme: Theme) -> Self {
        Self { selection, theme }
    }

    /// Calculate centered popup area
    pub fn centered_area(area: Rect) -> Rect {
        let x = area.x + (area.width.saturating_sub(POPUP_WIDTH)) / 2;
        let y = area.y + (area.height.saturating_sub(POPUP_HEIGHT)) / 2;
        Rect {
            x,
            y,
            width: POPUP_WIDTH.min(area.width),
            height: POPUP_HEIGHT.min(area.height),
        }
    }
}

impl Widget for QuitConfirmPopup {
    fn render(self, area: Rect, buf: &mut Buffer) {
        Clear.render(area, buf);

        let block = Block::default()
            .title(" Quit? ")
            .title_alignment(Alignment::Center)
            .borders(Borders::ALL)
            .border_style(Style::default().fg(self.theme.date()));

        let inner = block.inner(area);
        block.render(area, buf);

        let chunks = Layout::vertical([
            Constraint::Length(1), // [0] Padding
            Constraint::Length(1), // [1] Question
            Constraint::Length(1), // [2] Padding
            Constraint::Length(1), // [3] Buttons
            Constraint::Length(1), // [4] Padding between buttons and hints
            Constraint::Length(1), // [5] Hint line 1
            Constraint::Length(1), // [6] Hint line 2
        ])
        .split(inner);

        let question_line = Line::from(Span::styled(
            "Are you sure you want to quit?",
            Style::default().fg(self.theme.text()),
        ));
        Paragraph::new(question_line)
            .alignment(Alignment::Center)
            .render(chunks[1], buf);

        let (yes_marker, yes_style) = if self.selection == 0 {
            (
                "▸ ",
                Style::default()
                    .fg(Color::LightRed)
                    .add_modifier(Modifier::BOLD),
            )
        } else {
            ("  ", Style::default().fg(self.theme.muted()))
        };

        let (no_marker, no_style) = if self.selection == 1 {
            (
                "▸ ",
                Style::default()
                    .fg(Color::LightRed)
                    .add_modifier(Modifier::BOLD),
            )
        } else {
            ("  ", Style::default().fg(self.theme.muted()))
        };

        let buttons_line = Line::from(vec![
            Span::styled(yes_marker, yes_style),
            Span::styled("Yes", yes_style),
            Span::styled("       ", Style::default()),
            Span::styled(no_marker, no_style),
            Span::styled("No", no_style),
        ]);
        Paragraph::new(buttons_line)
            .alignment(Alignment::Center)
            .render(chunks[3], buf);

        let hint_line1 = Line::from(vec![
            Span::styled("  ←→", Style::default().fg(self.theme.accent())),
            Span::styled("  Select", Style::default().fg(self.theme.muted())),
        ]);
        Paragraph::new(hint_line1)
            .alignment(Alignment::Center)
            .render(chunks[5], buf);

        let hint_line2 = Line::from(vec![
            Span::styled("Enter", Style::default().fg(self.theme.accent())),
            Span::styled("  Confirm", Style::default().fg(self.theme.muted())),
        ]);
        Paragraph::new(hint_line2)
            .alignment(Alignment::Center)
            .render(chunks[6], buf);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_quit_confirm_default_selection_is_yes() {
        let state = QuitConfirmState::new();
        assert_eq!(state.selection, 0);
    }

    #[test]
    fn test_quit_confirm_state_default_is_yes() {
        let state = QuitConfirmState::default();
        assert_eq!(state.selection, 0);
    }

    #[test]
    fn test_quit_confirm_centered_area() {
        let area = Rect::new(0, 0, 100, 50);
        let popup_area = QuitConfirmPopup::centered_area(area);

        assert_eq!(popup_area.width, POPUP_WIDTH);
        assert_eq!(popup_area.height, POPUP_HEIGHT);
        assert_eq!(popup_area.x, (100 - POPUP_WIDTH) / 2);
        assert_eq!(popup_area.y, (50 - POPUP_HEIGHT) / 2);
    }

    #[test]
    fn test_quit_confirm_small_terminal() {
        let area = Rect::new(0, 0, 20, 4);
        let popup_area = QuitConfirmPopup::centered_area(area);

        assert_eq!(popup_area.width, 20);
        assert_eq!(popup_area.height, 4);
    }

    #[test]
    fn test_quit_confirm_renders_without_panic() {
        let area = Rect::new(0, 0, 60, 20);
        let popup_area = QuitConfirmPopup::centered_area(area);
        let mut buf = Buffer::empty(area);
        let popup = QuitConfirmPopup::new(1, Theme::Dark);
        popup.render(popup_area, &mut buf);
    }

    #[test]
    fn test_quit_confirm_renders_yes_selected() {
        let area = Rect::new(0, 0, 60, 20);
        let popup_area = QuitConfirmPopup::centered_area(area);
        let mut buf = Buffer::empty(area);
        let popup = QuitConfirmPopup::new(0, Theme::Dark);
        popup.render(popup_area, &mut buf);

        let content: String = buf.content().iter().map(|c| c.symbol()).collect();
        assert!(content.contains("Quit?"));
        assert!(content.contains("Yes"));
        assert!(content.contains("No"));
    }
}
