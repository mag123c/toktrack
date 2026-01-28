//! Help popup widget - displays keyboard shortcuts

use ratatui::{
    buffer::Buffer,
    layout::{Alignment, Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph, Widget},
};

/// Width and height of the help popup
const POPUP_WIDTH: u16 = 42;
const POPUP_HEIGHT: u16 = 16;

/// Help popup widget showing keyboard shortcuts
pub struct HelpPopup;

impl HelpPopup {
    pub fn new() -> Self {
        Self
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

impl Default for HelpPopup {
    fn default() -> Self {
        Self::new()
    }
}

impl Widget for HelpPopup {
    fn render(self, area: Rect, buf: &mut Buffer) {
        // Clear the area first (for overlay effect)
        Clear.render(area, buf);

        // Create block with border
        let block = Block::default()
            .title(" Help ")
            .title_alignment(Alignment::Center)
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Cyan));

        let inner = block.inner(area);
        block.render(area, buf);

        // Layout for content
        let chunks = Layout::vertical([
            Constraint::Length(1), // Padding
            Constraint::Length(1), // Navigation header
            Constraint::Length(1), // Separator
            Constraint::Length(1), // Tab/Shift+Tab
            Constraint::Length(1), // 1-4
            Constraint::Length(1), // Up/Down
            Constraint::Length(1), // Padding
            Constraint::Length(1), // General header
            Constraint::Length(1), // Separator
            Constraint::Length(1), // q/Esc
            Constraint::Length(1), // ?
            Constraint::Length(1), // Padding
            Constraint::Length(1), // Close hint
            Constraint::Min(0),    // Remaining
        ])
        .split(inner);

        // Navigation section
        let nav_header = Line::from(vec![Span::styled(
            "Navigation",
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )]);
        Paragraph::new(nav_header)
            .alignment(Alignment::Left)
            .render(chunks[1], buf);

        // Separator
        let sep = "─".repeat(inner.width as usize);
        buf.set_string(
            chunks[2].x,
            chunks[2].y,
            &sep,
            Style::default().fg(Color::DarkGray),
        );

        // Keybindings
        render_keybinding(chunks[3], buf, "Tab / Shift+Tab", "Switch view");
        render_keybinding(chunks[4], buf, "1-4", "Jump to view");
        render_keybinding(chunks[5], buf, "Up/Down or j/k", "Scroll (Daily)");

        // General section
        let gen_header = Line::from(vec![Span::styled(
            "General",
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )]);
        Paragraph::new(gen_header)
            .alignment(Alignment::Left)
            .render(chunks[7], buf);

        // Separator
        buf.set_string(
            chunks[8].x,
            chunks[8].y,
            &sep,
            Style::default().fg(Color::DarkGray),
        );

        render_keybinding(chunks[9], buf, "q / Esc", "Quit");
        render_keybinding(chunks[10], buf, "?", "Toggle help");

        // Close hint
        let hint = Line::from(vec![Span::styled(
            "Press ? or Esc to close",
            Style::default().fg(Color::DarkGray),
        )]);
        Paragraph::new(hint)
            .alignment(Alignment::Center)
            .render(chunks[12], buf);
    }
}

/// Render a single keybinding line
fn render_keybinding(area: Rect, buf: &mut Buffer, key: &str, desc: &str) {
    let line = Line::from(vec![
        Span::styled(format!("  {:<18}", key), Style::default().fg(Color::Cyan)),
        Span::styled(desc, Style::default().fg(Color::White)),
    ]);
    Paragraph::new(line)
        .alignment(Alignment::Left)
        .render(area, buf);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_help_popup_centered_area() {
        let area = Rect::new(0, 0, 100, 50);
        let popup_area = HelpPopup::centered_area(area);

        // Should be centered
        assert_eq!(popup_area.width, POPUP_WIDTH);
        assert_eq!(popup_area.height, POPUP_HEIGHT);
        assert_eq!(popup_area.x, (100 - POPUP_WIDTH) / 2);
        assert_eq!(popup_area.y, (50 - POPUP_HEIGHT) / 2);
    }

    #[test]
    fn test_help_popup_small_terminal() {
        // Terminal smaller than popup
        let area = Rect::new(0, 0, 30, 10);
        let popup_area = HelpPopup::centered_area(area);

        // Should clamp to terminal size
        assert_eq!(popup_area.width, 30);
        assert_eq!(popup_area.height, 10);
    }
}
