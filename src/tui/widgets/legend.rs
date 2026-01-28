//! Legend widget for heatmap intensity levels

use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Style},
    widgets::Widget,
};

use super::heatmap::HeatmapIntensity;

/// Legend widget showing intensity scale
pub struct Legend;

impl Legend {
    pub fn new() -> Self {
        Self
    }

    /// Returns the minimum width needed to render the legend
    pub fn min_width() -> u16 {
        // "Less ▪▪ ▪▪ ▪▪ ▪▪ More" = 21 chars (2-char cells with spaces)
        21
    }
}

impl Default for Legend {
    fn default() -> Self {
        Self::new()
    }
}

impl Widget for Legend {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if area.width < Self::min_width() {
            return;
        }

        let intensities = [
            HeatmapIntensity::Low,
            HeatmapIntensity::Medium,
            HeatmapIntensity::High,
            HeatmapIntensity::Max,
        ];

        // Right-align the legend
        let legend_width = Self::min_width();
        let start_x = area.x + area.width.saturating_sub(legend_width);
        let y = area.y;

        let mut x = start_x;

        // "Less "
        buf.set_string(x, y, "Less ", Style::default().fg(Color::DarkGray));
        x += 5;

        // Intensity cells (2-char each) - use uniform block for consistent visual size
        for intensity in intensities {
            let style = Style::default().fg(intensity.color());
            buf.set_string(x, y, "██", style);
            x += 2; // 2-char cell

            // Space between cells except last
            if intensity != HeatmapIntensity::Max {
                buf.set_string(x, y, " ", Style::default());
                x += 1;
            }
        }

        // " More"
        buf.set_string(x, y, " More", Style::default().fg(Color::DarkGray));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_legend_min_width() {
        // "Less ▪▪ ▪▪ ▪▪ ▪▪ More" = 21 chars (2-char cells)
        assert_eq!(Legend::min_width(), 21);
    }

    #[test]
    fn test_legend_new() {
        let _legend = Legend::new();
    }
}
