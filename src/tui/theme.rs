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

/// Spike detection level for cost coloring
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpikeLevel {
    Normal,
    Elevated,
    High,
}

/// Determine spike level for a cost value relative to the daily average.
/// Returns Normal if avg_cost is 0 (no data or single day).
pub fn spike_level(cost: f64, avg_cost: f64) -> SpikeLevel {
    if avg_cost > 0.0 && cost >= avg_cost * 2.0 {
        SpikeLevel::High
    } else if avg_cost > 0.0 && cost >= avg_cost * 1.5 {
        SpikeLevel::Elevated
    } else {
        SpikeLevel::Normal
    }
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

    /// Spike warning color (elevated spending: 1.5x~2x daily avg)
    pub fn spike_warn(self) -> Color {
        match self {
            Self::Dark => Color::Indexed(208), // orange (ANSI 256) — distinct from Yellow date
            Self::Light => Color::Indexed(166), // dark orange (ANSI 256)
        }
    }

    /// Spike high color (spike spending: >= 2x daily avg)
    pub fn spike_high(self) -> Color {
        match self {
            Self::Dark => Color::Indexed(196), // bright red (ANSI 256) — distinct from Magenta cost
            Self::Light => Color::Indexed(160), // strong red (ANSI 256)
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

    /// Spike detection color based on spike level
    pub fn spike_color(self, level: SpikeLevel) -> Color {
        match level {
            SpikeLevel::Normal => self.text(),
            SpikeLevel::Elevated => self.spike_warn(),
            SpikeLevel::High => self.spike_high(),
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
        assert_eq!(t.spike_warn(), Color::Indexed(208));
        assert_eq!(t.spike_high(), Color::Indexed(196));
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
        assert_eq!(t.spike_warn(), Color::Indexed(166));
        assert_eq!(t.spike_high(), Color::Indexed(160));
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

    #[test]
    fn test_spike_level_normal() {
        assert_eq!(spike_level(1.0, 1.0), SpikeLevel::Normal);
        assert_eq!(spike_level(1.49, 1.0), SpikeLevel::Normal);
    }

    #[test]
    fn test_spike_level_elevated() {
        assert_eq!(spike_level(1.5, 1.0), SpikeLevel::Elevated);
        assert_eq!(spike_level(1.99, 1.0), SpikeLevel::Elevated);
    }

    #[test]
    fn test_spike_level_high() {
        assert_eq!(spike_level(2.0, 1.0), SpikeLevel::High);
        assert_eq!(spike_level(5.0, 1.0), SpikeLevel::High);
    }

    #[test]
    fn test_spike_level_zero_avg() {
        assert_eq!(spike_level(0.0, 0.0), SpikeLevel::Normal);
        assert_eq!(spike_level(100.0, 0.0), SpikeLevel::Normal);
    }

    #[test]
    fn test_spike_level_zero_cost() {
        assert_eq!(spike_level(0.0, 1.0), SpikeLevel::Normal);
    }

    #[test]
    fn test_dark_spike_color() {
        let t = Theme::Dark;
        assert_eq!(t.spike_color(SpikeLevel::Normal), t.text());
        assert_eq!(t.spike_color(SpikeLevel::Elevated), t.spike_warn());
        assert_eq!(t.spike_color(SpikeLevel::High), t.spike_high());
    }

    #[test]
    fn test_light_spike_color() {
        let t = Theme::Light;
        assert_eq!(t.spike_color(SpikeLevel::Normal), t.text());
        assert_eq!(t.spike_color(SpikeLevel::Elevated), t.spike_warn());
        assert_eq!(t.spike_color(SpikeLevel::High), t.spike_high());
    }
}
