//! Tab bar widget for view navigation

use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Modifier, Style},
    widgets::Widget,
};

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
}

/// Tab bar widget showing available views
pub struct TabBar {
    selected: Tab,
}

impl TabBar {
    pub fn new(selected: Tab) -> Self {
        Self { selected }
    }
}

impl Widget for TabBar {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if area.width == 0 || area.height == 0 {
            return;
        }

        let mut x = area.x + 1; // Start with padding

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
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::DarkGray)
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
}
