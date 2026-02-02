//! Help popup widget - displays keyboard shortcuts

use ratatui::{
    buffer::Buffer,
    layout::{Alignment, Constraint, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph, Widget},
};

use crate::tui::theme::Theme;

/// Version from Cargo.toml
const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Width and height of the help popup
const POPUP_WIDTH: u16 = 42;
const POPUP_HEIGHT: u16 = 17;

/// Help popup widget showing keyboard shortcuts
pub struct HelpPopup {
    theme: Theme,
}

impl HelpPopup {
    pub fn new(theme: Theme) -> Self {
        Self { theme }
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
        Self::new(Theme::default())
    }
}

impl Widget for HelpPopup {
    fn render(self, area: Rect, buf: &mut Buffer) {
        // Clear the area first (for overlay effect)
        Clear.render(area, buf);

        // Create block with border
        let title = format!(" toktrack v{} ", VERSION);
        let block = Block::default()
            .title(title)
            .title_alignment(Alignment::Center)
            .borders(Borders::ALL)
            .border_style(Style::default().fg(self.theme.accent()));

        let inner = block.inner(area);
        block.render(area, buf);

        // Layout for content
        let chunks = Layout::vertical([
            Constraint::Length(1), // [0] Padding
            Constraint::Length(1), // [1] Navigation header
            Constraint::Length(1), // [2] Separator
            Constraint::Length(1), // [3] Tab/Shift+Tab
            Constraint::Length(1), // [4] 1-4
            Constraint::Length(1), // [5] Up/Down
            Constraint::Length(1), // [6] d/w/m
            Constraint::Length(1), // [7] Padding
            Constraint::Length(1), // [8] General header
            Constraint::Length(1), // [9] Separator
            Constraint::Length(1), // [10] q/Esc
            Constraint::Length(1), // [11] ?
            Constraint::Length(1), // [12] Padding
            Constraint::Length(1), // [13] Close hint
            Constraint::Min(0),    // Remaining
        ])
        .split(inner);

        // Navigation section
        let nav_header = Line::from(vec![Span::styled(
            "Navigation",
            Style::default()
                .fg(self.theme.date())
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
            Style::default().fg(self.theme.muted()),
        );

        // Keybindings
        render_keybinding(chunks[3], buf, "Tab / Shift+Tab", "Switch view", self.theme);
        render_keybinding(chunks[4], buf, "1-4", "Jump to view", self.theme);
        render_keybinding(
            chunks[5],
            buf,
            "Up/Down or j/k",
            "Scroll (Daily)",
            self.theme,
        );
        render_keybinding(
            chunks[6],
            buf,
            "d / w / m",
            "Daily/Weekly/Monthly",
            self.theme,
        );

        // General section
        let gen_header = Line::from(vec![Span::styled(
            "General",
            Style::default()
                .fg(self.theme.date())
                .add_modifier(Modifier::BOLD),
        )]);
        Paragraph::new(gen_header)
            .alignment(Alignment::Left)
            .render(chunks[8], buf);

        // Separator
        buf.set_string(
            chunks[9].x,
            chunks[9].y,
            &sep,
            Style::default().fg(self.theme.muted()),
        );

        render_keybinding(chunks[10], buf, "q / Esc", "Quit", self.theme);
        render_keybinding(chunks[11], buf, "?", "Toggle help", self.theme);

        // Close hint
        let hint = Line::from(vec![Span::styled(
            "Press ? or Esc to close",
            Style::default().fg(self.theme.muted()),
        )]);
        Paragraph::new(hint)
            .alignment(Alignment::Center)
            .render(chunks[13], buf);
    }
}

/// Render a single keybinding line
fn render_keybinding(area: Rect, buf: &mut Buffer, key: &str, desc: &str, theme: Theme) {
    let line = Line::from(vec![
        Span::styled(
            format!("  {:<18}", key),
            Style::default().fg(theme.accent()),
        ),
        Span::styled(desc, Style::default().fg(theme.text())),
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
