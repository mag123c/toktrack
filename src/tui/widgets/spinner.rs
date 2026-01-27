//! Loading spinner widget

use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Style},
    widgets::Widget,
};

/// Spinner animation frames
const SPINNER_FRAMES: &[char] = &['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];

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
        if area.height == 0 || area.width < 20 {
            return;
        }

        let spinner_char = self.current_char();
        let message = self.stage.message();
        let text = format!("{} {}", spinner_char, message);

        // Center vertically and horizontally
        let y = area.y + area.height / 2;
        let x = area.x + (area.width.saturating_sub(text.len() as u16)) / 2;

        buf.set_string(x, y, &text, Style::default().fg(Color::Cyan));
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
