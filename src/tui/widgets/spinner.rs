//! Loading spinner widget

use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Modifier, Style},
    widgets::Widget,
};

/// Spinner animation frames
const SPINNER_FRAMES: &[char] = &['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];

/// App branding
const APP_NAME: &str = "toktrack";
const TAGLINE: &str = "Ultra-fast LLM token tracker";

/// Loading stage for display
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoadingStage {
    Scanning,
    Parsing,
    Aggregating,
}

impl LoadingStage {
    pub fn message(self) -> &'static str {
        match self {
            Self::Scanning => "Scanning files...",
            Self::Parsing => "Parsing data...",
            Self::Aggregating => "Aggregating results...",
        }
    }
}

/// Loading spinner widget
pub struct Spinner {
    frame: usize,
    stage: LoadingStage,
}

impl Spinner {
    pub fn new(frame: usize, stage: LoadingStage) -> Self {
        Self { frame, stage }
    }

    /// Get the current spinner character
    pub fn current_char(&self) -> char {
        SPINNER_FRAMES[self.frame % SPINNER_FRAMES.len()]
    }

    /// Advance to next frame, returning the new frame index
    pub fn next_frame(frame: usize) -> usize {
        (frame + 1) % SPINNER_FRAMES.len()
    }
}

impl Widget for Spinner {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if area.height < 5 || area.width < 35 {
            return;
        }

        // Calculate center Y (4 lines: name, tagline, empty, spinner)
        let center_y = area.y + area.height / 2;

        // App name (bold, white)
        let name_y = center_y.saturating_sub(2);
        let name_x = area.x + (area.width.saturating_sub(APP_NAME.len() as u16)) / 2;
        buf.set_string(
            name_x,
            name_y,
            APP_NAME,
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        );

        // Tagline (dim gray)
        let tag_y = name_y + 1;
        let tag_x = area.x + (area.width.saturating_sub(TAGLINE.len() as u16)) / 2;
        buf.set_string(tag_x, tag_y, TAGLINE, Style::default().fg(Color::DarkGray));

        // Spinner (cyan) - 1 blank line after tagline
        let spinner_text = format!("{} {}", self.current_char(), self.stage.message());
        let spinner_y = tag_y + 2;
        let spinner_x = area.x + (area.width.saturating_sub(spinner_text.len() as u16)) / 2;
        buf.set_string(
            spinner_x,
            spinner_y,
            &spinner_text,
            Style::default().fg(Color::Cyan),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_spinner_frames() {
        assert_eq!(SPINNER_FRAMES.len(), 10);
    }

    #[test]
    fn test_spinner_current_char() {
        let spinner = Spinner::new(0, LoadingStage::Scanning);
        assert_eq!(spinner.current_char(), '⠋');

        let spinner = Spinner::new(5, LoadingStage::Scanning);
        assert_eq!(spinner.current_char(), '⠴');
    }

    #[test]
    fn test_spinner_wraps() {
        let spinner = Spinner::new(10, LoadingStage::Scanning);
        assert_eq!(spinner.current_char(), '⠋'); // 10 % 10 = 0
    }

    #[test]
    fn test_next_frame() {
        assert_eq!(Spinner::next_frame(0), 1);
        assert_eq!(Spinner::next_frame(9), 0);
    }

    #[test]
    fn test_loading_stage_message() {
        assert_eq!(LoadingStage::Scanning.message(), "Scanning files...");
        assert_eq!(LoadingStage::Parsing.message(), "Parsing data...");
        assert_eq!(
            LoadingStage::Aggregating.message(),
            "Aggregating results..."
        );
    }
}
