//! Tab bar widget for view navigation

use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Modifier, Style},
    widgets::Widget,
};

use crate::tui::theme::Theme;

/// Available tabs in the application
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Tab {
    #[default]
    Overview,
    Models,
    Daily,
    Stats,
}

impl Tab {
    /// Get the display label for this tab
    pub fn label(self) -> &'static str {
        match self {
            Self::Overview => "Overview",
            Self::Models => "Models",
            Self::Daily => "Daily",
            Self::Stats => "Stats",
        }
    }

    /// Get all tabs in order
    pub fn all() -> &'static [Tab] {
        &[Tab::Overview, Tab::Models, Tab::Daily, Tab::Stats]
    }

    /// Get the next tab (wrapping)
    pub fn next(self) -> Self {
        match self {
            Self::Overview => Self::Models,
            Self::Models => Self::Daily,
            Self::Daily => Self::Stats,
            Self::Stats => Self::Overview,
        }
    }

    /// Get the previous tab (wrapping)
    pub fn prev(self) -> Self {
        match self {
            Self::Overview => Self::Stats,
            Self::Models => Self::Overview,
            Self::Daily => Self::Models,
            Self::Stats => Self::Daily,
        }
    }

    /// Get tab from number key (1-4)
    pub fn from_number(n: u8) -> Option<Self> {
        match n {
            1 => Some(Self::Overview),
            2 => Some(Self::Models),
            3 => Some(Self::Daily),
            4 => Some(Self::Stats),
            _ => None,
        }
    }
}

/// Tab bar widget showing available views
pub struct TabBar {
    selected: Tab,
    theme: Theme,
}

impl TabBar {
    pub fn new(selected: Tab, theme: Theme) -> Self {
        Self { selected, theme }
    }
}

impl Widget for TabBar {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if area.width == 0 || area.height == 0 {
            return;
        }

        // Calculate total width of all tabs for centering
        let total_width: u16 = Tab::all()
            .iter()
            .map(|tab| {
                let label = tab.label();
                let display_len = if *tab == self.selected {
                    label.len() + 2 // "[label]"
                } else {
                    label.len()
                };
                display_len as u16 + 2 // + spacing
            })
            .sum::<u16>()
            .saturating_sub(2); // Remove trailing spacing

        // Center the tabs
        let start_x = area.x + (area.width.saturating_sub(total_width)) / 2;
        let mut x = start_x;

        for tab in Tab::all() {
            let is_selected = *tab == self.selected;
            let label = tab.label();

            // Calculate display string
            let display = if is_selected {
                format!("[{}]", label)
            } else {
                label.to_string()
            };

            let display_len = display.len() as u16;
            if x + display_len > area.x + area.width {
                break;
            }

            // Style based on selection
            let style = if is_selected {
                Style::default()
                    .fg(self.theme.accent())
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(self.theme.muted())
            };

            buf.set_string(x, area.y, &display, style);
            x += display_len + 2; // Add spacing between tabs
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tab_labels() {
        assert_eq!(Tab::Overview.label(), "Overview");
        assert_eq!(Tab::Models.label(), "Models");
        assert_eq!(Tab::Daily.label(), "Daily");
        assert_eq!(Tab::Stats.label(), "Stats");
    }

    #[test]
    fn test_tab_all() {
        let all = Tab::all();
        assert_eq!(all.len(), 4);
        assert_eq!(all[0], Tab::Overview);
        assert_eq!(all[3], Tab::Stats);
    }

    #[test]
    fn test_tab_next() {
        assert_eq!(Tab::Overview.next(), Tab::Models);
        assert_eq!(Tab::Models.next(), Tab::Daily);
        assert_eq!(Tab::Daily.next(), Tab::Stats);
        assert_eq!(Tab::Stats.next(), Tab::Overview);
    }

    #[test]
    fn test_tab_prev() {
        assert_eq!(Tab::Overview.prev(), Tab::Stats);
        assert_eq!(Tab::Stats.prev(), Tab::Daily);
        assert_eq!(Tab::Daily.prev(), Tab::Models);
        assert_eq!(Tab::Models.prev(), Tab::Overview);
    }

    #[test]
    fn test_tab_default() {
        assert_eq!(Tab::default(), Tab::Overview);
    }

    #[test]
    fn test_tab_from_number() {
        assert_eq!(Tab::from_number(1), Some(Tab::Overview));
        assert_eq!(Tab::from_number(2), Some(Tab::Models));
        assert_eq!(Tab::from_number(3), Some(Tab::Daily));
        assert_eq!(Tab::from_number(4), Some(Tab::Stats));
        assert_eq!(Tab::from_number(0), None);
        assert_eq!(Tab::from_number(5), None);
    }
}
