//! Overview layout widget

use chrono::NaiveDate;
use ratatui::{
    buffer::Buffer,
    layout::{Alignment, Constraint, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Paragraph, Widget},
};

use super::heatmap::Heatmap;
use super::legend::Legend;
use super::tabs::{Tab, TabBar};
use crate::tui::theme::Theme;
use crate::types::{SourceUsage, TotalSummary};

/// Format a number with thousand separators (e.g., 1234567 -> "1,234,567")
/// Optimized: no Vec<char> allocation since digits are ASCII
pub fn format_number(n: u64) -> String {
    if n == 0 {
        return "0".to_string();
    }

    let s = n.to_string();
    let len = s.len();
    let mut result = String::with_capacity(len + len / 3);

    // Digits are ASCII, so byte indexing is safe
    for (i, ch) in s.bytes().enumerate() {
        if i > 0 && (len - i).is_multiple_of(3) {
            result.push(',');
        }
        result.push(ch as char);
    }

    result
}

/// Data for the overview display (references to avoid cloning)
#[derive(Debug)]
pub struct OverviewData<'a> {
    pub total: &'a TotalSummary,
    pub daily_tokens: &'a [(NaiveDate, u64)],
    pub source_usage: &'a [SourceUsage],
    pub selected_source: Option<usize>,
    pub selected_tab: Tab,
}

/// Maximum content width for Overview (keeps layout clean on wide terminals)
/// 52 weeks * 3-char cells + 4 label = 160, so 170 gives some padding
const MAX_CONTENT_WIDTH: u16 = 170;

/// Overview widget combining all elements
pub struct Overview<'a> {
    data: OverviewData<'a>,
    today: NaiveDate,
    theme: Theme,
}

impl<'a> Overview<'a> {
    pub fn new(data: OverviewData<'a>, today: NaiveDate, theme: Theme) -> Self {
        Self { data, today, theme }
    }
}

impl Widget for Overview<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        // Apply max width constraint and center the content
        let content_width = area.width.min(MAX_CONTENT_WIDTH);
        let x_offset = (area.width.saturating_sub(content_width)) / 2;
        let centered_area = Rect {
            x: area.x + x_offset,
            y: area.y,
            width: content_width,
            height: area.height,
        };

        // Determine source section height (1 row per source, 0-4 sources shown)
        let source_rows = self.data.source_usage.len().min(4) as u16;
        let show_sources = source_rows > 0;

        // Build layout constraints dynamically
        let mut constraints = vec![
            Constraint::Length(1), // 0: TabBar
            Constraint::Length(1), // 1: Separator
            Constraint::Length(3), // 2: Hero stat
            Constraint::Length(1), // 3: Sub-stats (Cost only)
            Constraint::Length(1), // 4: Blank
        ];

        let sources_label_idx = constraints.len(); // 5
        constraints.push(Constraint::Length(if show_sources { 1 } else { 0 }));

        // Reserve one extra line for the estimated-cost legend when any shown
        // source is estimated.
        let any_estimated = self
            .data
            .source_usage
            .iter()
            .take(4)
            .any(|s| s.supported && s.estimated);
        let sources_bars_idx = constraints.len(); // 6
        constraints.push(Constraint::Length(if show_sources {
            source_rows + u16::from(any_estimated)
        } else {
            0
        }));

        let _blank_after_sources_idx = constraints.len(); // 7
        constraints.push(Constraint::Length(1));

        let heatmap_idx = constraints.len(); // 8
        constraints.push(Constraint::Fill(1));

        let sep_idx = constraints.len(); // 9
        constraints.push(Constraint::Length(1));

        let keybindings_idx = constraints.len(); // 10
        constraints.push(Constraint::Length(1));

        let chunks = Layout::vertical(constraints).split(centered_area);

        // Render tab bar
        TabBar::new(self.data.selected_tab, self.theme).render(chunks[0], buf);

        // Render separator
        self.render_separator(chunks[1], buf);

        // Render hero stat
        self.render_hero_stat(chunks[2], buf);

        // Render sub-stats (Cost only)
        self.render_sub_stats(chunks[3], buf);

        // Render sources section if present
        if show_sources {
            self.render_sources_label(chunks[sources_label_idx], buf);
            self.render_source_bars(chunks[sources_bars_idx], buf);
        }

        // Render heatmap with legend
        self.render_heatmap_section(chunks[heatmap_idx], buf);

        // Render separator
        self.render_separator(chunks[sep_idx], buf);

        // Render keybindings
        self.render_keybindings(chunks[keybindings_idx], buf);
    }
}

impl Overview<'_> {
    fn render_separator(&self, area: Rect, buf: &mut Buffer) {
        let line = "─".repeat(area.width as usize);
        buf.set_string(
            area.x,
            area.y,
            &line,
            Style::default().fg(self.theme.muted()),
        );
    }

    fn render_hero_stat(&self, area: Rect, buf: &mut Buffer) {
        let total_tokens = self.data.total.total_input_tokens
            + self.data.total.total_output_tokens
            + self.data.total.total_cache_read_tokens
            + self.data.total.total_cache_creation_tokens
            + self.data.total.total_reasoning_tokens;
        let formatted = format_number(total_tokens);

        let hero = Paragraph::new(vec![
            Line::from(Span::styled(
                &formatted,
                Style::default()
                    .fg(self.theme.accent())
                    .add_modifier(Modifier::BOLD),
            )),
            Line::from(Span::styled(
                "tokens",
                Style::default().fg(self.theme.muted()),
            )),
        ])
        .alignment(Alignment::Center);

        hero.render(area, buf);
    }

    fn render_sub_stats(&self, area: Rect, buf: &mut Buffer) {
        let cost_str = format!("Cost: ${:.2}", self.data.total.total_cost_usd);

        let stats = Paragraph::new(Line::from(vec![Span::styled(
            cost_str,
            Style::default().fg(self.theme.cost()),
        )]))
        .alignment(Alignment::Center);

        stats.render(area, buf);
    }

    fn render_sources_label(&self, area: Rect, buf: &mut Buffer) {
        let label = Paragraph::new(Line::from(Span::styled(
            "Sources:",
            Style::default()
                .fg(self.theme.text())
                .add_modifier(Modifier::BOLD),
        )))
        .alignment(Alignment::Center);

        label.render(area, buf);
    }

    fn render_source_bars(&self, area: Rect, buf: &mut Buffer) {
        if self.data.source_usage.is_empty() {
            return;
        }

        let max_tokens = self
            .data
            .source_usage
            .iter()
            .map(|s| s.total_tokens)
            .max()
            .unwrap_or(1);

        // Bar rendering config
        const SOURCE_NAME_WIDTH: usize = 12;
        const BAR_WIDTH: usize = 20;
        const TOTAL_LINE_WIDTH: usize = SOURCE_NAME_WIDTH + 2 + BAR_WIDTH + 2 + 15; // name + "  " + bar + "  " + count

        // Calculate centering offset (account for 2-char marker prefix)
        let full_width = 2 + TOTAL_LINE_WIDTH;
        let x_offset = area.width.saturating_sub(full_width as u16) / 2;

        let mut any_estimated = false;
        for (i, source) in self.data.source_usage.iter().take(4).enumerate() {
            let y = area.y + i as u16;
            if y >= area.y + area.height {
                break;
            }

            // Selection marker
            let is_selected = self.data.selected_source == Some(i);
            let marker = if is_selected { "▸ " } else { "  " };

            // Source name (left-padded, fixed width)
            let name = if source.source.chars().count() > SOURCE_NAME_WIDTH - 1 {
                format!(
                    "{}…",
                    source
                        .source
                        .chars()
                        .take(SOURCE_NAME_WIDTH - 2)
                        .collect::<String>()
                )
            } else {
                source.source.clone()
            };
            let name_display = format!("{:>width$}", name, width = SOURCE_NAME_WIDTH);

            // Bar representation
            let ratio = source.total_tokens as f64 / max_tokens as f64;
            let filled = (ratio * BAR_WIDTH as f64).round() as usize;
            let filled = if source.total_tokens > 0 {
                filled.max(1)
            } else {
                filled
            };
            let filled = filled.min(BAR_WIDTH);
            let bar = format!("{}{}", "█".repeat(filled), "░".repeat(BAR_WIDTH - filled));

            // Token count
            let count_str = format_number(source.total_tokens);

            // Unsupported sources render as a dimmed, bar-less notice row
            // instead of a usage bar.
            if !source.supported {
                let dim = Style::default()
                    .fg(self.theme.muted())
                    .add_modifier(Modifier::DIM);
                let spans = vec![
                    Span::raw(marker),
                    Span::styled(name_display, dim),
                    Span::raw("  "),
                    Span::styled("(unsupported — no local usage)", dim),
                ];
                let line = Line::from(spans);
                buf.set_line(area.x + x_offset, y, &line, area.width - x_offset);
                continue;
            }

            // Build the line
            let name_style = if is_selected {
                Style::default()
                    .fg(self.theme.accent())
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(self.theme.text())
            };

            let mut spans = vec![
                Span::styled(marker, Style::default().fg(self.theme.accent())),
                Span::styled(name_display, name_style),
                Span::raw("  "),
                Span::styled(&bar, Style::default().fg(self.theme.bar())),
                Span::raw("  "),
                Span::styled(count_str, Style::default().fg(self.theme.text())),
            ];
            // Estimated-cost marker: this source's cost is LiteLLM-calculated.
            if source.estimated {
                any_estimated = true;
                spans.push(Span::styled(
                    " ~",
                    Style::default()
                        .fg(self.theme.muted())
                        .add_modifier(Modifier::DIM),
                ));
            }

            // Render centered
            let line = Line::from(spans);
            buf.set_line(area.x + x_offset, y, &line, area.width - x_offset);
        }

        // Legend for the estimated-cost marker (only when shown).
        if any_estimated {
            let y = area.y + self.data.source_usage.len().min(4) as u16;
            if y < area.y + area.height {
                let legend = Line::from(Span::styled(
                    "  ~ = estimated cost (LiteLLM)",
                    Style::default()
                        .fg(self.theme.muted())
                        .add_modifier(Modifier::DIM),
                ));
                buf.set_line(area.x + x_offset, y, &legend, area.width - x_offset);
            }
        }
    }

    fn render_heatmap_section(&self, area: Rect, buf: &mut Buffer) {
        const HEATMAP_GRID_ROWS: u16 = 7;
        const MONTH_LABEL_ROWS: u16 = 1;
        const BLANK_ROWS: u16 = 1;
        const LEGEND_ROWS: u16 = 1;
        const LEGEND_Y_OFFSET: u16 = HEATMAP_GRID_ROWS + MONTH_LABEL_ROWS + BLANK_ROWS;
        const REQUIRED_HEIGHT: u16 = LEGEND_Y_OFFSET + LEGEND_ROWS;

        let weeks = Heatmap::weeks_for_width(area.width);
        let heatmap = Heatmap::new(self.data.daily_tokens, self.today, weeks, self.theme);
        heatmap.render(area, buf);

        if area.height >= REQUIRED_HEIGHT {
            const LABEL_WIDTH: u16 = 4;
            const CELL_WIDTH: u16 = 2;
            let heatmap_width = LABEL_WIDTH + (weeks as u16 * CELL_WIDTH);
            let x_offset = area.width.saturating_sub(heatmap_width) / 2;

            let legend_width = Legend::min_width();
            let legend_x = area.x + x_offset + heatmap_width.saturating_sub(legend_width);

            let legend_area = Rect {
                x: legend_x,
                y: area.y + LEGEND_Y_OFFSET,
                width: legend_width.min(area.width),
                height: LEGEND_ROWS,
            };
            Legend::new(self.theme).render(legend_area, buf);
        }
    }

    fn render_keybindings(&self, area: Rect, buf: &mut Buffer) {
        let bindings = Paragraph::new(Line::from(vec![
            Span::styled("Tab", Style::default().fg(self.theme.accent())),
            Span::styled(": Switch view", Style::default().fg(self.theme.muted())),
            Span::raw("  "),
            Span::styled("↑↓", Style::default().fg(self.theme.accent())),
            Span::styled(": Select", Style::default().fg(self.theme.muted())),
            Span::raw("  "),
            Span::styled("Enter", Style::default().fg(self.theme.accent())),
            Span::styled(": Details", Style::default().fg(self.theme.muted())),
            Span::raw("  "),
            Span::styled("?", Style::default().fg(self.theme.accent())),
            Span::styled(": Help", Style::default().fg(self.theme.muted())),
            Span::raw("  "),
            Span::styled("Ctrl+C", Style::default().fg(self.theme.accent())),
            Span::styled(": Quit", Style::default().fg(self.theme.muted())),
        ]))
        .alignment(Alignment::Center);

        bindings.render(area, buf);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ========== format_number tests ==========

    #[test]
    fn test_format_number_zero() {
        assert_eq!(format_number(0), "0");
    }

    #[test]
    fn test_format_number_small() {
        assert_eq!(format_number(999), "999");
    }

    #[test]
    fn test_format_number_thousand() {
        assert_eq!(format_number(1000), "1,000");
    }

    #[test]
    fn test_format_number_large() {
        assert_eq!(format_number(1234567), "1,234,567");
    }

    #[test]
    fn test_format_number_million() {
        assert_eq!(format_number(1000000), "1,000,000");
    }

    #[test]
    fn test_unsupported_source_renders_disabled_row() {
        let total = TotalSummary::default();
        let daily: Vec<(NaiveDate, u64)> = vec![];
        let sources = vec![
            SourceUsage {
                source: "claude".into(),
                total_tokens: 100,
                total_cost_usd: 1.0,
                supported: true,
                estimated: false,
            },
            SourceUsage {
                source: "antigravity".into(),
                total_tokens: 0,
                total_cost_usd: 0.0,
                supported: false,
                estimated: false,
            },
        ];
        let data = OverviewData {
            total: &total,
            daily_tokens: &daily,
            source_usage: &sources,
            selected_source: None,
            selected_tab: Tab::Overview,
        };
        let area = Rect::new(0, 0, 120, 30);
        let mut buf = Buffer::empty(area);
        Overview::new(
            data,
            NaiveDate::from_ymd_opt(2026, 6, 1).unwrap(),
            Theme::Dark,
        )
        .render(area, &mut buf);

        let mut text = String::new();
        for y in area.y..area.y + area.height {
            for x in area.x..area.x + area.width {
                text.push_str(buf[(x, y)].symbol());
            }
        }
        assert!(
            text.contains("unsupported"),
            "overview should render the unsupported notice for disabled sources"
        );
    }

    #[test]
    fn test_estimated_source_renders_marker_and_legend() {
        let total = TotalSummary::default();
        let daily: Vec<(NaiveDate, u64)> = vec![];
        let sources = vec![SourceUsage {
            source: "gemini".into(),
            total_tokens: 500,
            total_cost_usd: 0.10,
            supported: true,
            estimated: true,
        }];
        let data = OverviewData {
            total: &total,
            daily_tokens: &daily,
            source_usage: &sources,
            selected_source: None,
            selected_tab: Tab::Overview,
        };
        let area = Rect::new(0, 0, 120, 30);
        let mut buf = Buffer::empty(area);
        Overview::new(
            data,
            NaiveDate::from_ymd_opt(2026, 6, 1).unwrap(),
            Theme::Dark,
        )
        .render(area, &mut buf);

        let mut text = String::new();
        for y in area.y..area.y + area.height {
            for x in area.x..area.x + area.width {
                text.push_str(buf[(x, y)].symbol());
            }
        }
        assert!(
            text.contains("estimated cost"),
            "overview should render the estimated-cost legend"
        );
    }
}
