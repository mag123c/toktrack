//! Terminal theme detection and color definitions

use ratatui::style::Color;

/// Heatmap intensity level for theme-aware coloring
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HeatmapLevel {
    None,
    Low,
    Medium,
    High,
    Max,
}

/// Terminal color scheme (dark or light background)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Theme {
    #[default]
    Dark,
    Light,
}

impl Theme {
    /// Auto-detect terminal theme from background luminance.
    /// Must be called **before** entering raw mode (ratatui::init).
    /// Falls back to Dark if detection fails.
    pub fn detect() -> Self {
        match terminal_light::luma() {
            Ok(luma) if luma > 0.6 => Self::Light,
            _ => Self::Dark,
        }
    }

    /// Primary text color (headers, body text)
    pub fn text(self) -> Color {
        match self {
            Self::Dark => Color::White,
            Self::Light => Color::Black,
        }
    }

    /// Active/accent color (selected tabs, keybinding keys, interactive elements)
    pub fn accent(self) -> Color {
        match self {
            Self::Dark => Color::Cyan,
            Self::Light => Color::Indexed(25), // dark blue (ANSI 256)
        }
    }

    /// Secondary/muted text (separators, inactive tabs, hints)
    pub fn muted(self) -> Color {
        match self {
            Self::Dark => Color::DarkGray,
            Self::Light => Color::Gray,
        }
    }

    /// Date text color
    pub fn date(self) -> Color {
        match self {
            Self::Dark => Color::Yellow,
            Self::Light => Color::Indexed(130), // dark orange/yellow (ANSI 256)
        }
    }

    /// Cost/money text color
    pub fn cost(self) -> Color {
        match self {
            Self::Dark => Color::Magenta,
            Self::Light => Color::Indexed(90), // dark magenta (ANSI 256)
        }
    }

    /// Bar/sparkline/positive indicator color
    pub fn bar(self) -> Color {
        match self {
            Self::Dark => Color::Green,
            Self::Light => Color::Indexed(22), // dark green (ANSI 256)
        }
    }

    /// Error/negative indicator color
    pub fn error(self) -> Color {
        match self {
            Self::Dark => Color::Red,
            Self::Light => Color::Indexed(124), // dark red (ANSI 256)
        }
    }

    /// Stats accent color (Daily Average card)
    pub fn stat_blue(self) -> Color {
        match self {
            Self::Dark => Color::Blue,
            Self::Light => Color::Indexed(25), // dark blue (ANSI 256)
        }
    }

    /// Stats warm highlight (Total Cost card)
    pub fn stat_warm(self) -> Color {
        match self {
            Self::Dark => Color::LightRed,
            Self::Light => Color::Red,
        }
    }

    /// Heatmap intensity color
    pub fn heatmap_color(self, level: HeatmapLevel) -> Color {
        match self {
            Self::Dark => match level {
                HeatmapLevel::None => Color::Indexed(236),
                HeatmapLevel::Low => Color::Indexed(22),
                HeatmapLevel::Medium => Color::Indexed(28),
                HeatmapLevel::High => Color::Indexed(34),
                HeatmapLevel::Max => Color::Indexed(40),
            },
            Self::Light => match level {
                HeatmapLevel::None => Color::Indexed(254),
                HeatmapLevel::Low => Color::Indexed(194),
                HeatmapLevel::Medium => Color::Indexed(157),
                HeatmapLevel::High => Color::Indexed(71),
                HeatmapLevel::Max => Color::Indexed(28),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_dark_theme_colors() {
        let t = Theme::Dark;
        assert_eq!(t.text(), Color::White);
        assert_eq!(t.accent(), Color::Cyan);
        assert_eq!(t.muted(), Color::DarkGray);
        assert_eq!(t.date(), Color::Yellow);
        assert_eq!(t.cost(), Color::Magenta);
        assert_eq!(t.bar(), Color::Green);
        assert_eq!(t.error(), Color::Red);
        assert_eq!(t.stat_blue(), Color::Blue);
        assert_eq!(t.stat_warm(), Color::LightRed);
    }

    #[test]
    fn test_light_theme_colors() {
        let t = Theme::Light;
        assert_eq!(t.text(), Color::Black);
        assert_eq!(t.accent(), Color::Indexed(25));
        assert_eq!(t.muted(), Color::Gray);
        assert_eq!(t.date(), Color::Indexed(130));
        assert_eq!(t.cost(), Color::Indexed(90));
        assert_eq!(t.bar(), Color::Indexed(22));
        assert_eq!(t.error(), Color::Indexed(124));
        assert_eq!(t.stat_blue(), Color::Indexed(25));
        assert_eq!(t.stat_warm(), Color::Red);
    }

    #[test]
    fn test_default_is_dark() {
        assert_eq!(Theme::default(), Theme::Dark);
    }

    #[test]
    fn test_dark_heatmap_colors() {
        let t = Theme::Dark;
        assert_eq!(t.heatmap_color(HeatmapLevel::None), Color::Indexed(236));
        assert_eq!(t.heatmap_color(HeatmapLevel::Low), Color::Indexed(22));
        assert_eq!(t.heatmap_color(HeatmapLevel::Medium), Color::Indexed(28));
        assert_eq!(t.heatmap_color(HeatmapLevel::High), Color::Indexed(34));
        assert_eq!(t.heatmap_color(HeatmapLevel::Max), Color::Indexed(40));
    }

    #[test]
    fn test_light_heatmap_colors() {
        let t = Theme::Light;
        assert_eq!(t.heatmap_color(HeatmapLevel::None), Color::Indexed(254));
        assert_eq!(t.heatmap_color(HeatmapLevel::Low), Color::Indexed(194));
        assert_eq!(t.heatmap_color(HeatmapLevel::Medium), Color::Indexed(157));
        assert_eq!(t.heatmap_color(HeatmapLevel::High), Color::Indexed(71));
        assert_eq!(t.heatmap_color(HeatmapLevel::Max), Color::Indexed(28));
    }
}
