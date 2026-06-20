//! Projects view widget - displays per-project usage statistics with a
//! selectable list that drills down into a per-day breakdown.

use std::collections::HashMap;

use ratatui::{
    buffer::Buffer,
    layout::{Alignment, Constraint, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Paragraph, Widget},
};

use super::models::format_percentage_bar;
use super::overview::format_number;
use super::tabs::{Tab, TabBar};
use crate::tui::theme::Theme;
use crate::types::{ProjectUsage, NO_PROJECT};

/// Turn a raw project identifier (the session working directory for Claude
/// Code) into a compact display label: the last two path segments, e.g.
/// `Scripts/toktrack`. The `(no project)` sentinel is shown verbatim.
pub fn project_display_name(key: &str) -> String {
    if key == NO_PROJECT {
        return key.to_string();
    }
    let parts: Vec<&str> = key.split(['/', '\\']).filter(|s| !s.is_empty()).collect();
    match parts.as_slice() {
        [] => key.to_string(),
        [only] => (*only).to_string(),
        [.., parent, leaf] => format!("{}/{}", parent, leaf),
    }
}

/// A single project's rolled-up usage for display (pre-sorted).
#[derive(Debug, Clone)]
pub struct ProjectSummary {
    /// Raw project identifier (working directory), used to look up drill-down data.
    pub key: String,
    /// Compact label shown to the user.
    pub display: String,
    pub total_tokens: u64,
    pub cost_usd: f64,
}

/// Data for the projects view.
#[derive(Debug, Default)]
pub struct ProjectsData {
    /// Projects sorted by cost descending.
    pub projects: Vec<ProjectSummary>,
    /// Total cost across all projects (for the usage bar).
    pub total_cost: f64,
}

impl ProjectsData {
    /// Build from `Aggregator::by_project_from_daily()` output.
    pub fn from_project_usage(project_map: &HashMap<String, ProjectUsage>) -> Self {
        let total_cost: f64 = project_map.values().map(|p| p.cost_usd).sum();

        let mut projects: Vec<ProjectSummary> = project_map
            .iter()
            .map(|(key, usage)| ProjectSummary {
                key: key.clone(),
                display: project_display_name(key),
                total_tokens: usage.total_tokens(),
                cost_usd: usage.cost_usd,
            })
            .filter(|p| p.total_tokens > 0)
            .collect();

        // Sort by cost descending (NaN-safe), tie-break by tokens.
        projects.sort_by(|a, b| {
            b.cost_usd
                .partial_cmp(&a.cost_usd)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then(b.total_tokens.cmp(&a.total_tokens))
        });

        Self {
            projects,
            total_cost,
        }
    }
}

/// Maximum content width (consistent with the Models view).
const MAX_CONTENT_WIDTH: u16 = 170;

/// Table width: marker(2) + Project(34) + Tokens(18) + Cost(12) + Usage(16) = 82
const TABLE_WIDTH: u16 = 82;

/// Projects view widget.
pub struct ProjectsView<'a> {
    data: &'a ProjectsData,
    selected: Option<usize>,
    theme: Theme,
    tab: Tab,
}

impl<'a> ProjectsView<'a> {
    pub fn new(data: &'a ProjectsData, selected: Option<usize>, theme: Theme) -> Self {
        Self {
            data,
            selected,
            theme,
            tab: Tab::Projects,
        }
    }

    pub fn with_tab(mut self, tab: Tab) -> Self {
        self.tab = tab;
        self
    }
}

impl Widget for ProjectsView<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let content_width = area.width.min(MAX_CONTENT_WIDTH);
        let x_offset = (area.width.saturating_sub(content_width)) / 2;
        let centered_area = Rect {
            x: area.x + x_offset,
            y: area.y,
            width: content_width,
            height: area.height,
        };

        let chunks = Layout::vertical([
            Constraint::Length(1), // 0: Top padding
            Constraint::Length(1), // 1: Tabs
            Constraint::Length(1), // 2: Separator
            Constraint::Length(1), // 3: Header
            Constraint::Fill(1),   // 4: Project rows
            Constraint::Length(1), // 5: Separator
            Constraint::Length(1), // 6: Keybindings
        ])
        .split(centered_area);

        TabBar::new(self.tab, self.theme).render(chunks[1], buf);
        self.render_separator(chunks[2], buf);
        self.render_header(chunks[3], buf);
        self.render_projects(chunks[4], buf);
        self.render_separator(chunks[5], buf);
        self.render_keybindings(chunks[6], buf);
    }
}

impl ProjectsView<'_> {
    fn table_offset(&self, area_width: u16) -> u16 {
        area_width.saturating_sub(TABLE_WIDTH) / 2
    }

    fn render_separator(&self, area: Rect, buf: &mut Buffer) {
        let line = "─".repeat(area.width as usize);
        buf.set_string(
            area.x,
            area.y,
            &line,
            Style::default().fg(self.theme.muted()),
        );
    }

    fn render_header(&self, area: Rect, buf: &mut Buffer) {
        let offset = self.table_offset(area.width);
        let bold = Style::default()
            .fg(self.theme.text())
            .add_modifier(Modifier::BOLD);
        let header = Line::from(vec![
            Span::styled(format!("{:<2}", ""), bold),
            Span::styled(format!("{:<34}", "Project"), bold),
            Span::styled(format!("{:>18}", "Tokens"), bold),
            Span::styled(format!("{:>12}", "Cost"), bold),
            Span::styled(format!("{:>16}", "Usage"), bold),
        ]);
        Paragraph::new(header).alignment(Alignment::Left).render(
            Rect {
                x: area.x + offset,
                y: area.y,
                width: TABLE_WIDTH.min(area.width),
                height: area.height,
            },
            buf,
        );
    }

    fn render_projects(&self, area: Rect, buf: &mut Buffer) {
        let offset = self.table_offset(area.width);
        let height = area.height as usize;
        if height == 0 || self.data.projects.is_empty() {
            if self.data.projects.is_empty() {
                let msg = Paragraph::new(Line::from(Span::styled(
                    "No project data yet — usage from supported CLIs populates this view.",
                    Style::default().fg(self.theme.muted()),
                )))
                .alignment(Alignment::Center);
                msg.render(area, buf);
            }
            return;
        }

        // Auto-scroll so the selected row stays visible.
        let selected = self.selected.unwrap_or(0);
        let scroll = if selected >= height {
            selected + 1 - height
        } else {
            0
        };

        for (row, project) in self
            .data
            .projects
            .iter()
            .enumerate()
            .skip(scroll)
            .take(height)
        {
            let y = area.y + (row - scroll) as u16;
            let is_selected = self.selected == Some(row);

            let percent = if self.data.total_cost > 0.0 {
                (project.cost_usd / self.data.total_cost) * 100.0
            } else {
                0.0
            };
            let bar = format_percentage_bar(percent, 12);

            let name = if project.display.chars().count() > 32 {
                format!("{}…", project.display.chars().take(31).collect::<String>())
            } else {
                project.display.clone()
            };

            let marker = if is_selected { "▸ " } else { "  " };
            let name_style = if is_selected {
                Style::default()
                    .fg(self.theme.accent())
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(self.theme.accent())
            };

            let row_line = Line::from(vec![
                Span::styled(marker.to_string(), Style::default().fg(self.theme.accent())),
                Span::styled(format!("{:<34}", name), name_style),
                Span::styled(
                    format!("{:>18}", format_number(project.total_tokens)),
                    Style::default().fg(self.theme.text()),
                ),
                Span::styled(
                    format!("{:>12}", format!("${:.2}", project.cost_usd)),
                    Style::default().fg(self.theme.cost()),
                ),
                Span::styled(
                    format!("{:>16}", bar),
                    Style::default().fg(self.theme.bar()),
                ),
            ]);

            Paragraph::new(row_line).alignment(Alignment::Left).render(
                Rect {
                    x: area.x + offset,
                    y,
                    width: TABLE_WIDTH.min(area.width),
                    height: 1,
                },
                buf,
            );
        }
    }

    fn render_keybindings(&self, area: Rect, buf: &mut Buffer) {
        let bindings = Paragraph::new(Line::from(vec![
            Span::styled("↑↓", Style::default().fg(self.theme.accent())),
            Span::styled(": Select", Style::default().fg(self.theme.muted())),
            Span::raw("  "),
            Span::styled("Enter", Style::default().fg(self.theme.accent())),
            Span::styled(": Details", Style::default().fg(self.theme.muted())),
            Span::raw("  "),
            Span::styled("Tab", Style::default().fg(self.theme.accent())),
            Span::styled(": Switch view", Style::default().fg(self.theme.muted())),
            Span::raw("  "),
            Span::styled("?", Style::default().fg(self.theme.accent())),
            Span::styled(": Help", Style::default().fg(self.theme.muted())),
        ]))
        .alignment(Alignment::Center);
        bindings.render(area, buf);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn usage(cost: f64, input: u64) -> ProjectUsage {
        ProjectUsage {
            input_tokens: input,
            cost_usd: cost,
            count: 1,
            ..Default::default()
        }
    }

    #[test]
    fn test_project_display_name_two_segments() {
        // Forward slashes (POSIX-style).
        assert_eq!(project_display_name("/srv/work/alpha/beta"), "alpha/beta");
        // Backslashes (Windows-style) — synthetic placeholder, not a real path.
        assert_eq!(project_display_name("Z:\\work\\alpha\\beta"), "alpha/beta");
    }

    #[test]
    fn test_project_display_name_single_segment() {
        assert_eq!(project_display_name("/solo"), "solo");
        assert_eq!(project_display_name("solo"), "solo");
    }

    #[test]
    fn test_project_display_name_no_project_sentinel() {
        assert_eq!(project_display_name(NO_PROJECT), NO_PROJECT);
    }

    #[test]
    fn test_projects_data_sorted_by_cost_desc() {
        let mut map: HashMap<String, ProjectUsage> = HashMap::new();
        map.insert("/a/low".to_string(), usage(0.10, 100));
        map.insert("/a/high".to_string(), usage(0.90, 100));
        map.insert("/a/mid".to_string(), usage(0.50, 100));

        let data = ProjectsData::from_project_usage(&map);
        assert_eq!(data.projects.len(), 3);
        assert_eq!(data.projects[0].key, "/a/high");
        assert_eq!(data.projects[1].key, "/a/mid");
        assert_eq!(data.projects[2].key, "/a/low");
        assert!((data.total_cost - 1.50).abs() < f64::EPSILON);
    }

    #[test]
    fn test_projects_data_filters_zero_token() {
        let mut map: HashMap<String, ProjectUsage> = HashMap::new();
        map.insert("/a/empty".to_string(), usage(0.0, 0));
        map.insert("/a/real".to_string(), usage(0.10, 50));

        let data = ProjectsData::from_project_usage(&map);
        assert_eq!(data.projects.len(), 1);
        assert_eq!(data.projects[0].key, "/a/real");
    }

    #[test]
    fn test_projects_view_renders_without_panic() {
        let mut map: HashMap<String, ProjectUsage> = HashMap::new();
        map.insert("/srv/work/alpha/beta".to_string(), usage(1.0, 1000));
        let data = ProjectsData::from_project_usage(&map);
        let view = ProjectsView::new(&data, Some(0), Theme::Dark);
        let area = Rect::new(0, 0, 120, 20);
        let mut buf = Buffer::empty(area);
        view.render(area, &mut buf);

        let mut content = String::new();
        for y in 0..area.height {
            for x in 0..area.width {
                content.push_str(buf[(x, y)].symbol());
            }
        }
        assert!(content.contains("alpha/beta"));
        assert!(content.contains("Project"));
    }

    #[test]
    fn test_projects_view_empty_shows_hint() {
        let data = ProjectsData::default();
        let view = ProjectsView::new(&data, None, Theme::Dark);
        let area = Rect::new(0, 0, 120, 20);
        let mut buf = Buffer::empty(area);
        view.render(area, &mut buf);

        let mut content = String::new();
        for y in 0..area.height {
            for x in 0..area.width {
                content.push_str(buf[(x, y)].symbol());
            }
        }
        assert!(content.contains("No project data"));
    }
}
