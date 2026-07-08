//! Application state and event loop

use std::collections::HashMap;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use chrono::{Local, NaiveDate};
use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use ratatui::{
    buffer::Buffer, layout::Rect, style::Style, widgets::Widget, DefaultTerminal, Frame,
};

use super::theme::Theme;

use crate::parsers::ParserRegistry;
use crate::services::audit::{self, AuditReport};
use crate::services::update_checker::{check_for_update, execute_update, UpdateCheckResult};
use crate::services::{
    Aggregator, DailySummaryCacheService, DataLoaderService, RemoteOptions, RemoteSourceService,
};
use crate::types::{CacheWarning, DailySummary, SourceUsage, StatsData, TotalSummary};

use super::widgets::{
    daily::{DailyData, DailyView, DailyViewMode, SortDirection, SortKey},
    help::HelpPopup,
    model_breakdown::{ModelBreakdownPopup, ModelBreakdownState},
    models::ModelsData,
    overview::{Overview, OverviewData},
    projects::{project_display_name, ProjectsData, ProjectsView},
    quit_confirm::{QuitConfirmPopup, QuitConfirmState},
    source_detail::SourceDetailView,
    spinner::{LoadingStage, Spinner},
    stats::StatsView,
    tabs::Tab,
    update_popup::{DimOverlay, UpdateMessagePopup, UpdatePopup},
};

/// Current view mode
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ViewMode {
    Dashboard {
        tab: Tab,
    },
    SourceDetail {
        source: String,
    },
    /// Drill-down into a single project's per-day breakdown. `project` holds the
    /// raw project identifier (working directory) used to look up its data.
    ProjectDetail {
        project: String,
    },
}

impl Default for ViewMode {
    fn default() -> Self {
        Self::Dashboard { tab: Tab::Overview }
    }
}

/// Configuration for TUI startup
#[derive(Debug, Clone, Default)]
pub struct TuiConfig {
    pub initial_view_mode: DailyViewMode,
    pub initial_tab: Option<Tab>,
    pub remote_options: RemoteOptions,
}

/// Application state
pub enum AppState {
    /// Loading data with spinner animation
    Loading {
        spinner_frame: usize,
        stage: LoadingStage,
    },
    /// Ready with loaded data
    Ready { data: Box<AppData> },
    /// Error state
    Error { message: String },
}

/// Loaded application data
pub struct AppData {
    pub total: TotalSummary,
    pub daily_tokens: Vec<(NaiveDate, u64)>,
    pub models_data: ModelsData,
    pub daily_data: DailyData,
    pub stats_data: StatsData,
    /// Usage breakdown by source CLI
    pub source_usage: Vec<SourceUsage>,
    /// Per-source daily data
    pub source_daily_data: HashMap<String, DailyData>,
    /// Per-source models data
    #[allow(dead_code)] // Reserved for future per-source models view
    pub source_models_data: HashMap<String, ModelsData>,
    /// Per-source stats data
    pub source_stats_data: HashMap<String, StatsData>,
    /// High-level per-project usage list (Projects tab).
    pub projects_data: ProjectsData,
    /// Per-project daily data, keyed by raw project identifier (drill-down).
    pub project_daily_data: HashMap<String, DailyData>,
    /// Per-project stats data, keyed by raw project identifier (drill-down).
    pub project_stats_data: HashMap<String, StatsData>,
    /// Cache warning indicator for display in TUI
    #[allow(dead_code)] // Reserved for warning indicator feature
    pub cache_warning: Option<CacheWarning>,
}

impl AppData {
    /// Apply the sort to every daily table: the aggregate one plus the
    /// per-source and per-project drill-down tables, so the shared sort state
    /// holds no matter which drill-down is opened next.
    fn apply_sort(&mut self, key: SortKey, direction: SortDirection) {
        self.daily_data.apply_sort(key, direction);
        for daily in self.source_daily_data.values_mut() {
            daily.apply_sort(key, direction);
        }
        for daily in self.project_daily_data.values_mut() {
            daily.apply_sort(key, direction);
        }
    }
}

/// Update overlay status
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpdateStatus {
    /// Background check in progress
    Checking,
    /// Update available, showing overlay
    Available { current: String, latest: String },
    /// User confirmed update, transitioning to background thread
    Updating,
    /// Background thread running npm update
    UpdateRunning,
    /// Update finished (success or failure)
    UpdateDone { success: bool, message: String },
    /// Resolved (no overlay)
    Resolved,
}

impl UpdateStatus {
    /// Whether the update overlay is currently displayed
    pub fn shows_overlay(&self) -> bool {
        matches!(
            self,
            UpdateStatus::Available { .. }
                | UpdateStatus::Updating
                | UpdateStatus::UpdateRunning
                | UpdateStatus::UpdateDone { .. }
        )
    }
}

/// Data loading function — injectable so tests never touch the real filesystem.
type DataLoader = fn(RemoteOptions) -> Result<Box<AppData>, String>;

/// Main application
pub struct App {
    state: AppState,
    should_quit: bool,
    view_mode: ViewMode,
    source_selected: usize,
    /// Selected row in the Projects tab list.
    project_selected: usize,
    daily_scroll: usize,
    weekly_scroll: usize,
    monthly_scroll: usize,
    daily_selected: Option<usize>,
    weekly_selected: Option<usize>,
    monthly_selected: Option<usize>,
    daily_view_mode: DailyViewMode,
    /// Sort state shared by all daily tables (same pattern as daily_view_mode).
    sort_key: SortKey,
    sort_direction: SortDirection,
    show_help: bool,
    update_status: UpdateStatus,
    update_selection: u8, // 0 = Update now, 1 = Skip
    pending_data: Option<Result<Box<AppData>, String>>,
    theme: Theme,
    quit_confirm: Option<QuitConfirmState>,
    model_breakdown: Option<ModelBreakdownState>,
    terminal_height: u16,
    /// Remote options retained for the on-demand audit computation.
    remote_options: RemoteOptions,
    /// Lazily-computed data-preservation audit (Audit tab).
    audit: Option<AuditReport>,
    /// Receiver for the background audit computation, if one is in flight.
    audit_rx: Option<mpsc::Receiver<std::result::Result<AuditReport, String>>>,
    /// Error from the audit computation, if it failed.
    audit_error: Option<String>,
    /// Receiver for the in-flight background data load (startup or manual refresh).
    data_rx: Option<mpsc::Receiver<std::result::Result<Box<AppData>, String>>>,
    /// True while a manual refresh runs with the previous data still on screen.
    refreshing: bool,
    /// Loader executed on background data loads.
    data_loader: DataLoader,
}

impl App {
    /// Create a new app in loading state with the given configuration
    pub fn new(config: TuiConfig, theme: Theme) -> Self {
        Self {
            state: AppState::Loading {
                spinner_frame: 0,
                stage: LoadingStage::Scanning,
            },
            should_quit: false,
            view_mode: ViewMode::Dashboard {
                tab: config.initial_tab.unwrap_or_default(),
            },
            source_selected: 0,
            project_selected: 0,
            daily_scroll: 0,
            weekly_scroll: 0,
            monthly_scroll: 0,
            daily_selected: None,
            weekly_selected: None,
            monthly_selected: None,
            daily_view_mode: config.initial_view_mode,
            sort_key: SortKey::default(),
            sort_direction: SortDirection::default(),
            show_help: false,
            update_status: UpdateStatus::Checking,
            update_selection: 0,
            pending_data: None,
            theme,
            quit_confirm: None,
            model_breakdown: None,
            terminal_height: 24,
            remote_options: config.remote_options.clone(),
            audit: None,
            audit_rx: None,
            audit_error: None,
            data_rx: None,
            refreshing: false,
            data_loader: load_data_sync,
        }
    }

    /// Kick off a background data load (startup load or manual refresh via 'r').
    /// No-op while a load is already in flight.
    pub fn start_data_load(&mut self) {
        if self.data_rx.is_some() {
            return;
        }
        match self.state {
            // Keep the current data on screen during a manual refresh.
            AppState::Ready { .. } => self.refreshing = true,
            // Retry from the error screen goes back to the loading spinner.
            AppState::Error { .. } => {
                self.state = AppState::Loading {
                    spinner_frame: 0,
                    stage: LoadingStage::Scanning,
                }
            }
            AppState::Loading { .. } => {}
        }
        let loader = self.data_loader;
        let remote_options = self.remote_options.clone();
        let (tx, rx) = mpsc::channel();
        thread::spawn(move || {
            let _ = tx.send(loader(remote_options));
        });
        self.data_rx = Some(rx);
    }

    /// Poll the background data load without blocking the event loop.
    pub fn poll_data(&mut self) {
        let Some(rx) = &self.data_rx else {
            return;
        };
        match rx.try_recv() {
            Ok(result) => {
                self.data_rx = None;
                self.refreshing = false;
                if self.update_status.shows_overlay() {
                    // Overlay is active, store data for later
                    self.pending_data = Some(result);
                } else {
                    self.apply_data_result(result);
                }
            }
            // Sender dropped without sending (worker thread panicked). A failed
            // refresh keeps the current data; only a load with nothing on
            // screen surfaces the error.
            Err(mpsc::TryRecvError::Disconnected) => {
                self.data_rx = None;
                self.refreshing = false;
                if !matches!(self.state, AppState::Ready { .. }) {
                    self.state = AppState::Error {
                        message: "Data loading failed unexpectedly".to_string(),
                    };
                }
            }
            Err(mpsc::TryRecvError::Empty) => {}
        }
    }

    /// Calculate the effective number of visible rows based on the current view and terminal height.
    /// SourceDetail/ProjectDetail fixed overhead: src-header(1) + stats(1) + sep(1) + mode(1) + table-header(1) + sep(1) + keybindings(1) = 7
    /// Dashboard renders no daily table, so the 6 only pre-seeds `daily_scroll` in
    /// `apply_data_result` (recomputed as 7 on drill-in). It approximates the default
    /// Overview top chrome: tabs(1) + sep(1) + hero(3) + sub-stats(1) = 6.
    fn effective_visible_rows(&self) -> usize {
        let overhead: u16 = match &self.view_mode {
            ViewMode::SourceDetail { .. } | ViewMode::ProjectDetail { .. } => 7,
            ViewMode::Dashboard { .. } => 6,
        };
        self.terminal_height.saturating_sub(overhead) as usize
    }

    /// Get scroll offset for the current daily view mode
    fn active_scroll(&self) -> usize {
        match self.daily_view_mode {
            DailyViewMode::Daily => self.daily_scroll,
            DailyViewMode::Weekly => self.weekly_scroll,
            DailyViewMode::Monthly => self.monthly_scroll,
        }
    }

    /// Get mutable reference to scroll offset for the current daily view mode
    fn active_scroll_mut(&mut self) -> &mut usize {
        match self.daily_view_mode {
            DailyViewMode::Daily => &mut self.daily_scroll,
            DailyViewMode::Weekly => &mut self.weekly_scroll,
            DailyViewMode::Monthly => &mut self.monthly_scroll,
        }
    }

    /// Get selected index for the current daily view mode
    fn active_selected(&self) -> Option<usize> {
        match self.daily_view_mode {
            DailyViewMode::Daily => self.daily_selected,
            DailyViewMode::Weekly => self.weekly_selected,
            DailyViewMode::Monthly => self.monthly_selected,
        }
    }

    /// Get mutable reference to selected index for the current daily view mode
    fn active_selected_mut(&mut self) -> &mut Option<usize> {
        match self.daily_view_mode {
            DailyViewMode::Daily => &mut self.daily_selected,
            DailyViewMode::Weekly => &mut self.weekly_selected,
            DailyViewMode::Monthly => &mut self.monthly_selected,
        }
    }

    /// Handle keyboard and resize events
    pub fn handle_event(&mut self, event: Event) {
        if let Event::Resize(_, h) = event {
            self.terminal_height = h;
            return;
        }
        if let Event::Key(key) = event {
            if key.kind == KeyEventKind::Press {
                // Ctrl+C shows quit confirmation
                if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
                    self.quit_confirm = Some(QuitConfirmState::new());
                    return;
                }

                match &self.view_mode {
                    ViewMode::Dashboard { .. } => self.handle_dashboard_event(key.code),
                    ViewMode::SourceDetail { .. } | ViewMode::ProjectDetail { .. } => {
                        self.handle_detail_event(key.code)
                    }
                }
            }
        }
    }

    /// Get the current dashboard tab
    fn current_tab(&self) -> Tab {
        match &self.view_mode {
            ViewMode::Dashboard { tab } => *tab,
            _ => Tab::Overview,
        }
    }

    /// Set the current dashboard tab
    fn set_tab(&mut self, tab: Tab) {
        self.view_mode = ViewMode::Dashboard { tab };
        if tab == Tab::Audit {
            self.ensure_audit_loading();
        }
    }

    /// Kick off the background audit computation the first time the Audit tab
    /// is opened. The audit needs a full raw parse, so it must never run on the
    /// startup hot path — only on demand.
    fn ensure_audit_loading(&mut self) {
        if self.audit.is_some() || self.audit_rx.is_some() {
            return;
        }
        self.audit_error = None;
        let remote_options = self.remote_options.clone();
        let (tx, rx) = mpsc::channel();
        thread::spawn(move || {
            let _ = tx.send(compute_audit(remote_options));
        });
        self.audit_rx = Some(rx);
    }

    /// Poll the background audit thread without blocking the event loop.
    fn poll_audit(&mut self) {
        let Some(rx) = &self.audit_rx else {
            return;
        };
        match rx.try_recv() {
            Ok(Ok(report)) => {
                self.audit = Some(report);
                self.audit_rx = None;
            }
            Ok(Err(e)) => {
                self.audit_error = Some(e);
                self.audit_rx = None;
            }
            // Sender dropped without sending (e.g. the worker thread panicked) —
            // surface an error instead of spinning on "Computing…" forever.
            Err(mpsc::TryRecvError::Disconnected) => {
                self.audit_error = Some("Audit computation failed unexpectedly".to_string());
                self.audit_rx = None;
            }
            Err(mpsc::TryRecvError::Empty) => {}
        }
    }

    /// Handle keyboard events in Dashboard mode
    fn handle_dashboard_event(&mut self, code: KeyCode) {
        // Common keys for all tabs
        match code {
            KeyCode::Esc => {
                if self.show_help {
                    self.show_help = false;
                }
                return;
            }
            KeyCode::Tab | KeyCode::BackTab => {
                let tab = self.current_tab();
                let next = if code == KeyCode::Tab {
                    tab.next()
                } else {
                    tab.prev()
                };
                self.set_tab(next);
                return;
            }
            KeyCode::Char('1') => {
                if let Some(tab) = Tab::from_number(1) {
                    self.set_tab(tab);
                }
                return;
            }
            KeyCode::Char('2') => {
                if let Some(tab) = Tab::from_number(2) {
                    self.set_tab(tab);
                }
                return;
            }
            KeyCode::Char('3') => {
                if let Some(tab) = Tab::from_number(3) {
                    self.set_tab(tab);
                }
                return;
            }
            KeyCode::Char('4') => {
                if let Some(tab) = Tab::from_number(4) {
                    self.set_tab(tab);
                }
                return;
            }
            KeyCode::Char('5') => {
                if let Some(tab) = Tab::from_number(5) {
                    self.set_tab(tab);
                }
                return;
            }
            KeyCode::Char('?') => {
                self.show_help = !self.show_help;
                return;
            }
            KeyCode::Char('r') => {
                self.start_data_load();
                return;
            }
            _ => {}
        }

        // Tab-specific keys
        match self.current_tab() {
            Tab::Overview => match code {
                KeyCode::Up | KeyCode::Char('k') if self.source_selected > 0 => {
                    self.source_selected -= 1;
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    if let AppState::Ready { data } = &self.state {
                        let max = data.source_usage.len().saturating_sub(1);
                        if self.source_selected < max {
                            self.source_selected += 1;
                        }
                    }
                }
                KeyCode::Enter => {
                    if let AppState::Ready { data } = &self.state {
                        if let Some(source) = data.source_usage.get(self.source_selected) {
                            self.view_mode = ViewMode::SourceDetail {
                                source: source.source.clone(),
                            };
                            self.reset_detail_selection_and_scroll();
                        }
                    }
                }
                _ => {}
            },
            Tab::Projects => match code {
                KeyCode::Up | KeyCode::Char('k') if self.project_selected > 0 => {
                    self.project_selected -= 1;
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    if let AppState::Ready { data } = &self.state {
                        let max = data.projects_data.projects.len().saturating_sub(1);
                        if self.project_selected < max {
                            self.project_selected += 1;
                        }
                    }
                }
                KeyCode::Enter => self.open_project_detail(),
                _ => {}
            },
            Tab::Stats | Tab::Models | Tab::Audit => {
                // These tabs have no additional keys beyond the common ones.
            }
        }
    }

    /// Enter the per-project drill-down for the currently selected project.
    fn open_project_detail(&mut self) {
        let AppState::Ready { data } = &self.state else {
            return;
        };
        let Some(project) = data.projects_data.projects.get(self.project_selected) else {
            return;
        };
        let key = project.key.clone();

        self.view_mode = ViewMode::ProjectDetail { project: key };
        self.reset_detail_selection_and_scroll();
    }

    /// Handle keyboard events in a drill-down detail view (SourceDetail or
    /// ProjectDetail). Both share the same daily-table interaction.
    fn handle_detail_event(&mut self, code: KeyCode) {
        match code {
            KeyCode::Esc => {
                if self.show_help {
                    self.show_help = false;
                } else {
                    // Return to the tab the drill-down was entered from.
                    let tab = match &self.view_mode {
                        ViewMode::ProjectDetail { .. } => Tab::Projects,
                        _ => Tab::Overview,
                    };
                    self.view_mode = ViewMode::Dashboard { tab };
                }
            }
            KeyCode::Up | KeyCode::Char('k') => {
                self.select_prev();
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.select_next();
            }
            KeyCode::Enter => {
                self.open_model_breakdown();
            }
            KeyCode::Char('d') => {
                self.daily_view_mode = DailyViewMode::Daily;
            }
            KeyCode::Char('w') => {
                self.daily_view_mode = DailyViewMode::Weekly;
            }
            KeyCode::Char('m') => {
                self.daily_view_mode = DailyViewMode::Monthly;
            }
            KeyCode::Char('s') => {
                self.sort_key = self.sort_key.next();
                self.sort_direction = self.sort_key.default_direction();
                self.apply_sort_and_reset();
            }
            KeyCode::Char('S') => {
                self.sort_direction = self.sort_direction.reversed();
                self.apply_sort_and_reset();
            }
            KeyCode::Char('?') => {
                self.show_help = !self.show_help;
            }
            KeyCode::Char('r') => {
                self.start_data_load();
            }
            _ => {}
        }
    }

    /// Re-sort all loaded daily tables to the current sort state, then reset
    /// selection/scroll as if the view was freshly opened — re-ordering the
    /// data makes the index-based selection and scroll offsets stale.
    fn apply_sort_and_reset(&mut self) {
        if let AppState::Ready { data } = &mut self.state {
            data.apply_sort(self.sort_key, self.sort_direction);
        }
        self.reset_detail_selection_and_scroll();
    }

    /// Whether the current sort keeps the historical date-ascending order,
    /// where the most relevant rows (latest days) sit at the bottom. Every
    /// other sort ranks its most relevant rows at the top.
    fn sort_anchors_to_latest(&self) -> bool {
        self.sort_key == SortKey::Date && self.sort_direction == SortDirection::Asc
    }

    /// Row where a first selection (no row selected yet) lands: the same
    /// anchor the scroll reset uses, so the cursor appears where the view
    /// is already looking.
    fn initial_selection_index(&self, count: usize) -> usize {
        if self.sort_anchors_to_latest() {
            count.saturating_sub(1)
        } else {
            0
        }
    }

    /// Clear row selection and recompute scroll for all three period modes.
    /// Date-ascending keeps the historical behavior of jumping to the latest
    /// entries (bottom); any other sort starts at the top of its ranking.
    fn reset_detail_selection_and_scroll(&mut self) {
        self.daily_selected = None;
        self.weekly_selected = None;
        self.monthly_selected = None;

        let (daily, weekly, monthly) = match &self.state {
            AppState::Ready { data } if self.sort_anchors_to_latest() => {
                let vr = self.effective_visible_rows();
                let daily_data = self.active_daily_data(data);
                (
                    DailyView::max_scroll_offset(daily_data, DailyViewMode::Daily, vr),
                    DailyView::max_scroll_offset(daily_data, DailyViewMode::Weekly, vr),
                    DailyView::max_scroll_offset(daily_data, DailyViewMode::Monthly, vr),
                )
            }
            _ => (0, 0, 0),
        };
        self.daily_scroll = daily;
        self.weekly_scroll = weekly;
        self.monthly_scroll = monthly;
    }

    /// Handle keyboard events when quit confirm overlay is displayed
    pub fn handle_quit_confirm_event(&mut self, event: Event) {
        if let Event::Key(key) = event {
            if key.kind == KeyEventKind::Press {
                match key.code {
                    // Arrow keys toggle selection
                    KeyCode::Left | KeyCode::Right | KeyCode::Up | KeyCode::Down => {
                        if let Some(ref mut state) = self.quit_confirm {
                            state.selection = 1 - state.selection;
                        }
                    }
                    // Enter confirms the selection
                    KeyCode::Enter => {
                        if let Some(ref state) = self.quit_confirm {
                            if state.selection == 0 {
                                // Yes selected -> quit
                                self.should_quit = true;
                            }
                        }
                        self.quit_confirm = None;
                    }
                    // Esc or 'n' cancels
                    KeyCode::Esc | KeyCode::Char('n') | KeyCode::Char('N') => {
                        self.quit_confirm = None;
                    }
                    // 'y' quits immediately
                    KeyCode::Char('y') | KeyCode::Char('Y') => {
                        self.should_quit = true;
                        self.quit_confirm = None;
                    }
                    _ => {}
                }
            }
        }
    }

    /// Handle keyboard events when model breakdown popup is displayed
    pub fn handle_model_breakdown_event(&mut self, event: Event) {
        if let Event::Key(key) = event {
            if key.kind == KeyEventKind::Press {
                match key.code {
                    KeyCode::Esc | KeyCode::Enter | KeyCode::Char('q') => {
                        self.model_breakdown = None;
                    }
                    _ => {}
                }
            }
        }
    }

    /// Handle keyboard events when update overlay is displayed
    pub fn handle_update_event(&mut self, event: Event) {
        if let Event::Key(key) = event {
            if key.kind == KeyEventKind::Press {
                match (&self.update_status, key.code) {
                    // Available state: up/down to select, Enter to confirm, Esc to dismiss
                    (UpdateStatus::Available { .. }, KeyCode::Up | KeyCode::Down) => {
                        self.update_selection = 1 - self.update_selection;
                    }
                    (UpdateStatus::Available { .. }, KeyCode::Enter) => {
                        if self.update_selection == 0 {
                            self.update_status = UpdateStatus::Updating;
                        } else {
                            self.update_status = UpdateStatus::Resolved;
                            self.consume_pending_data();
                        }
                    }
                    // Esc dismisses update overlay (skip update)
                    (UpdateStatus::Available { .. }, KeyCode::Esc) => {
                        self.update_status = UpdateStatus::Resolved;
                        self.consume_pending_data();
                    }
                    // UpdateDone state: any key dismisses
                    (UpdateStatus::UpdateDone { success, .. }, _) => {
                        if *success {
                            self.should_quit = true;
                        } else {
                            self.update_status = UpdateStatus::Resolved;
                            self.consume_pending_data();
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    /// Consume pending data if available, transitioning to Ready state
    fn consume_pending_data(&mut self) {
        if let Some(result) = self.pending_data.take() {
            self.apply_data_result(result);
        }
    }

    /// Apply data loading result to app state
    fn apply_data_result(&mut self, result: Result<Box<AppData>, String>) {
        match result {
            Ok(mut data) => {
                // Fresh loads arrive date-ascending; re-apply the user's sort.
                data.apply_sort(self.sort_key, self.sort_direction);
                self.state = AppState::Ready { data };
                self.reset_detail_selection_and_scroll();
                // Fresh data invalidates any previously computed audit.
                self.audit = None;
                self.audit_error = None;
                self.audit_rx = None;
                if self.current_tab() == Tab::Audit {
                    self.ensure_audit_loading();
                }
            }
            Err(message) => {
                // Graceful degrade: a failed refresh keeps the previous data on
                // screen; only a load with nothing to show surfaces the error.
                if !matches!(self.state, AppState::Ready { .. }) {
                    self.state = AppState::Error { message };
                }
            }
        }
    }

    /// Get the active DailyData depending on the current view mode
    fn active_daily_data<'a>(&self, data: &'a AppData) -> &'a DailyData {
        match &self.view_mode {
            ViewMode::SourceDetail { source } => data
                .source_daily_data
                .get(source)
                .unwrap_or(&data.daily_data),
            ViewMode::ProjectDetail { project } => data
                .project_daily_data
                .get(project)
                .unwrap_or(&data.daily_data),
            ViewMode::Dashboard { .. } => &data.daily_data,
        }
    }

    /// Whether the current view is a drill-down detail (source or project).
    fn is_detail_view(&self) -> bool {
        matches!(
            self.view_mode,
            ViewMode::SourceDetail { .. } | ViewMode::ProjectDetail { .. }
        )
    }

    /// Select previous row (move up) in a detail view
    fn select_prev(&mut self) {
        if !self.is_detail_view() {
            return;
        }

        let count = match &self.state {
            AppState::Ready { data } => {
                let daily_data = self.active_daily_data(data);
                let (summaries, _) = daily_data.for_mode(self.daily_view_mode);
                summaries.len()
            }
            _ => return,
        };

        if count == 0 {
            return;
        }

        let current = self.active_selected();
        let new_idx = match current {
            None => self.initial_selection_index(count),
            Some(0) => 0,
            Some(idx) => idx.saturating_sub(1),
        };
        *self.active_selected_mut() = Some(new_idx);

        self.adjust_scroll_for_selection();
    }

    /// Select next row (move down) in a detail view
    fn select_next(&mut self) {
        if !self.is_detail_view() {
            return;
        }

        let count = match &self.state {
            AppState::Ready { data } => {
                let daily_data = self.active_daily_data(data);
                let (summaries, _) = daily_data.for_mode(self.daily_view_mode);
                summaries.len()
            }
            _ => return,
        };

        if count == 0 {
            return;
        }

        let max_idx = count.saturating_sub(1);

        let current = self.active_selected();
        let new_idx = match current {
            None => self.initial_selection_index(count),
            Some(idx) if idx >= max_idx => max_idx,
            Some(idx) => idx + 1,
        };
        *self.active_selected_mut() = Some(new_idx);

        self.adjust_scroll_for_selection();
    }

    /// Adjust scroll offset to keep the current selection visible
    fn adjust_scroll_for_selection(&mut self) {
        let visible_rows = self.effective_visible_rows();

        let selected = match self.active_selected() {
            Some(idx) => idx,
            None => return,
        };

        let scroll = self.active_scroll();

        if selected < scroll {
            *self.active_scroll_mut() = selected;
        } else if selected >= scroll + visible_rows {
            *self.active_scroll_mut() = selected.saturating_sub(visible_rows - 1);
        }
    }

    /// Open model breakdown popup for the currently selected row
    fn open_model_breakdown(&mut self) {
        if !self.is_detail_view() {
            return;
        }
        let selected = match self.active_selected() {
            Some(idx) => idx,
            None => return,
        };

        if let AppState::Ready { data } = &self.state {
            let daily_data = self.active_daily_data(data);
            let (summaries, _) = daily_data.for_mode(self.daily_view_mode);
            if let Some(summary) = summaries.get(selected) {
                let date_label = match self.daily_view_mode {
                    DailyViewMode::Daily | DailyViewMode::Weekly => {
                        summary.date.format("%Y-%m-%d").to_string()
                    }
                    DailyViewMode::Monthly => summary.date.format("%Y-%m").to_string(),
                };

                let models: Vec<_> = summary
                    .models
                    .iter()
                    .map(|(k, v)| (k.clone(), v.clone()))
                    .collect();

                self.model_breakdown = Some(ModelBreakdownState::new(date_label, models));
            }
        }
    }

    /// Update spinner animation
    pub fn tick(&mut self) {
        if let AppState::Loading {
            spinner_frame,
            stage,
        } = &self.state
        {
            self.state = AppState::Loading {
                spinner_frame: Spinner::next_frame(*spinner_frame),
                stage: *stage,
            };
        }
    }

    /// Check if app should quit
    pub fn should_quit(&self) -> bool {
        self.should_quit
    }

    /// Draw the application
    pub fn draw(&self, frame: &mut Frame) {
        frame.render_widget(self, frame.area());
    }
}

impl Default for App {
    fn default() -> Self {
        Self::new(TuiConfig::default(), Theme::default())
    }
}

impl Widget for &App {
    fn render(self, area: Rect, buf: &mut Buffer) {
        match &self.state {
            AppState::Loading {
                spinner_frame,
                stage,
            } => {
                let spinner = Spinner::new(*spinner_frame, *stage, self.theme);
                spinner.render(area, buf);
            }
            AppState::Ready { data } => {
                match &self.view_mode {
                    ViewMode::Dashboard { tab } => match tab {
                        Tab::Overview => {
                            let today = Local::now().date_naive();
                            let overview_data = OverviewData {
                                total: &data.total,
                                daily_tokens: &data.daily_tokens,
                                source_usage: &data.source_usage,
                                selected_source: Some(self.source_selected),
                                selected_tab: *tab,
                            };
                            let overview = Overview::new(overview_data, today, self.theme);
                            overview.render(area, buf);
                        }
                        Tab::Stats => {
                            let stats_view =
                                StatsView::new(&data.stats_data, self.theme).with_tab(*tab);
                            stats_view.render(area, buf);
                        }
                        Tab::Models => {
                            let models_view = super::widgets::models::ModelsView::new(
                                &data.models_data,
                                self.theme,
                            )
                            .with_tab(*tab);
                            models_view.render(area, buf);
                        }
                        Tab::Projects => {
                            ProjectsView::new(
                                &data.projects_data,
                                Some(self.project_selected),
                                self.theme,
                            )
                            .with_tab(*tab)
                            .render(area, buf);
                        }
                        Tab::Audit => {
                            super::widgets::audit::AuditView::new(
                                self.audit.as_ref(),
                                self.audit_error.as_deref(),
                                self.theme,
                            )
                            .render(area, buf);
                        }
                    },
                    ViewMode::SourceDetail { source } => {
                        let daily_data = data
                            .source_daily_data
                            .get(source)
                            .unwrap_or(&data.daily_data);
                        let stats_data = data
                            .source_stats_data
                            .get(source)
                            .unwrap_or(&data.stats_data);
                        let source_detail = SourceDetailView::new(
                            source,
                            daily_data,
                            stats_data,
                            self.active_scroll(),
                            self.daily_view_mode,
                            self.active_selected(),
                            self.theme,
                        )
                        .with_sort(self.sort_key, self.sort_direction);
                        source_detail.render(area, buf);
                    }
                    ViewMode::ProjectDetail { project } => {
                        let daily_data = data
                            .project_daily_data
                            .get(project)
                            .unwrap_or(&data.daily_data);
                        let stats_data = data
                            .project_stats_data
                            .get(project)
                            .unwrap_or(&data.stats_data);
                        let label = project_display_name(project);
                        let project_detail = SourceDetailView::new(
                            &label,
                            daily_data,
                            stats_data,
                            self.active_scroll(),
                            self.daily_view_mode,
                            self.active_selected(),
                            self.theme,
                        )
                        .with_sort(self.sort_key, self.sort_direction);
                        project_detail.render(area, buf);
                    }
                }

                // Manual-refresh indicator (top-right corner, above the tab bar)
                if self.refreshing {
                    let text = "⟳ Refreshing…";
                    let w = text.chars().count() as u16;
                    if area.width > w + 1 {
                        let x = area.x + area.width - w - 1;
                        buf.set_string(x, area.y, text, Style::default().fg(self.theme.muted()));
                    }
                }

                // Render help popup overlay if active
                if self.show_help {
                    let popup_area = HelpPopup::centered_area(area);
                    HelpPopup::new(self.theme).render(popup_area, buf);
                }

                // Render model breakdown popup if active
                if let Some(ref state) = self.model_breakdown {
                    DimOverlay.render(area, buf);
                    let popup_area = ModelBreakdownPopup::centered_area(area, state.models.len());
                    ModelBreakdownPopup::new(state, self.theme).render(popup_area, buf);
                }
            }
            AppState::Error { message } => {
                let y = area.y + area.height / 2;
                let text = format!("Error: {}", message);
                let x = area.x + (area.width.saturating_sub(text.len() as u16)) / 2;
                buf.set_string(x, y, &text, Style::default().fg(self.theme.error()));
            }
        }

        // Render update overlay on top of everything (works in both Loading and Ready states)
        match &self.update_status {
            UpdateStatus::Available { current, latest } => {
                DimOverlay.render(area, buf);
                let popup_area = UpdatePopup::centered_area(area);
                UpdatePopup::new(current, latest, self.update_selection, self.theme)
                    .render(popup_area, buf);
            }
            UpdateStatus::Updating | UpdateStatus::UpdateRunning => {
                DimOverlay.render(area, buf);
                let popup_area = UpdateMessagePopup::centered_area(area);
                UpdateMessagePopup::new("Running npm update -g toktrack...", self.theme.date())
                    .render(popup_area, buf);
            }
            UpdateStatus::UpdateDone { success, message } => {
                DimOverlay.render(area, buf);
                let popup_area = UpdateMessagePopup::centered_area(area);
                let color = if *success {
                    self.theme.bar()
                } else {
                    self.theme.error()
                };
                UpdateMessagePopup::new(message, color).render(popup_area, buf);
            }
            UpdateStatus::Checking | UpdateStatus::Resolved => {}
        }

        // Render quit confirm overlay (highest z-index, above everything including update overlay)
        if let Some(ref state) = self.quit_confirm {
            DimOverlay.render(area, buf);
            let popup_area = QuitConfirmPopup::centered_area(area);
            QuitConfirmPopup::new(state.selection, self.theme).render(popup_area, buf);
        }
    }
}

/// Run the TUI application with the given configuration
pub fn run(config: TuiConfig) -> anyhow::Result<()> {
    // Detect theme before entering raw mode (escape-sequence detection needs normal stdin)
    let theme = Theme::detect();
    let mut terminal = ratatui::init();
    let result = run_app(&mut terminal, config, theme);
    ratatui::restore();
    result
}

/// Compute the data-preservation audit (runs on a background thread when the
/// Audit tab is first opened — never on the startup hot path).
fn compute_audit(remote_options: RemoteOptions) -> Result<AuditReport, String> {
    let extra_sources =
        RemoteSourceService::sync_and_build_sources(&remote_options).map_err(|e| e.to_string())?;
    let registry = ParserRegistry::with_extra_sources(extra_sources);
    let cache = DailySummaryCacheService::new().map_err(|e| e.to_string())?;
    Ok(audit::build_report(
        registry.sources(),
        &cache,
        Local::now().date_naive(),
    ))
}

/// Load data synchronously (extracted for background thread).
/// Uses cache-first strategy via DataLoaderService.
fn load_data_sync(remote_options: RemoteOptions) -> Result<Box<AppData>, String> {
    let extra_sources =
        RemoteSourceService::sync_and_build_sources(&remote_options).map_err(|e| e.to_string())?;
    let result = DataLoaderService::with_extra_sources(extra_sources)
        .load()
        .map_err(|e| e.to_string())?;

    build_app_data_from_summaries(
        result.summaries,
        result.source_usage,
        result.source_summaries,
        result.cache_warning,
    )
}

/// Build AppData from DailySummary list (no raw entries needed).
fn build_app_data_from_summaries(
    summaries: Vec<DailySummary>,
    source_usage: Vec<SourceUsage>,
    source_summaries: HashMap<String, Vec<DailySummary>>,
    cache_warning: Option<CacheWarning>,
) -> Result<Box<AppData>, String> {
    let total = Aggregator::total_from_daily(&summaries);

    let daily_tokens: Vec<(NaiveDate, u64)> = summaries
        .iter()
        .map(|d| {
            (
                d.date,
                d.total_input_tokens
                    + d.total_output_tokens
                    + d.total_cache_read_tokens
                    + d.total_cache_creation_tokens
                    + d.total_reasoning_tokens,
            )
        })
        .collect();

    let model_map = Aggregator::by_model_from_daily(&summaries);
    let models_data = ModelsData::from_model_usage(&model_map);
    let stats_data = StatsData::from_daily_summaries(&summaries);

    // Build per-project data (high-level list + per-project drill-down) before
    // `summaries` is moved into the daily view below.
    let project_map = Aggregator::by_project_from_daily(&summaries);
    let projects_data = ProjectsData::from_project_usage(&project_map);
    let project_summaries = Aggregator::project_daily_summaries(&summaries);
    let mut project_daily_data = HashMap::new();
    let mut project_stats_data = HashMap::new();
    for (project, summ) in &project_summaries {
        project_daily_data.insert(
            project.clone(),
            DailyData::from_daily_summaries(summ.clone()),
        );
        project_stats_data.insert(project.clone(), StatsData::from_daily_summaries(summ));
    }

    let daily_data = DailyData::from_daily_summaries(summaries);

    // Build per-source data
    let mut source_daily_data = HashMap::new();
    let mut source_models_data = HashMap::new();
    let mut source_stats_data = HashMap::new();

    for (source_name, src_summaries) in &source_summaries {
        let src_model_map = Aggregator::by_model_from_daily(src_summaries);
        source_daily_data.insert(
            source_name.clone(),
            DailyData::from_daily_summaries(src_summaries.clone()),
        );
        source_models_data.insert(
            source_name.clone(),
            ModelsData::from_model_usage(&src_model_map),
        );
        source_stats_data.insert(
            source_name.clone(),
            StatsData::from_daily_summaries(src_summaries),
        );
    }

    Ok(Box::new(AppData {
        total,
        daily_tokens,
        models_data,
        daily_data,
        stats_data,
        source_usage,
        source_daily_data,
        source_models_data,
        source_stats_data,
        projects_data,
        project_daily_data,
        project_stats_data,
        cache_warning,
    }))
}

fn run_app(terminal: &mut DefaultTerminal, config: TuiConfig, theme: Theme) -> anyhow::Result<()> {
    let mut app = App::new(config, theme);
    app.terminal_height = terminal.size()?.height;

    // Kick off the startup data load on a background thread
    app.start_data_load();

    // Spawn background thread for update check
    let (update_tx, update_rx) = mpsc::channel();
    thread::spawn(move || {
        let result = check_for_update();
        let _ = update_tx.send(result);
    });

    // Channel for async execute_update result
    let (execute_tx, execute_rx) = mpsc::channel();

    loop {
        terminal.draw(|frame| app.draw(frame))?;

        if app.should_quit() {
            break;
        }

        // Check for data loading completion (non-blocking)
        app.poll_data();

        // Check for on-demand audit completion (non-blocking)
        app.poll_audit();

        // Check for update check completion (non-blocking)
        if app.update_status == UpdateStatus::Checking {
            if let Ok(result) = update_rx.try_recv() {
                match result {
                    UpdateCheckResult::UpdateAvailable { current, latest } => {
                        app.update_status = UpdateStatus::Available { current, latest };
                    }
                    UpdateCheckResult::UpToDate | UpdateCheckResult::CheckFailed => {
                        app.update_status = UpdateStatus::Resolved;
                    }
                }
            }
        }

        // Handle Updating state: spawn background thread for npm update
        if app.update_status == UpdateStatus::Updating {
            app.update_status = UpdateStatus::UpdateRunning;
            let tx = execute_tx.clone();
            thread::spawn(move || {
                let result = execute_update();
                let _ = tx.send(result);
            });
        }

        // Check for execute_update completion (non-blocking)
        if app.update_status == UpdateStatus::UpdateRunning {
            if let Ok(result) = execute_rx.try_recv() {
                match result {
                    Ok(()) => {
                        app.update_status = UpdateStatus::UpdateDone {
                            success: true,
                            message: "Updated! Press any key to exit.".to_string(),
                        };
                    }
                    Err(e) => {
                        app.update_status = UpdateStatus::UpdateDone {
                            success: false,
                            message: format!("Failed: {}", e),
                        };
                    }
                }
            }
        }

        // Poll for events with 100ms timeout for spinner animation
        if event::poll(Duration::from_millis(100))? {
            let ev = event::read()?;
            // Priority chain: quit_confirm > model_breakdown > update > main
            if app.quit_confirm.is_some() {
                app.handle_quit_confirm_event(ev);
            } else if app.model_breakdown.is_some() {
                app.handle_model_breakdown_event(ev);
            } else if app.update_status.shows_overlay() {
                app.handle_update_event(ev);
            } else {
                app.handle_event(ev);
            }
        } else {
            app.tick();
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use std::collections::HashMap;

    /// Helper to create a ready app with minimal data for testing
    fn make_ready_app() -> App {
        use crate::types::DailySummary;
        use chrono::NaiveDate;

        let summaries: Vec<DailySummary> = (1..=20)
            .map(|d| DailySummary {
                date: NaiveDate::from_ymd_opt(2025, 1, d).unwrap(),
                total_input_tokens: 100,
                total_output_tokens: 50,
                total_cache_read_tokens: 0,
                total_cache_creation_tokens: 0,
                total_reasoning_tokens: 0,
                total_cache_creation_5m_tokens: 0,
                total_cache_creation_1h_tokens: 0,
                total_web_search_requests: 0,
                total_cost_usd: 0.01,
                models: HashMap::new(),
                projects: HashMap::new(),
            })
            .collect();

        let daily_tokens: Vec<(NaiveDate, u64)> = summaries.iter().map(|d| (d.date, 150)).collect();

        let daily_data = DailyData::from_daily_summaries(summaries.clone());
        let stats_data = crate::types::StatsData::from_daily_summaries(&summaries);
        let models_data = super::ModelsData::from_model_usage(&HashMap::new());

        let mut app = App::default();
        let vr = app.effective_visible_rows();
        let daily_scroll = DailyView::max_scroll_offset(&daily_data, DailyViewMode::Daily, vr);
        let weekly_scroll = DailyView::max_scroll_offset(&daily_data, DailyViewMode::Weekly, vr);
        let monthly_scroll = DailyView::max_scroll_offset(&daily_data, DailyViewMode::Monthly, vr);

        app.state = AppState::Ready {
            data: Box::new(AppData {
                total: crate::types::TotalSummary::default(),
                daily_tokens,
                models_data,
                daily_data,
                stats_data,
                source_usage: vec![SourceUsage {
                    source: "claude".to_string(),
                    total_tokens: 3000,
                    total_cost_usd: 0.20,
                    supported: true,
                    estimated: false,
                }],
                source_daily_data: HashMap::new(),
                source_models_data: HashMap::new(),
                source_stats_data: HashMap::new(),
                projects_data: Default::default(),
                project_daily_data: HashMap::new(),
                project_stats_data: HashMap::new(),
                cache_warning: None,
            }),
        };
        app.daily_scroll = daily_scroll;
        app.weekly_scroll = weekly_scroll;
        app.monthly_scroll = monthly_scroll;
        app
    }

    #[test]
    fn test_app_initial_state() {
        let app = App::default();
        assert!(matches!(
            app.state,
            AppState::Loading {
                spinner_frame: 0,
                stage: LoadingStage::Scanning
            }
        ));
        assert!(!app.should_quit());
    }

    #[test]
    fn test_q_key_does_nothing() {
        let mut app = App::default();
        assert!(!app.should_quit());
        assert!(app.quit_confirm.is_none());

        let event = Event::Key(KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE));
        app.handle_event(event);

        assert!(!app.should_quit());
        assert!(app.quit_confirm.is_none());
    }

    #[test]
    fn test_esc_closes_help_popup() {
        let mut app = App {
            show_help: true,
            ..App::default()
        };

        let event = Event::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        app.handle_event(event);

        assert!(!app.show_help);
        assert!(app.quit_confirm.is_none());
        assert!(!app.should_quit());
    }

    #[test]
    fn test_esc_does_nothing_when_no_popup_dashboard() {
        let mut app = App::default();
        assert!(!app.show_help);

        let event = Event::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        app.handle_event(event);

        assert!(!app.show_help);
        assert!(app.quit_confirm.is_none());
        assert!(!app.should_quit());
    }

    #[test]
    fn test_app_tick_updates_spinner() {
        let mut app = App::default();
        assert!(matches!(
            app.state,
            AppState::Loading {
                spinner_frame: 0,
                ..
            }
        ));

        app.tick();
        assert!(matches!(
            app.state,
            AppState::Loading {
                spinner_frame: 1,
                ..
            }
        ));
    }

    #[test]
    fn test_view_mode_starts_dashboard() {
        let app = App::default();
        assert!(matches!(app.view_mode, ViewMode::Dashboard { .. }));
    }

    #[test]
    fn test_enter_navigates_to_source_detail() {
        let mut app = make_ready_app();
        // source_selected defaults to 0, source_usage has "claude"
        let event = Event::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        app.handle_event(event);

        assert_eq!(
            app.view_mode,
            ViewMode::SourceDetail {
                source: "claude".to_string()
            }
        );
    }

    #[test]
    fn test_esc_returns_to_dashboard() {
        let mut app = make_ready_app();
        app.view_mode = ViewMode::SourceDetail {
            source: "claude".to_string(),
        };

        let event = Event::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        app.handle_event(event);

        assert!(matches!(app.view_mode, ViewMode::Dashboard { .. }));
    }

    #[test]
    fn test_source_selection_up_down() {
        let mut app = make_ready_app();
        // Add a second source
        if let AppState::Ready { data } = &mut app.state {
            data.source_usage.push(SourceUsage {
                source: "opencode".to_string(),
                total_tokens: 1000,
                total_cost_usd: 0.05,
                supported: true,
                estimated: false,
            });
        }

        assert_eq!(app.source_selected, 0);

        // Down
        app.handle_event(Event::Key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE)));
        assert_eq!(app.source_selected, 1);

        // Down again should stay at 1 (max)
        app.handle_event(Event::Key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE)));
        assert_eq!(app.source_selected, 1);

        // Up
        app.handle_event(Event::Key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE)));
        assert_eq!(app.source_selected, 0);

        // Up again should stay at 0
        app.handle_event(Event::Key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE)));
        assert_eq!(app.source_selected, 0);
    }

    #[test]
    fn test_app_help_toggle() {
        let mut app = App::default();
        assert!(!app.show_help);

        let event = Event::Key(KeyEvent::new(KeyCode::Char('?'), KeyModifiers::NONE));
        app.handle_event(event.clone());
        assert!(app.show_help);

        app.handle_event(event);
        assert!(!app.show_help);
    }

    #[test]
    fn test_d_w_m_keys_in_source_detail() {
        let mut app = make_ready_app();
        app.view_mode = ViewMode::SourceDetail {
            source: "claude".to_string(),
        };
        assert_eq!(app.daily_view_mode, DailyViewMode::Daily);

        app.handle_event(Event::Key(KeyEvent::new(
            KeyCode::Char('w'),
            KeyModifiers::NONE,
        )));
        assert_eq!(app.daily_view_mode, DailyViewMode::Weekly);

        app.handle_event(Event::Key(KeyEvent::new(
            KeyCode::Char('m'),
            KeyModifiers::NONE,
        )));
        assert_eq!(app.daily_view_mode, DailyViewMode::Monthly);

        app.handle_event(Event::Key(KeyEvent::new(
            KeyCode::Char('d'),
            KeyModifiers::NONE,
        )));
        assert_eq!(app.daily_view_mode, DailyViewMode::Daily);
    }

    #[test]
    fn test_d_w_m_keys_ignored_on_dashboard() {
        let mut app = make_ready_app();
        assert!(matches!(app.view_mode, ViewMode::Dashboard { .. }));
        assert_eq!(app.daily_view_mode, DailyViewMode::Daily);

        app.handle_event(Event::Key(KeyEvent::new(
            KeyCode::Char('w'),
            KeyModifiers::NONE,
        )));
        assert_eq!(app.daily_view_mode, DailyViewMode::Daily);
    }

    // ========== Sort toggle tests (issue #206) ==========

    fn jan(day: u32) -> chrono::NaiveDate {
        chrono::NaiveDate::from_ymd_opt(2025, 1, day).unwrap()
    }

    /// Daily summary with the given day of Jan 2025, token counts, and cost.
    fn sort_test_summary(day: u32, input: u64, cache_read: u64, cost: f64) -> DailySummary {
        DailySummary {
            date: jan(day),
            total_input_tokens: input,
            total_output_tokens: 0,
            total_cache_read_tokens: cache_read,
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

    /// Ready app inside the source detail view whose 20-day daily data has
    /// distinct costs and token totals: Jan 5 is the most expensive and Jan 7
    /// has the most tokens (cache-heavy), so cost, tokens, and date orders
    /// all differ from each other.
    fn make_sort_test_app() -> App {
        let summaries: Vec<DailySummary> = (1..=20)
            .map(|d| {
                let cost = if d == 5 { 99.0 } else { d as f64 * 0.01 };
                let cache_read = if d == 7 { 1_000_000 } else { 0 };
                sort_test_summary(d, 100 + d as u64, cache_read, cost)
            })
            .collect();

        let mut app = make_ready_app();
        if let AppState::Ready { data } = &mut app.state {
            data.daily_data = DailyData::from_daily_summaries(summaries);
        }
        app.view_mode = ViewMode::SourceDetail {
            source: "claude".to_string(),
        };
        app
    }

    fn press(app: &mut App, code: KeyCode) {
        app.handle_event(Event::Key(KeyEvent::new(code, KeyModifiers::NONE)));
    }

    /// Date of the first row of the daily table in its current order.
    fn first_daily_date(app: &App) -> chrono::NaiveDate {
        match &app.state {
            AppState::Ready { data } => data.daily_data.daily_summaries[0].date,
            _ => panic!("expected Ready state"),
        }
    }

    #[test]
    fn test_s_key_cycles_sort_in_detail() {
        let mut app = make_sort_test_app();
        assert_eq!(app.sort_key, SortKey::Date);
        assert_eq!(app.sort_direction, SortDirection::Asc);

        press(&mut app, KeyCode::Char('s'));
        assert_eq!(app.sort_key, SortKey::Cost);
        assert_eq!(app.sort_direction, SortDirection::Desc);

        press(&mut app, KeyCode::Char('s'));
        assert_eq!(app.sort_key, SortKey::Tokens);
        assert_eq!(app.sort_direction, SortDirection::Desc);

        press(&mut app, KeyCode::Char('s'));
        assert_eq!(app.sort_key, SortKey::Date);
        assert_eq!(app.sort_direction, SortDirection::Asc);
    }

    #[test]
    fn test_s_key_resorts_data_and_resets_selection_scroll() {
        let mut app = make_sort_test_app();
        app.daily_selected = Some(3);
        app.weekly_selected = Some(1);

        press(&mut app, KeyCode::Char('s')); // cost descending

        assert_eq!(first_daily_date(&app), jan(5)); // most expensive first
        assert_eq!(app.daily_selected, None);
        assert_eq!(app.weekly_selected, None);
        assert_eq!(app.monthly_selected, None);
        // The ranking starts at the top, not at the latest date.
        assert_eq!(app.daily_scroll, 0);
        assert_eq!(app.weekly_scroll, 0);
        assert_eq!(app.monthly_scroll, 0);
    }

    #[test]
    fn test_s_key_tokens_sort_uses_token_totals() {
        let mut app = make_sort_test_app();

        press(&mut app, KeyCode::Char('s'));
        press(&mut app, KeyCode::Char('s')); // tokens descending

        assert_eq!(first_daily_date(&app), jan(7)); // cache-heavy day first
    }

    #[test]
    fn test_shift_s_reverses_sort_direction() {
        let mut app = make_sort_test_app();

        press(&mut app, KeyCode::Char('S')); // date descending
        assert_eq!(app.sort_key, SortKey::Date);
        assert_eq!(app.sort_direction, SortDirection::Desc);
        assert_eq!(first_daily_date(&app), jan(20)); // newest first
        assert_eq!(app.daily_scroll, 0);

        press(&mut app, KeyCode::Char('S')); // back to ascending
        assert_eq!(app.sort_direction, SortDirection::Asc);
        assert_eq!(first_daily_date(&app), jan(1));
    }

    #[test]
    fn test_sort_keys_ignored_on_dashboard() {
        let mut app = make_ready_app();
        assert!(matches!(app.view_mode, ViewMode::Dashboard { .. }));

        press(&mut app, KeyCode::Char('s'));
        press(&mut app, KeyCode::Char('S'));

        assert_eq!(app.sort_key, SortKey::Date);
        assert_eq!(app.sort_direction, SortDirection::Asc);
    }

    #[test]
    fn test_sort_cycle_back_to_date_restores_bottom_scroll() {
        let mut app = make_sort_test_app();

        press(&mut app, KeyCode::Char('s'));
        press(&mut app, KeyCode::Char('s'));
        press(&mut app, KeyCode::Char('s')); // back to date ascending

        let expected = match &app.state {
            AppState::Ready { data } => DailyView::max_scroll_offset(
                &data.daily_data,
                DailyViewMode::Daily,
                app.effective_visible_rows(),
            ),
            _ => panic!("expected Ready state"),
        };
        assert!(expected > 0, "fixture must be taller than the viewport");
        assert_eq!(app.daily_scroll, expected); // latest days visible again
    }

    #[test]
    fn test_sort_persists_across_detail_views() {
        let mut app = make_sort_test_app();
        press(&mut app, KeyCode::Char('s')); // cost descending

        press(&mut app, KeyCode::Esc); // back to the dashboard
        assert!(matches!(app.view_mode, ViewMode::Dashboard { .. }));
        assert_eq!(app.sort_key, SortKey::Cost);

        press(&mut app, KeyCode::Enter); // re-enter the claude drill-down
        assert!(matches!(app.view_mode, ViewMode::SourceDetail { .. }));
        assert_eq!(first_daily_date(&app), jan(5)); // still sorted by cost
        assert_eq!(app.daily_selected, None);
        assert_eq!(app.daily_scroll, 0); // entry respects the active sort
    }

    #[test]
    fn test_refresh_reapplies_current_sort() {
        let mut app = make_sort_test_app();
        press(&mut app, KeyCode::Char('s')); // cost descending

        // A fresh load arrives date-ascending, like real loads do.
        let mut fresh = marker_app_data(1);
        fresh.daily_data = DailyData::from_daily_summaries(vec![
            sort_test_summary(1, 100, 0, 0.10),
            sort_test_summary(2, 100, 0, 5.00),
            sort_test_summary(3, 100, 0, 1.00),
        ]);
        let (tx, rx) = mpsc::channel();
        app.data_rx = Some(rx);
        app.refreshing = true;

        tx.send(Ok(fresh)).unwrap();
        app.poll_data();

        assert_eq!(first_daily_date(&app), jan(2)); // most expensive first
        assert_eq!(app.daily_selected, None);
        assert_eq!(app.daily_scroll, 0);
    }

    #[test]
    fn test_first_selection_starts_at_bottom_for_date_asc() {
        let mut app = make_sort_test_app();

        press(&mut app, KeyCode::Char('j')); // default date-ascending order

        assert_eq!(app.daily_selected, Some(19)); // latest day (bottom row)
    }

    #[test]
    fn test_first_selection_starts_at_top_when_sorted() {
        let mut app = make_sort_test_app();
        press(&mut app, KeyCode::Char('s')); // cost descending, anchored to the top

        press(&mut app, KeyCode::Char('j'));

        assert_eq!(app.daily_selected, Some(0)); // most expensive day (top row)
        assert_eq!(app.daily_scroll, 0); // view must not jump to the bottom
    }

    #[test]
    fn test_first_selection_starts_at_top_for_date_desc() {
        let mut app = make_sort_test_app();
        press(&mut app, KeyCode::Char('S')); // newest-first

        press(&mut app, KeyCode::Char('k'));

        assert_eq!(app.daily_selected, Some(0)); // today (top row)
        assert_eq!(app.daily_scroll, 0);
    }

    #[test]
    fn test_enter_opens_breakdown_for_sorted_row() {
        let mut app = make_sort_test_app();
        press(&mut app, KeyCode::Char('s')); // cost descending: Jan 5 is priciest, first

        press(&mut app, KeyCode::Char('j')); // first press selects the top row
        press(&mut app, KeyCode::Enter);

        let breakdown = app.model_breakdown.expect("breakdown popup must open");
        assert_eq!(breakdown.date_label, "2025-01-05");
    }

    // ========== Update overlay tests ==========

    #[test]
    fn test_app_initial_update_status() {
        let app = App::default();
        assert_eq!(app.update_status, UpdateStatus::Checking);
        assert!(app.pending_data.is_none());
    }

    fn make_update_available_app() -> App {
        App {
            update_status: UpdateStatus::Available {
                current: "0.1.14".to_string(),
                latest: "0.2.0".to_string(),
            },
            ..App::default()
        }
    }

    #[test]
    fn test_update_overlay_skip_via_selection() {
        let mut app = make_update_available_app();

        let down = Event::Key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
        app.handle_update_event(down);
        assert_eq!(app.update_selection, 1);

        let enter = Event::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        app.handle_update_event(enter);

        assert_eq!(app.update_status, UpdateStatus::Resolved);
        assert!(!app.should_quit());
    }

    #[test]
    fn test_update_overlay_enter_triggers_update() {
        let mut app = make_update_available_app();

        assert_eq!(app.update_selection, 0);
        let enter = Event::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        app.handle_update_event(enter);

        assert_eq!(app.update_status, UpdateStatus::Updating);
    }

    #[test]
    fn test_update_overlay_arrow_toggles_selection() {
        let mut app = make_update_available_app();
        assert_eq!(app.update_selection, 0);

        let down = Event::Key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
        app.handle_update_event(down);
        assert_eq!(app.update_selection, 1);

        let up = Event::Key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE));
        app.handle_update_event(up);
        assert_eq!(app.update_selection, 0);
    }

    #[test]
    fn test_update_overlay_esc_dismisses() {
        let mut app = make_update_available_app();

        let event = Event::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        app.handle_update_event(event);

        assert!(!app.should_quit());
        assert_eq!(app.update_status, UpdateStatus::Resolved);
    }

    #[test]
    fn test_pending_data_consumed_on_skip() {
        use crate::types::DailySummary;
        use chrono::NaiveDate;

        let mut app = make_update_available_app();

        let summaries: Vec<DailySummary> = vec![DailySummary {
            date: NaiveDate::from_ymd_opt(2025, 1, 1).unwrap(),
            total_input_tokens: 100,
            total_output_tokens: 50,
            total_cache_read_tokens: 0,
            total_cache_creation_tokens: 0,
            total_reasoning_tokens: 0,
            total_cache_creation_5m_tokens: 0,
            total_cache_creation_1h_tokens: 0,
            total_web_search_requests: 0,
            total_cost_usd: 0.01,
            models: HashMap::new(),
            projects: HashMap::new(),
        }];
        let daily_tokens: Vec<(NaiveDate, u64)> = vec![(summaries[0].date, 150)];
        let daily_data = DailyData::from_daily_summaries(summaries.clone());
        let stats_data = crate::types::StatsData::from_daily_summaries(&summaries);
        let models_data = ModelsData::from_model_usage(&HashMap::new());

        app.pending_data = Some(Ok(Box::new(AppData {
            total: crate::types::TotalSummary::default(),
            daily_tokens,
            models_data,
            daily_data,
            stats_data,
            source_usage: vec![],
            source_daily_data: HashMap::new(),
            source_models_data: HashMap::new(),
            source_stats_data: HashMap::new(),
            projects_data: Default::default(),
            project_daily_data: HashMap::new(),
            project_stats_data: HashMap::new(),
            cache_warning: None,
        })));

        let down = Event::Key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
        app.handle_update_event(down);
        let enter = Event::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        app.handle_update_event(enter);

        assert_eq!(app.update_status, UpdateStatus::Resolved);
        assert!(app.pending_data.is_none());
        assert!(matches!(app.state, AppState::Ready { .. }));
    }

    #[test]
    fn test_show_update_overlay_states() {
        assert!(!UpdateStatus::Checking.shows_overlay());
        assert!(!UpdateStatus::Resolved.shows_overlay());
        assert!(UpdateStatus::Available {
            current: "1.0.0".to_string(),
            latest: "2.0.0".to_string()
        }
        .shows_overlay());
        assert!(UpdateStatus::Updating.shows_overlay());
        assert!(UpdateStatus::UpdateDone {
            success: true,
            message: "ok".to_string()
        }
        .shows_overlay());
    }

    #[test]
    fn test_update_done_success_quits_on_any_key() {
        let mut app = App {
            update_status: UpdateStatus::UpdateDone {
                success: true,
                message: "Updated!".to_string(),
            },
            ..App::default()
        };

        let event = Event::Key(KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE));
        app.handle_update_event(event);

        assert!(app.should_quit());
    }

    #[test]
    fn test_update_done_failure_dismisses_on_any_key() {
        let mut app = App {
            update_status: UpdateStatus::UpdateDone {
                success: false,
                message: "Failed".to_string(),
            },
            ..App::default()
        };

        let event = Event::Key(KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE));
        app.handle_update_event(event);

        assert!(!app.should_quit());
        assert_eq!(app.update_status, UpdateStatus::Resolved);
    }

    // ========== TuiConfig & App::new tests ==========

    #[test]
    fn test_tuiconfig_default_values() {
        let config = TuiConfig::default();
        assert_eq!(config.initial_view_mode, DailyViewMode::Daily);
    }

    #[test]
    fn test_app_new_with_custom_config() {
        let config = TuiConfig {
            initial_view_mode: DailyViewMode::Weekly,
            initial_tab: None,
            ..TuiConfig::default()
        };
        let app = App::new(config, Theme::Dark);

        assert!(matches!(app.view_mode, ViewMode::Dashboard { .. }));
        assert_eq!(app.daily_view_mode, DailyViewMode::Weekly);

        assert!(!app.should_quit);
        assert!(matches!(
            app.state,
            AppState::Loading {
                spinner_frame: 0,
                stage: LoadingStage::Scanning
            }
        ));
        assert_eq!(app.update_status, UpdateStatus::Checking);
        assert!(!app.show_help);
        assert_eq!(app.daily_scroll, 0);
        assert_eq!(app.weekly_scroll, 0);
        assert_eq!(app.monthly_scroll, 0);
        assert!(app.pending_data.is_none());
    }

    #[test]
    fn test_checking_state_does_not_show_overlay() {
        assert!(!UpdateStatus::Checking.shows_overlay());

        let mut app = App::default();
        assert_eq!(app.update_status, UpdateStatus::Checking);

        let event = Event::Key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL));
        app.handle_event(event);
        assert!(app.quit_confirm.is_some());
    }

    #[test]
    fn test_pending_data_consumed_on_update_done_failure() {
        let mut app = App {
            update_status: UpdateStatus::UpdateDone {
                success: false,
                message: "npm error".to_string(),
            },
            ..App::default()
        };

        app.pending_data = Some(Err("load failed".to_string()));

        let event = Event::Key(KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE));
        app.handle_update_event(event);

        assert_eq!(app.update_status, UpdateStatus::Resolved);
        assert!(app.pending_data.is_none());
        match &app.state {
            AppState::Error { message } => assert_eq!(message, "load failed"),
            other => panic!(
                "Expected AppState::Error, got {:?}",
                std::mem::discriminant(other)
            ),
        }
    }

    // ========== Quit confirm popup tests ==========

    #[test]
    fn test_ctrl_c_shows_quit_confirm_popup() {
        let mut app = App::default();
        let event = Event::Key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL));
        app.handle_event(event);

        assert!(app.quit_confirm.is_some());
        assert!(!app.should_quit());
    }

    #[test]
    fn test_quit_confirm_default_is_yes() {
        let mut app = App::default();
        app.handle_event(Event::Key(KeyEvent::new(
            KeyCode::Char('c'),
            KeyModifiers::CONTROL,
        )));

        assert_eq!(app.quit_confirm.as_ref().unwrap().selection, 0);
    }

    #[test]
    fn test_quit_confirm_yes_quits() {
        let mut app = App {
            quit_confirm: Some(QuitConfirmState { selection: 0 }),
            ..App::default()
        };

        let event = Event::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        app.handle_quit_confirm_event(event);

        assert!(app.should_quit());
        assert!(app.quit_confirm.is_none());
    }

    #[test]
    fn test_quit_confirm_no_cancels() {
        let mut app = App {
            quit_confirm: Some(QuitConfirmState { selection: 1 }),
            ..App::default()
        };

        let event = Event::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        app.handle_quit_confirm_event(event);

        assert!(!app.should_quit());
        assert!(app.quit_confirm.is_none());
    }

    #[test]
    fn test_quit_confirm_esc_cancels() {
        let mut app = App {
            quit_confirm: Some(QuitConfirmState { selection: 0 }),
            ..App::default()
        };

        let event = Event::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        app.handle_quit_confirm_event(event);

        assert!(!app.should_quit());
        assert!(app.quit_confirm.is_none());
    }

    #[test]
    fn test_quit_confirm_n_key_cancels() {
        let mut app = App {
            quit_confirm: Some(QuitConfirmState { selection: 0 }),
            ..App::default()
        };

        let event = Event::Key(KeyEvent::new(KeyCode::Char('n'), KeyModifiers::NONE));
        app.handle_quit_confirm_event(event);

        assert!(!app.should_quit());
        assert!(app.quit_confirm.is_none());
    }

    #[test]
    fn test_quit_confirm_y_key_quits() {
        let mut app = App {
            quit_confirm: Some(QuitConfirmState { selection: 1 }),
            ..App::default()
        };

        let event = Event::Key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE));
        app.handle_quit_confirm_event(event);

        assert!(app.should_quit());
        assert!(app.quit_confirm.is_none());
    }

    #[test]
    fn test_quit_confirm_arrow_toggles() {
        let mut app = App {
            quit_confirm: Some(QuitConfirmState { selection: 1 }),
            ..App::default()
        };

        let event = Event::Key(KeyEvent::new(KeyCode::Left, KeyModifiers::NONE));
        app.handle_quit_confirm_event(event);
        assert_eq!(app.quit_confirm.as_ref().unwrap().selection, 0);

        let event = Event::Key(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE));
        app.handle_quit_confirm_event(event);
        assert_eq!(app.quit_confirm.as_ref().unwrap().selection, 1);

        let event = Event::Key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE));
        app.handle_quit_confirm_event(event);
        assert_eq!(app.quit_confirm.as_ref().unwrap().selection, 0);

        let event = Event::Key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
        app.handle_quit_confirm_event(event);
        assert_eq!(app.quit_confirm.as_ref().unwrap().selection, 1);
    }

    #[test]
    fn test_quit_confirm_priority_over_update() {
        let mut app = App {
            update_status: UpdateStatus::Available {
                current: "0.1.0".to_string(),
                latest: "0.2.0".to_string(),
            },
            quit_confirm: Some(QuitConfirmState { selection: 1 }),
            ..App::default()
        };

        let event = Event::Key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE));
        app.handle_quit_confirm_event(event);

        assert!(app.should_quit());
        assert!(matches!(app.update_status, UpdateStatus::Available { .. }));
    }

    #[test]
    fn test_app_new_has_no_quit_confirm() {
        let app = App::new(TuiConfig::default(), Theme::Dark);
        assert!(app.quit_confirm.is_none());
    }

    // ========== Model breakdown popup tests ==========

    #[test]
    fn test_app_new_has_no_model_breakdown() {
        let app = App::new(TuiConfig::default(), Theme::Dark);
        assert!(app.model_breakdown.is_none());
    }

    #[test]
    fn test_model_breakdown_esc_closes_popup() {
        let mut app = App {
            model_breakdown: Some(ModelBreakdownState::new("2026-02-05".to_string(), vec![])),
            ..App::default()
        };

        app.handle_model_breakdown_event(Event::Key(KeyEvent::new(
            KeyCode::Esc,
            KeyModifiers::NONE,
        )));

        assert!(app.model_breakdown.is_none());
    }

    #[test]
    fn test_model_breakdown_enter_closes_popup() {
        let mut app = App {
            model_breakdown: Some(ModelBreakdownState::new("2026-02-05".to_string(), vec![])),
            ..App::default()
        };

        app.handle_model_breakdown_event(Event::Key(KeyEvent::new(
            KeyCode::Enter,
            KeyModifiers::NONE,
        )));

        assert!(app.model_breakdown.is_none());
    }

    #[test]
    fn test_model_breakdown_q_closes_popup() {
        let mut app = App {
            model_breakdown: Some(ModelBreakdownState::new("2026-02-05".to_string(), vec![])),
            ..App::default()
        };

        app.handle_model_breakdown_event(Event::Key(KeyEvent::new(
            KeyCode::Char('q'),
            KeyModifiers::NONE,
        )));

        assert!(app.model_breakdown.is_none());
    }

    #[test]
    fn test_selection_adjusts_scroll() {
        let mut app = make_ready_app();
        app.view_mode = ViewMode::SourceDetail {
            source: "claude".to_string(),
        };
        app.daily_scroll = 10;
        app.daily_selected = Some(5);

        app.adjust_scroll_for_selection();
        assert_eq!(app.daily_scroll, 5);
    }

    // ========== Tab switching tests ==========

    #[test]
    fn test_tab_key_switches_tab() {
        let mut app = App::default();
        assert!(matches!(
            app.view_mode,
            ViewMode::Dashboard { tab: Tab::Overview }
        ));

        app.handle_event(Event::Key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE)));
        assert!(matches!(
            app.view_mode,
            ViewMode::Dashboard { tab: Tab::Stats }
        ));

        app.handle_event(Event::Key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE)));
        assert!(matches!(
            app.view_mode,
            ViewMode::Dashboard { tab: Tab::Models }
        ));

        app.handle_event(Event::Key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE)));
        assert!(matches!(
            app.view_mode,
            ViewMode::Dashboard { tab: Tab::Projects }
        ));

        app.handle_event(Event::Key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE)));
        assert!(matches!(
            app.view_mode,
            ViewMode::Dashboard { tab: Tab::Audit }
        ));

        app.handle_event(Event::Key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE)));
        assert!(matches!(
            app.view_mode,
            ViewMode::Dashboard { tab: Tab::Overview }
        ));
    }

    #[test]
    fn test_backtab_switches_tab() {
        let mut app = App::default();

        // From Overview, BackTab wraps to the last tab (Audit).
        app.handle_event(Event::Key(KeyEvent::new(
            KeyCode::BackTab,
            KeyModifiers::SHIFT,
        )));
        assert!(matches!(
            app.view_mode,
            ViewMode::Dashboard { tab: Tab::Audit }
        ));
    }

    #[test]
    fn test_poll_audit_surfaces_disconnected_worker() {
        // If the audit worker thread dies without sending (sender dropped),
        // poll_audit must surface an error and clear the receiver instead of
        // leaving the Audit tab stuck on "Computing…" forever.
        let mut app = App::default();
        let (tx, rx) = mpsc::channel::<std::result::Result<AuditReport, String>>();
        drop(tx);
        app.audit_rx = Some(rx);

        app.poll_audit();

        assert!(app.audit_error.is_some());
        assert!(app.audit_rx.is_none());
        assert!(app.audit.is_none());
    }

    #[test]
    fn test_number_keys_switch_tab() {
        let mut app = App::default();

        app.handle_event(Event::Key(KeyEvent::new(
            KeyCode::Char('2'),
            KeyModifiers::NONE,
        )));
        assert!(matches!(
            app.view_mode,
            ViewMode::Dashboard { tab: Tab::Stats }
        ));

        app.handle_event(Event::Key(KeyEvent::new(
            KeyCode::Char('3'),
            KeyModifiers::NONE,
        )));
        assert!(matches!(
            app.view_mode,
            ViewMode::Dashboard { tab: Tab::Models }
        ));

        app.handle_event(Event::Key(KeyEvent::new(
            KeyCode::Char('1'),
            KeyModifiers::NONE,
        )));
        assert!(matches!(
            app.view_mode,
            ViewMode::Dashboard { tab: Tab::Overview }
        ));
    }

    #[test]
    fn test_initial_tab_config() {
        let config = TuiConfig {
            initial_view_mode: DailyViewMode::Daily,
            initial_tab: Some(Tab::Stats),
            ..TuiConfig::default()
        };
        let app = App::new(config, Theme::Dark);
        assert!(matches!(
            app.view_mode,
            ViewMode::Dashboard { tab: Tab::Stats }
        ));
    }

    /// Build a Ready app holding a single project so the Projects drill-down can
    /// be exercised end-to-end.
    fn ready_app_with_one_project(key: &str) -> App {
        let mut summary = DailySummary {
            date: NaiveDate::from_ymd_opt(2026, 1, 1).unwrap(),
            total_input_tokens: 100,
            total_output_tokens: 50,
            total_cache_read_tokens: 0,
            total_cache_creation_tokens: 0,
            total_reasoning_tokens: 0,
            total_cache_creation_5m_tokens: 0,
            total_cache_creation_1h_tokens: 0,
            total_web_search_requests: 0,
            total_cost_usd: 1.0,
            models: HashMap::new(),
            projects: HashMap::new(),
        };
        let mut pu = crate::types::ProjectUsage {
            input_tokens: 100,
            output_tokens: 50,
            cost_usd: 1.0,
            count: 1,
            ..Default::default()
        };
        pu.models
            .insert("claude-opus".to_string(), Default::default());
        summary.projects.insert(key.to_string(), pu.clone());

        let summaries = vec![summary];
        let projects_data =
            ProjectsData::from_project_usage(&Aggregator::by_project_from_daily(&summaries));
        let project_summaries = Aggregator::project_daily_summaries(&summaries);
        let mut project_daily_data = HashMap::new();
        let mut project_stats_data = HashMap::new();
        for (p, s) in &project_summaries {
            project_daily_data.insert(p.clone(), DailyData::from_daily_summaries(s.clone()));
            project_stats_data.insert(p.clone(), StatsData::from_daily_summaries(s));
        }

        let data = AppData {
            total: crate::types::TotalSummary::default(),
            daily_tokens: vec![],
            models_data: ModelsData::from_model_usage(&HashMap::new()),
            daily_data: DailyData::from_daily_summaries(summaries.clone()),
            stats_data: StatsData::from_daily_summaries(&summaries),
            source_usage: vec![],
            source_daily_data: HashMap::new(),
            source_models_data: HashMap::new(),
            source_stats_data: HashMap::new(),
            projects_data,
            project_daily_data,
            project_stats_data,
            cache_warning: None,
        };

        App {
            state: AppState::Ready {
                data: Box::new(data),
            },
            ..App::default()
        }
    }

    #[test]
    fn test_projects_tab_enter_drills_into_project_detail() {
        let key = "/srv/work/alpha/beta";
        let mut app = ready_app_with_one_project(key);

        // Jump to the Projects tab (key '4').
        app.handle_event(Event::Key(KeyEvent::new(
            KeyCode::Char('4'),
            KeyModifiers::NONE,
        )));
        assert!(matches!(
            app.view_mode,
            ViewMode::Dashboard { tab: Tab::Projects }
        ));

        // Enter drills into the selected project.
        app.handle_event(Event::Key(KeyEvent::new(
            KeyCode::Enter,
            KeyModifiers::NONE,
        )));
        match &app.view_mode {
            ViewMode::ProjectDetail { project } => assert_eq!(project, key),
            other => panic!("expected ProjectDetail, got {:?}", other),
        }

        // Esc returns to the Projects tab (not Overview).
        app.handle_event(Event::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)));
        assert!(matches!(
            app.view_mode,
            ViewMode::Dashboard { tab: Tab::Projects }
        ));
    }

    #[test]
    fn test_key_5_switches_to_audit() {
        let mut app = App::default();
        app.handle_event(Event::Key(KeyEvent::new(
            KeyCode::Char('5'),
            KeyModifiers::NONE,
        )));
        assert!(matches!(
            app.view_mode,
            ViewMode::Dashboard { tab: Tab::Audit }
        ));
    }

    // ========== Manual refresh tests (issue #198) ==========

    /// Stub loader for refresh tests — never touches the real filesystem.
    fn stub_loader(_: RemoteOptions) -> Result<Box<AppData>, String> {
        Err("stub loader".to_string())
    }

    /// Minimal AppData whose daily_tokens value distinguishes it from make_ready_app data.
    fn marker_app_data(marker: u64) -> Box<AppData> {
        use chrono::NaiveDate;
        let empty: Vec<DailySummary> = vec![];
        Box::new(AppData {
            total: crate::types::TotalSummary::default(),
            daily_tokens: vec![(NaiveDate::from_ymd_opt(2025, 1, 1).unwrap(), marker)],
            models_data: ModelsData::from_model_usage(&HashMap::new()),
            daily_data: DailyData::from_daily_summaries(empty.clone()),
            stats_data: StatsData::from_daily_summaries(&empty),
            source_usage: vec![],
            source_daily_data: HashMap::new(),
            source_models_data: HashMap::new(),
            source_stats_data: HashMap::new(),
            projects_data: Default::default(),
            project_daily_data: HashMap::new(),
            project_stats_data: HashMap::new(),
            cache_warning: None,
        })
    }

    fn press_r(app: &mut App) {
        app.handle_event(Event::Key(KeyEvent::new(
            KeyCode::Char('r'),
            KeyModifiers::NONE,
        )));
    }

    #[test]
    fn test_r_key_in_ready_dashboard_starts_refresh() {
        let mut app = make_ready_app();
        app.data_loader = stub_loader;

        press_r(&mut app);

        assert!(app.refreshing);
        assert!(app.data_rx.is_some());
        // Current data stays on screen — no flicker back to the loading spinner.
        assert!(matches!(app.state, AppState::Ready { .. }));
    }

    #[test]
    fn test_r_key_is_noop_while_load_in_flight() {
        let mut app = make_ready_app();
        app.data_loader = stub_loader;
        let (tx, rx) = mpsc::channel();
        app.data_rx = Some(rx);
        app.refreshing = true;

        // 'r' must not replace the in-flight channel.
        press_r(&mut app);

        // The original channel must still be the one polled.
        tx.send(Ok(marker_app_data(999))).unwrap();
        app.poll_data();

        match &app.state {
            AppState::Ready { data } => assert_eq!(
                data.daily_tokens,
                vec![(chrono::NaiveDate::from_ymd_opt(2025, 1, 1).unwrap(), 999)]
            ),
            _ => panic!("expected Ready state"),
        }
        assert!(!app.refreshing);
        assert!(app.data_rx.is_none());
    }

    #[test]
    fn test_poll_data_applies_refreshed_data_and_invalidates_audit() {
        use chrono::NaiveDate;
        let mut app = make_ready_app();
        app.audit = Some(AuditReport {
            generated_for: NaiveDate::from_ymd_opt(2025, 1, 1).unwrap(),
            total_cache_only_days: 3,
            sources: vec![],
        });
        app.audit_error = Some("stale".to_string());
        // In-flight audit computation started before the refresh — its result is
        // stale and must be dropped, not resolved later by poll_audit.
        let (stale_audit_tx, stale_audit_rx) = mpsc::channel();
        app.audit_rx = Some(stale_audit_rx);
        let (tx, rx) = mpsc::channel();
        app.data_rx = Some(rx);
        app.refreshing = true;

        tx.send(Ok(marker_app_data(777))).unwrap();
        app.poll_data();

        match &app.state {
            AppState::Ready { data } => assert_eq!(data.daily_tokens[0].1, 777),
            _ => panic!("expected Ready state"),
        }
        assert!(!app.refreshing);
        assert!(app.data_rx.is_none());
        // Refreshed data invalidates the previously computed audit.
        assert!(app.audit.is_none());
        assert!(app.audit_error.is_none());
        // The in-flight audit receiver must be dropped so poll_audit can never
        // resolve a stale pre-refresh result.
        assert!(app.audit_rx.is_none());
        assert!(stale_audit_tx
            .send(Ok(AuditReport {
                generated_for: NaiveDate::from_ymd_opt(2025, 1, 2).unwrap(),
                total_cache_only_days: 9,
                sources: vec![],
            }))
            .is_err());
    }

    #[test]
    fn test_refresh_failure_keeps_existing_data() {
        let mut app = make_ready_app();
        let (tx, rx) = mpsc::channel();
        app.data_rx = Some(rx);
        app.refreshing = true;

        tx.send(Err("boom".to_string())).unwrap();
        app.poll_data();

        // Graceful degrade: old data survives a failed refresh.
        assert!(matches!(app.state, AppState::Ready { .. }));
        assert!(!app.refreshing);
        assert!(app.data_rx.is_none());
    }

    #[test]
    fn test_refresh_worker_panic_keeps_existing_data() {
        let mut app = make_ready_app();
        let (tx, rx) = mpsc::channel::<Result<Box<AppData>, String>>();
        app.data_rx = Some(rx);
        app.refreshing = true;

        // Sender dropped without sending — as if the worker thread panicked.
        drop(tx);
        app.poll_data();

        assert!(matches!(app.state, AppState::Ready { .. }));
        assert!(!app.refreshing);
        assert!(app.data_rx.is_none());
    }

    #[test]
    fn test_startup_load_worker_panic_sets_error_state() {
        let mut app = App::default();
        let (tx, rx) = mpsc::channel::<Result<Box<AppData>, String>>();
        app.data_rx = Some(rx);

        drop(tx);
        app.poll_data();

        assert!(matches!(app.state, AppState::Error { .. }));
        assert!(app.data_rx.is_none());
    }

    #[test]
    fn test_r_key_in_error_state_retries() {
        let mut app = App {
            data_loader: stub_loader,
            state: AppState::Error {
                message: "load failed".to_string(),
            },
            ..App::default()
        };

        press_r(&mut app);

        // Retry from the error screen goes back to the loading spinner.
        assert!(matches!(app.state, AppState::Loading { .. }));
        assert!(app.data_rx.is_some());
        assert!(!app.refreshing);
    }

    #[test]
    fn test_r_key_in_detail_view_starts_refresh_and_preserves_view() {
        let mut app = make_ready_app();
        app.data_loader = stub_loader;
        app.view_mode = ViewMode::SourceDetail {
            source: "claude".to_string(),
        };

        press_r(&mut app);
        assert!(app.refreshing);

        // Swap in a deterministic channel and apply the result.
        let (tx, rx) = mpsc::channel();
        app.data_rx = Some(rx);
        tx.send(Ok(marker_app_data(555))).unwrap();
        app.poll_data();

        // The drill-down view survives the refresh.
        assert!(matches!(app.view_mode, ViewMode::SourceDetail { .. }));
        assert!(matches!(app.state, AppState::Ready { .. }));
    }

    #[test]
    fn test_refresh_result_deferred_while_update_overlay_shown() {
        let mut app = make_ready_app();
        app.update_status = UpdateStatus::Available {
            current: "1.0.0".to_string(),
            latest: "2.0.0".to_string(),
        };
        let (tx, rx) = mpsc::channel();
        app.data_rx = Some(rx);
        app.refreshing = true;

        tx.send(Ok(marker_app_data(333))).unwrap();
        app.poll_data();

        assert!(app.pending_data.is_some());
        assert!(app.data_rx.is_none());
        assert!(!app.refreshing);
    }
}
