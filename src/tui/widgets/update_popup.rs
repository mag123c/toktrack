//! Update popup widget - displays update notification overlay

use ratatui::{
    buffer::Buffer,
    layout::{Alignment, Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph, Widget},
};

/// Width and height of the update popup
const POPUP_WIDTH: u16 = 48;
const POPUP_HEIGHT: u16 = 11;

/// Update popup overlay showing available update info
pub struct UpdatePopup<'a> {
    current: &'a str,
    latest: &'a str,
    selection: u8, // 0 = Update now, 1 = Skip
}

impl<'a> UpdatePopup<'a> {
    pub fn new(current: &'a str, latest: &'a str, selection: u8) -> Self {
        Self {
            current,
            latest,
            selection,
        }
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

impl<'a> Widget for UpdatePopup<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        // Clear the area first (for overlay effect)
        Clear.render(area, buf);

        // Create block with border
        let block = Block::default()
            .title(" Update Available ")
            .title_alignment(Alignment::Center)
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Yellow));

        let inner = block.inner(area);
        block.render(area, buf);

        // Layout for content
        let chunks = Layout::vertical([
            Constraint::Length(1), // [0] Padding
            Constraint::Length(1), // [1] Version info
            Constraint::Length(1), // [2] Padding
            Constraint::Length(1), // [3] Separator
            Constraint::Length(1), // [4] Padding
            Constraint::Length(1), // [5] Update now
            Constraint::Length(1), // [6] Skip
            Constraint::Length(1), // [7] Padding
            Constraint::Length(1), // [8] Key hints
        ])
        .split(inner);

        // Version info line
        let version_line = Line::from(vec![
            Span::styled("  v", Style::default().fg(Color::White)),
            Span::styled(
                self.current,
                Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
            ),
            Span::styled("  →  ", Style::default().fg(Color::DarkGray)),
            Span::styled("v", Style::default().fg(Color::White)),
            Span::styled(
                self.latest,
                Style::default()
                    .fg(Color::Green)
                    .add_modifier(Modifier::BOLD),
            ),
        ]);
        Paragraph::new(version_line)
            .alignment(Alignment::Center)
            .render(chunks[1], buf);

        // Separator
        let sep = "─".repeat(inner.width as usize);
        buf.set_string(
            chunks[3].x,
            chunks[3].y,
            &sep,
            Style::default().fg(Color::DarkGray),
        );

        // Selection items
        let (update_marker, update_style) = if self.selection == 0 {
            (
                "▸ ",
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            )
        } else {
            ("  ", Style::default().fg(Color::DarkGray))
        };
        let update_line = Line::from(vec![
            Span::styled(update_marker, update_style),
            Span::styled("Update now", update_style),
        ]);
        Paragraph::new(update_line)
            .alignment(Alignment::Center)
            .render(chunks[5], buf);

        let (skip_marker, skip_style) = if self.selection == 1 {
            (
                "▸ ",
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            )
        } else {
            ("  ", Style::default().fg(Color::DarkGray))
        };
        let skip_line = Line::from(vec![
            Span::styled(skip_marker, skip_style),
            Span::styled("Skip", skip_style),
        ]);
        Paragraph::new(skip_line)
            .alignment(Alignment::Center)
            .render(chunks[6], buf);

        // Key hints
        let hint_line = Line::from(vec![
            Span::styled(
                "↑↓",
                Style::default()
                    .fg(Color::DarkGray)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(" Select  ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                "Enter",
                Style::default()
                    .fg(Color::DarkGray)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(" Confirm", Style::default().fg(Color::DarkGray)),
        ]);
        Paragraph::new(hint_line)
            .alignment(Alignment::Center)
            .render(chunks[8], buf);
    }
}

/// Message popup for update progress/result
pub struct UpdateMessagePopup<'a> {
    message: &'a str,
    color: Color,
}

impl<'a> UpdateMessagePopup<'a> {
    pub fn new(message: &'a str, color: Color) -> Self {
        Self { message, color }
    }

    /// Calculate centered popup area (smaller than main popup)
    pub fn centered_area(area: Rect) -> Rect {
        let width = 48u16;
        let height = 5u16;
        let x = area.x + (area.width.saturating_sub(width)) / 2;
        let y = area.y + (area.height.saturating_sub(height)) / 2;
        Rect {
            x,
            y,
            width: width.min(area.width),
            height: height.min(area.height),
        }
    }
}

impl<'a> Widget for UpdateMessagePopup<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        Clear.render(area, buf);

        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(self.color));

        let inner = block.inner(area);
        block.render(area, buf);

        let chunks = Layout::vertical([
            Constraint::Length(1), // Padding
            Constraint::Length(1), // Message
            Constraint::Min(0),    // Remaining
        ])
        .split(inner);

        let line = Line::from(Span::styled(self.message, Style::default().fg(self.color)));
        Paragraph::new(line)
            .alignment(Alignment::Center)
            .render(chunks[1], buf);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_update_popup_centered_area() {
        let area = Rect::new(0, 0, 100, 50);
        let popup_area = UpdatePopup::centered_area(area);

        assert_eq!(popup_area.width, POPUP_WIDTH);
        assert_eq!(popup_area.height, POPUP_HEIGHT);
        assert_eq!(popup_area.x, (100 - POPUP_WIDTH) / 2);
        assert_eq!(popup_area.y, (50 - POPUP_HEIGHT) / 2);
    }

    #[test]
    fn test_update_popup_small_terminal() {
        let area = Rect::new(0, 0, 30, 5);
        let popup_area = UpdatePopup::centered_area(area);

        assert_eq!(popup_area.width, 30);
        assert_eq!(popup_area.height, 5);
    }

    #[test]
    fn test_update_popup_renders_without_panic() {
        let area = Rect::new(0, 0, 60, 20);
        let popup_area = UpdatePopup::centered_area(area);
        let mut buf = Buffer::empty(area);
        let popup = UpdatePopup::new("0.1.14", "0.2.0", 0);
        popup.render(popup_area, &mut buf);
    }

    #[test]
    fn test_update_message_popup_centered_area() {
        let area = Rect::new(0, 0, 100, 50);
        let popup_area = UpdateMessagePopup::centered_area(area);

        assert_eq!(popup_area.width, 48);
        assert_eq!(popup_area.height, 5);
    }

    #[test]
    fn test_update_message_popup_renders_content() {
        let area = Rect::new(0, 0, 60, 20);
        let popup_area = UpdateMessagePopup::centered_area(area);

        // Success message
        let mut buf = Buffer::empty(area);
        let popup_success =
            UpdateMessagePopup::new("Updated! Press any key to exit.", Color::Green);
        popup_success.render(popup_area, &mut buf);

        let content: String = buf.content().iter().map(|c| c.symbol()).collect();
        assert!(
            content.contains("Updated! Press any key to exit."),
            "Success message not found in buffer"
        );

        // Error message
        let mut buf2 = Buffer::empty(area);
        let popup_error = UpdateMessagePopup::new("Failed: npm error", Color::Red);
        popup_error.render(popup_area, &mut buf2);

        let content2: String = buf2.content().iter().map(|c| c.symbol()).collect();
        assert!(
            content2.contains("Failed: npm error"),
            "Error message not found in buffer"
        );
    }
}
