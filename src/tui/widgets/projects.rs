//! Projects view widget - displays per-project usage statistics with a
//! selectable list that drills down into a per-day breakdown.

use std::collections::HashMap;

use chrono::{Datelike, Duration, NaiveDate};
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
use crate::types::{DailySummary, ProjectUsage, NO_PROJECT};

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

/// Time window the Projects list is scoped to.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum ProjectPeriod {
    Today,
    Week,
    Month,
    #[default]
    All,
}

impl ProjectPeriod {
    pub const ALL: [ProjectPeriod; 4] = [Self::Today, Self::Week, Self::Month, Self::All];

    pub fn key(self) -> char {
        match self {
            Self::Today => 'd',
            Self::Week => 'w',
            Self::Month => 'm',
            Self::All => 'a',
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Today => "Today",
            Self::Week => "This week",
            Self::Month => "This month",
            Self::All => "All time",
        }
    }

    /// First day included in the period, or `None` for all time. Weeks start
    /// on Sunday and months on the 1st, matching `Aggregator::weekly/monthly`
    /// so the totals line up with the drill-down tables.
    pub fn start(self, today: NaiveDate) -> Option<NaiveDate> {
        match self {
            Self::Today => Some(today),
            Self::Week => {
                Some(today - Duration::days(today.weekday().num_days_from_sunday() as i64))
            }
            Self::Month => today.with_day(1),
            Self::All => None,
        }
    }
}

/// Sum each project's days on or after `since` into a `ProjectUsage`, so a
/// period list can be built with `ProjectsData::from_project_usage`.
/// Projects with no days in range are left out.
pub fn usage_since(
    daily: &HashMap<String, Vec<DailySummary>>,
    since: NaiveDate,
) -> HashMap<String, ProjectUsage> {
    let mut out: HashMap<String, ProjectUsage> = HashMap::new();
    for (key, days) in daily {
        let mut in_range = days.iter().filter(|d| d.date >= since).peekable();
        if in_range.peek().is_none() {
            continue;
        }
        let usage = out.entry(key.clone()).or_default();
        for day in in_range {
            usage.input_tokens = usage.input_tokens.saturating_add(day.total_input_tokens);
            usage.output_tokens = usage.output_tokens.saturating_add(day.total_output_tokens);
            usage.cache_read_tokens = usage
                .cache_read_tokens
                .saturating_add(day.total_cache_read_tokens);
            usage.cache_creation_tokens = usage
                .cache_creation_tokens
                .saturating_add(day.total_cache_creation_tokens);
            usage.reasoning_tokens = usage
                .reasoning_tokens
                .saturating_add(day.total_reasoning_tokens);
            usage.cost_usd += day.total_cost_usd;
        }
    }
    out
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
    /// Starred as a root folder (subfolders are folded into it).
    pub is_root: bool,
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
    /// Build from `Aggregator::by_project_from_daily()` output (already folded
    /// onto `roots`); keys listed in `roots` are flagged as root rows.
    pub fn from_project_usage(
        project_map: &HashMap<String, ProjectUsage>,
        roots: &[String],
    ) -> Self {
        let total_cost: f64 = project_map.values().map(|p| p.cost_usd).sum();

        let mut projects: Vec<ProjectSummary> = project_map
            .iter()
            .map(|(key, usage)| ProjectSummary {
                key: key.clone(),
                display: project_display_name(key),
                total_tokens: usage.total_tokens(),
                cost_usd: usage.cost_usd,
                is_root: roots.iter().any(|root| root == key),
            })
            .filter(|p| p.total_tokens > 0)
            .collect();

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
    period: ProjectPeriod,
    /// Shown in place of the keybindings (e.g. a failed root-folder save).
    error: Option<&'a str>,
}

impl<'a> ProjectsView<'a> {
    pub fn new(data: &'a ProjectsData, selected: Option<usize>, theme: Theme) -> Self {
        Self {
            data,
            selected,
            theme,
            tab: Tab::Projects,
            period: ProjectPeriod::All,
            error: None,
        }
    }

    pub fn with_tab(mut self, tab: Tab) -> Self {
        self.tab = tab;
        self
    }

    pub fn with_period(mut self, period: ProjectPeriod) -> Self {
        self.period = period;
        self
    }

    pub fn with_error(mut self, error: Option<&'a str>) -> Self {
        self.error = error;
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
            Constraint::Length(1), // 0: Tabs
            Constraint::Length(1), // 1: Separator
            Constraint::Length(1), // 2: Period indicator
            Constraint::Length(1), // 3: Header
            Constraint::Fill(1),   // 4: Project rows
            Constraint::Length(1), // 5: Separator
            Constraint::Length(1), // 6: Keybindings
        ])
        .split(centered_area);

        TabBar::new(self.tab, self.theme).render(chunks[0], buf);
        self.render_separator(chunks[1], buf);
        self.render_period_indicator(chunks[2], buf);
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

    fn render_period_indicator(&self, area: Rect, buf: &mut Buffer) {
        let mut spans = Vec::new();
        for period in ProjectPeriod::ALL {
            if !spans.is_empty() {
                spans.push(Span::raw("  "));
            }
            let style = if period == self.period {
                Style::default()
                    .fg(self.theme.accent())
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(self.theme.text())
            };
            spans.push(Span::styled(
                format!("{}:{}", period.key(), period.label()),
                style,
            ));
        }
        spans.push(Span::styled(
            "  │  ",
            Style::default().fg(self.theme.muted()),
        ));
        spans.push(Span::styled(
            format!("${:.2}", self.data.total_cost),
            Style::default().fg(self.theme.cost()),
        ));
        Paragraph::new(Line::from(spans))
            .alignment(Alignment::Center)
            .render(area, buf);
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
                let text = match self.period {
                    ProjectPeriod::All => {
                        "No project data yet — usage from supported CLIs populates this view."
                    }
                    ProjectPeriod::Today => "No project usage today.",
                    ProjectPeriod::Week => "No project usage this week.",
                    ProjectPeriod::Month => "No project usage this month.",
                };
                let msg = Paragraph::new(Line::from(Span::styled(
                    text,
                    Style::default().fg(self.theme.muted()),
                )))
                .alignment(Alignment::Center);
                msg.render(area, buf);
            }
            return;
        }

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

            // Root rows reserve two columns of the name cell for " *". ASCII
            // rather than "★", which CJK terminals may draw two cells wide.
            let star = if project.is_root { " *" } else { "" };
            let max_name = 32 - star.chars().count();
            let name = if project.display.chars().count() > max_name {
                format!(
                    "{}…",
                    project
                        .display
                        .chars()
                        .take(max_name - 1)
                        .collect::<String>()
                )
            } else {
                project.display.clone()
            };
            let padding = 34 - name.chars().count() - star.chars().count();

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
                Span::styled(name, name_style),
                Span::styled(star.to_string(), Style::default().fg(self.theme.date())),
                Span::raw(" ".repeat(padding)),
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
        if let Some(error) = self.error {
            Paragraph::new(Line::from(Span::styled(
                format!("Could not save root folders: {}", error),
                Style::default().fg(self.theme.error()),
            )))
            .alignment(Alignment::Center)
            .render(area, buf);
            return;
        }
        let bindings = Paragraph::new(Line::from(vec![
            Span::styled("↑↓", Style::default().fg(self.theme.accent())),
            Span::styled(": Select", Style::default().fg(self.theme.muted())),
            Span::raw("  "),
            Span::styled("Enter", Style::default().fg(self.theme.accent())),
            Span::styled(": Details", Style::default().fg(self.theme.muted())),
            Span::raw("  "),
            Span::styled("*", Style::default().fg(self.theme.accent())),
            Span::styled(": Root", Style::default().fg(self.theme.muted())),
            Span::raw("  "),
            Span::styled("d/w/m/a", Style::default().fg(self.theme.accent())),
            Span::styled(": Period", Style::default().fg(self.theme.muted())),
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
        assert_eq!(project_display_name("/srv/work/alpha/beta"), "alpha/beta");
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

        let data = ProjectsData::from_project_usage(&map, &[]);
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

        let data = ProjectsData::from_project_usage(&map, &[]);
        assert_eq!(data.projects.len(), 1);
        assert_eq!(data.projects[0].key, "/a/real");
    }

    #[test]
    fn test_projects_view_renders_without_panic() {
        let mut map: HashMap<String, ProjectUsage> = HashMap::new();
        map.insert("/srv/work/alpha/beta".to_string(), usage(1.0, 1000));
        let data = ProjectsData::from_project_usage(&map, &[]);
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
    fn test_projects_data_flags_roots() {
        let mut map: HashMap<String, ProjectUsage> = HashMap::new();
        map.insert("/w/repo".to_string(), usage(1.0, 100));
        map.insert("/w/other".to_string(), usage(0.5, 100));

        let data = ProjectsData::from_project_usage(&map, &["/w/repo".to_string()]);
        assert!(data.projects[0].is_root);
        assert!(!data.projects[1].is_root);
    }

    #[test]
    fn test_projects_view_renders_star_on_root() {
        let mut map: HashMap<String, ProjectUsage> = HashMap::new();
        map.insert("/w/repo".to_string(), usage(1.0, 100));
        map.insert("/w/other".to_string(), usage(0.5, 100));
        let data = ProjectsData::from_project_usage(&map, &["/w/repo".to_string()]);
        let view = ProjectsView::new(&data, Some(0), Theme::Dark);
        let area = Rect::new(0, 0, 120, 20);
        let mut buf = Buffer::empty(area);
        view.render(area, &mut buf);

        let rows: Vec<String> = (0..area.height)
            .map(|y| (0..area.width).map(|x| buf[(x, y)].symbol()).collect())
            .collect();
        let repo = rows.iter().find(|r| r.contains("w/repo")).unwrap();
        let other = rows.iter().find(|r| r.contains("w/other")).unwrap();
        assert!(repo.contains("repo *"));
        assert!(!other.contains("other *"));
        // The star must not shift the numeric columns.
        let dollar_col = |row: &str| row.chars().position(|c| c == '$');
        assert_eq!(dollar_col(repo), dollar_col(other));
    }

    #[test]
    fn test_projects_view_shows_save_error_in_footer() {
        let data = ProjectsData::default();
        let view = ProjectsView::new(&data, None, Theme::Dark).with_error(Some("disk full"));
        let area = Rect::new(0, 0, 120, 20);
        let mut buf = Buffer::empty(area);
        view.render(area, &mut buf);

        let footer: String = (0..area.width)
            .map(|x| buf[(x, area.height - 1)].symbol())
            .collect();
        assert!(footer.contains("disk full"));
    }

    fn oct(day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 10, day).unwrap()
    }

    fn day_summary(date: NaiveDate, input: u64, cost: f64) -> DailySummary {
        DailySummary {
            date,
            total_input_tokens: input,
            total_output_tokens: 0,
            total_cache_read_tokens: 0,
            total_cache_creation_tokens: 0,
            total_reasoning_tokens: 0,
            total_cache_creation_5m_tokens: 0,
            total_cache_creation_1h_tokens: 0,
            total_web_search_requests: 0,
            total_cost_usd: cost,
            models: HashMap::new(),
            projects: HashMap::new(),
        }
    }

    fn render_rows(view: ProjectsView<'_>) -> Vec<String> {
        let area = Rect::new(0, 0, 120, 20);
        let mut buf = Buffer::empty(area);
        view.render(area, &mut buf);
        (0..area.height)
            .map(|y| (0..area.width).map(|x| buf[(x, y)].symbol()).collect())
            .collect()
    }

    #[test]
    fn test_project_period_start_dates() {
        // 2026-10-30 is a Friday; weeks start on Sunday like Aggregator::weekly.
        let today = oct(30);
        assert_eq!(ProjectPeriod::Today.start(today), Some(oct(30)));
        assert_eq!(ProjectPeriod::Week.start(today), Some(oct(25)));
        assert_eq!(ProjectPeriod::Month.start(today), Some(oct(1)));
        assert_eq!(ProjectPeriod::All.start(today), None);
        // A Sunday starts its own week.
        assert_eq!(ProjectPeriod::Week.start(oct(25)), Some(oct(25)));
    }

    #[test]
    fn test_project_period_default_is_all() {
        assert_eq!(ProjectPeriod::default(), ProjectPeriod::All);
    }

    #[test]
    fn test_usage_since_sums_days_on_or_after_start() {
        let mut daily: HashMap<String, Vec<DailySummary>> = HashMap::new();
        daily.insert(
            "/w/a".to_string(),
            vec![
                day_summary(oct(24), 1000, 1.0),
                day_summary(oct(25), 100, 0.25),
                day_summary(oct(30), 10, 0.5),
            ],
        );
        daily.insert("/w/old".to_string(), vec![day_summary(oct(1), 50, 2.0)]);

        let usage = usage_since(&daily, oct(25));
        assert_eq!(usage.len(), 1);
        let a = usage.get("/w/a").unwrap();
        assert_eq!(a.total_tokens(), 110);
        assert!((a.cost_usd - 0.75).abs() < 1e-9);
    }

    #[test]
    fn test_projects_view_shows_period_indicator_and_total() {
        let mut map: HashMap<String, ProjectUsage> = HashMap::new();
        map.insert("/w/repo".to_string(), usage(1.5, 100));
        let data = ProjectsData::from_project_usage(&map, &[]);
        let rows = render_rows(
            ProjectsView::new(&data, Some(0), Theme::Dark).with_period(ProjectPeriod::Week),
        );
        let indicator = rows.iter().find(|r| r.contains("w:This week")).unwrap();
        assert!(indicator.contains("d:Today"));
        assert!(indicator.contains("m:This month"));
        assert!(indicator.contains("a:All time"));
        assert!(indicator.contains("$1.50"));
    }

    #[test]
    fn test_projects_view_empty_period_message() {
        let data = ProjectsData::default();
        let rows = render_rows(
            ProjectsView::new(&data, None, Theme::Dark).with_period(ProjectPeriod::Today),
        );
        assert!(rows.iter().any(|r| r.contains("No project usage today")));
    }

    #[test]
    fn test_projects_view_footer_lists_period_keys() {
        let data = ProjectsData::default();
        let rows = render_rows(ProjectsView::new(&data, None, Theme::Dark));
        assert!(rows.last().unwrap().contains("d/w/m/a"));
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
