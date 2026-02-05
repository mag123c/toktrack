//! Application state and event loop

use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use chrono::{Local, NaiveDate};
use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use ratatui::{
    buffer::Buffer, layout::Rect, style::Style, widgets::Widget, DefaultTerminal, Frame,
};

use super::theme::Theme;

use crate::services::update_checker::{check_for_update, execute_update, UpdateCheckResult};
use crate::services::{Aggregator, DataLoaderService};
use crate::types::{CacheWarning, SourceUsage, StatsData, TotalSummary};

use super::widgets::{
    daily::{DailyData, DailyView, DailyViewMode},
    help::HelpPopup,
    model_breakdown::{ModelBreakdownPopup, ModelBreakdownState},
    models::{ModelsData, ModelsView},
    overview::{Overview, OverviewData},
    quit_confirm::{QuitConfirmPopup, QuitConfirmState},
    spinner::{LoadingStage, Spinner},
    stats::StatsView,
    tabs::Tab,
    update_popup::{DimOverlay, UpdateMessagePopup, UpdatePopup},
};

/// Configuration for TUI startup
#[derive(Debug, Clone, Default)]
pub struct TuiConfig {
    pub initial_tab: Tab,
    pub initial_view_mode: DailyViewMode,
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
    /// Cache warning indicator for display in TUI
    #[allow(dead_code)] // Reserved for warning indicator feature
    pub cache_warning: Option<CacheWarning>,
}

/// Update overlay status
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpdateStatus {
    /// Background check in progress
    Checking,
    /// Update available, showing overlay
    Available { current: String, latest: String },
    /// Running npm update
    Updating,
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
                | UpdateStatus::UpdateDone { .. }
        )
    }
}

/// Main application
pub struct App {
    state: AppState,
    should_quit: bool,
    current_tab: Tab,
    daily_scroll: usize,
    weekly_scroll: usize,
    monthly_scroll: usize,
    daily_selected: Option<usize>,
    weekly_selected: Option<usize>,
    monthly_selected: Option<usize>,
    daily_view_mode: DailyViewMode,
    show_help: bool,
    update_status: UpdateStatus,
    update_selection: u8, // 0 = Update now, 1 = Skip
    pending_data: Option<Result<Box<AppData>, String>>,
    theme: Theme,
    quit_confirm: Option<QuitConfirmState>,
    model_breakdown: Option<ModelBreakdownState>,
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
            current_tab: config.initial_tab,
            daily_scroll: 0,
            weekly_scroll: 0,
            monthly_scroll: 0,
            daily_selected: None,
            weekly_selected: None,
            monthly_selected: None,
            daily_view_mode: config.initial_view_mode,
            show_help: false,
            update_status: UpdateStatus::Checking,
            update_selection: 0,
            pending_data: None,
            theme,
            quit_confirm: None,
            model_breakdown: None,
        }
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

    /// Handle keyboard events
    pub fn handle_event(&mut self, event: Event) {
        if let Event::Key(key) = event {
            if key.kind == KeyEventKind::Press {
                // Ctrl+C shows quit confirmation
                if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
                    self.quit_confirm = Some(QuitConfirmState::new());
                    return;
                }

                match key.code {
                    KeyCode::Esc => {
                        // Esc only closes popups (e.g., help), does not trigger quit
                        if self.show_help {
                            self.show_help = false;
                        }
                        // If no popup is open, Esc does nothing
                    }
                    KeyCode::Tab => {
                        self.current_tab = self.current_tab.next();
                    }
                    KeyCode::BackTab => {
                        self.current_tab = self.current_tab.prev();
                    }
                    KeyCode::Up | KeyCode::Char('k') => {
                        self.select_prev();
                    }
                    KeyCode::Down | KeyCode::Char('j') => {
                        self.select_next();
                    }
                    KeyCode::Enter => {
                        if self.current_tab == Tab::Daily {
                            self.open_model_breakdown();
                        }
                    }
                    KeyCode::Char(c @ '1'..='4') => {
                        if let Some(tab) = Tab::from_number(c as u8 - b'0') {
                            self.current_tab = tab;
                        }
                    }
                    KeyCode::Char('?') => {
                        self.show_help = !self.show_help;
                    }
                    KeyCode::Char('d') if self.current_tab == Tab::Daily => {
                        self.daily_view_mode = DailyViewMode::Daily;
                    }
                    KeyCode::Char('w') if self.current_tab == Tab::Daily => {
                        self.daily_view_mode = DailyViewMode::Weekly;
                    }
                    KeyCode::Char('m') if self.current_tab == Tab::Daily => {
                        self.daily_view_mode = DailyViewMode::Monthly;
                    }
                    _ => {}
                }
            }
        }
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
                    // Available state: ↑↓ to select, Enter to confirm, q/Esc to quit
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
            Ok(data) => {
                self.daily_scroll =
                    DailyView::max_scroll_offset(&data.daily_data, DailyViewMode::Daily);
                self.weekly_scroll =
                    DailyView::max_scroll_offset(&data.daily_data, DailyViewMode::Weekly);
                self.monthly_scroll =
                    DailyView::max_scroll_offset(&data.daily_data, DailyViewMode::Monthly);
                self.state = AppState::Ready { data };
            }
            Err(message) => self.state = AppState::Error { message },
        }
    }

    /// Select previous row (move up) in Daily tab
    fn select_prev(&mut self) {
        if self.current_tab != Tab::Daily {
            return;
        }

        // First, get the count from state (immutable borrow)
        let count = match &self.state {
            AppState::Ready { data } => {
                let (summaries, _) = data.daily_data.for_mode(self.daily_view_mode);
                summaries.len()
            }
            _ => return,
        };

        if count == 0 {
            return;
        }

        // Now mutate (mutable borrow)
        let current = self.active_selected();
        let new_idx = match current {
            None => count.saturating_sub(1), // Start from bottom (most recent)
            Some(0) => 0,                    // Already at top
            Some(idx) => idx.saturating_sub(1),
        };
        *self.active_selected_mut() = Some(new_idx);

        // Adjust scroll to keep selection visible
        self.adjust_scroll_for_selection();
    }

    /// Select next row (move down) in Daily tab
    fn select_next(&mut self) {
        if self.current_tab != Tab::Daily {
            return;
        }

        // First, get the count from state (immutable borrow)
        let count = match &self.state {
            AppState::Ready { data } => {
                let (summaries, _) = data.daily_data.for_mode(self.daily_view_mode);
                summaries.len()
            }
            _ => return,
        };

        if count == 0 {
            return;
        }

        let max_idx = count.saturating_sub(1);

        // Now mutate (mutable borrow)
        let current = self.active_selected();
        let new_idx = match current {
            None => count.saturating_sub(1), // Start from bottom (most recent)
            Some(idx) if idx >= max_idx => max_idx, // Already at bottom
            Some(idx) => idx + 1,
        };
        *self.active_selected_mut() = Some(new_idx);

        // Adjust scroll to keep selection visible
        self.adjust_scroll_for_selection();
    }

    /// Adjust scroll offset to keep the current selection visible
    fn adjust_scroll_for_selection(&mut self) {
        use super::widgets::daily::VISIBLE_ROWS;

        let selected = match self.active_selected() {
            Some(idx) => idx,
            None => return,
        };

        let scroll = self.active_scroll();

        // If selection is above visible area, scroll up
        if selected < scroll {
            *self.active_scroll_mut() = selected;
        }
        // If selection is below visible area, scroll down
        else if selected >= scroll + VISIBLE_ROWS {
            *self.active_scroll_mut() = selected.saturating_sub(VISIBLE_ROWS - 1);
        }
    }

    /// Open model breakdown popup for the currently selected row
    fn open_model_breakdown(&mut self) {
        if self.current_tab != Tab::Daily {
            return;
        }
        let selected = match self.active_selected() {
            Some(idx) => idx,
            None => return,
        };

        if let AppState::Ready { data } = &self.state {
            let (summaries, _) = data.daily_data.for_mode(self.daily_view_mode);
            if let Some(summary) = summaries.get(selected) {
                // Format date label based on view mode
                let date_label = match self.daily_view_mode {
                    DailyViewMode::Daily | DailyViewMode::Weekly => {
                        summary.date.format("%Y-%m-%d").to_string()
                    }
                    DailyViewMode::Monthly => summary.date.format("%Y-%m").to_string(),
                };

                // Collect models as Vec
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
                // Render main view
                match self.current_tab {
                    Tab::Overview => {
                        let today = Local::now().date_naive();
                        let overview_data = OverviewData {
                            total: &data.total,
                            daily_tokens: &data.daily_tokens,
                            source_usage: &data.source_usage,
                        };
                        let overview = Overview::new(overview_data, today, self.theme)
                            .with_tab(self.current_tab);
                        overview.render(area, buf);
                    }
                    Tab::Models => {
                        let models_view = ModelsView::new(&data.models_data, self.theme)
                            .with_tab(self.current_tab);
                        models_view.render(area, buf);
                    }
                    Tab::Daily => {
                        let daily_view = DailyView::new(
                            &data.daily_data,
                            self.active_scroll(),
                            self.daily_view_mode,
                            self.theme,
                        )
                        .with_tab(self.current_tab)
                        .with_selected_index(self.active_selected());
                        daily_view.render(area, buf);
                    }
                    Tab::Stats => {
                        let stats_view =
                            StatsView::new(&data.stats_data, self.theme).with_tab(self.current_tab);
                        stats_view.render(area, buf);
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
            UpdateStatus::Updating => {
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

/// Load data synchronously (extracted for background thread).
/// Uses cache-first strategy via DataLoaderService.
fn load_data_sync() -> Result<Box<AppData>, String> {
    let result = DataLoaderService::new().load().map_err(|e| e.to_string())?;

    build_app_data_from_summaries(result.summaries, result.source_usage, result.cache_warning)
}

/// Build AppData from DailySummary list (no raw entries needed).
fn build_app_data_from_summaries(
    summaries: Vec<crate::types::DailySummary>,
    source_usage: Vec<SourceUsage>,
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
                    + d.total_cache_creation_tokens,
            )
        })
        .collect();

    let model_map = Aggregator::by_model_from_daily(&summaries);
    let models_data = ModelsData::from_model_usage(&model_map);
    let stats_data = StatsData::from_daily_summaries(&summaries);
    let daily_data = DailyData::from_daily_summaries(summaries);

    Ok(Box::new(AppData {
        total,
        daily_tokens,
        models_data,
        daily_data,
        stats_data,
        source_usage,
        cache_warning,
    }))
}

fn run_app(terminal: &mut DefaultTerminal, config: TuiConfig, theme: Theme) -> anyhow::Result<()> {
    let mut app = App::new(config, theme);

    // Spawn background thread for data loading
    let (data_tx, data_rx) = mpsc::channel();
    thread::spawn(move || {
        let result = load_data_sync();
        let _ = data_tx.send(result);
    });

    // Spawn background thread for update check
    let (update_tx, update_rx) = mpsc::channel();
    thread::spawn(move || {
        let result = check_for_update();
        let _ = update_tx.send(result);
    });

    loop {
        terminal.draw(|frame| app.draw(frame))?;

        if app.should_quit() {
            break;
        }

        // Check for data loading completion (non-blocking)
        if matches!(app.state, AppState::Loading { .. }) {
            if let Ok(result) = data_rx.try_recv() {
                if app.update_status.shows_overlay() {
                    // Overlay is active, store data for later
                    app.pending_data = Some(result);
                } else {
                    app.apply_data_result(result);
                }
            }
        }

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

        // Handle Updating state: run npm update in background
        if app.update_status == UpdateStatus::Updating {
            // Draw once to show "Running..." message before blocking
            terminal.draw(|frame| app.draw(frame))?;
            match execute_update() {
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
                total_cost_usd: 0.01,
                models: HashMap::new(),
            })
            .collect();

        let daily_tokens: Vec<(NaiveDate, u64)> = summaries.iter().map(|d| (d.date, 150)).collect();

        let daily_data = DailyData::from_daily_summaries(summaries.clone());
        let stats_data = crate::types::StatsData::from_daily_summaries(&summaries);
        let models_data = super::ModelsData::from_model_usage(&HashMap::new());

        let mut app = App::default();
        let daily_scroll = DailyView::max_scroll_offset(&daily_data, DailyViewMode::Daily);
        let weekly_scroll = DailyView::max_scroll_offset(&daily_data, DailyViewMode::Weekly);
        let monthly_scroll = DailyView::max_scroll_offset(&daily_data, DailyViewMode::Monthly);

        app.state = AppState::Ready {
            data: Box::new(AppData {
                total: crate::types::TotalSummary::default(),
                daily_tokens,
                models_data,
                daily_data,
                stats_data,
                source_usage: vec![],
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

        // q key should do nothing (no quit confirm, no quit)
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

        // Esc should close help popup, not show quit confirm
        assert!(!app.show_help);
        assert!(app.quit_confirm.is_none());
        assert!(!app.should_quit());
    }

    #[test]
    fn test_esc_does_nothing_when_no_popup() {
        let mut app = App::default();
        // Default show_help is false
        assert!(!app.show_help);

        let event = Event::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        app.handle_event(event);

        // Esc should do nothing when no popup is open
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
    fn test_app_tab_navigation() {
        let mut app = App::default();
        assert_eq!(app.current_tab, Tab::Overview);

        // Tab forward
        let event = Event::Key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
        app.handle_event(event);
        assert_eq!(app.current_tab, Tab::Models);

        app.handle_event(Event::Key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE)));
        assert_eq!(app.current_tab, Tab::Daily);

        app.handle_event(Event::Key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE)));
        assert_eq!(app.current_tab, Tab::Stats);

        // Wrap around
        app.handle_event(Event::Key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE)));
        assert_eq!(app.current_tab, Tab::Overview);
    }

    #[test]
    fn test_app_tab_navigation_backward() {
        let mut app = App::default();
        assert_eq!(app.current_tab, Tab::Overview);

        // Shift+Tab (BackTab)
        let event = Event::Key(KeyEvent::new(KeyCode::BackTab, KeyModifiers::SHIFT));
        app.handle_event(event);
        assert_eq!(app.current_tab, Tab::Stats);

        app.handle_event(Event::Key(KeyEvent::new(
            KeyCode::BackTab,
            KeyModifiers::SHIFT,
        )));
        assert_eq!(app.current_tab, Tab::Daily);
    }

    #[test]
    fn test_app_number_key_navigation() {
        let mut app = App::default();
        assert_eq!(app.current_tab, Tab::Overview);

        // Press '2' to go to Models
        let event = Event::Key(KeyEvent::new(KeyCode::Char('2'), KeyModifiers::NONE));
        app.handle_event(event);
        assert_eq!(app.current_tab, Tab::Models);

        // Press '4' to go to Stats
        let event = Event::Key(KeyEvent::new(KeyCode::Char('4'), KeyModifiers::NONE));
        app.handle_event(event);
        assert_eq!(app.current_tab, Tab::Stats);

        // Press '3' to go to Daily
        let event = Event::Key(KeyEvent::new(KeyCode::Char('3'), KeyModifiers::NONE));
        app.handle_event(event);
        assert_eq!(app.current_tab, Tab::Daily);

        // Press '1' to go back to Overview
        let event = Event::Key(KeyEvent::new(KeyCode::Char('1'), KeyModifiers::NONE));
        app.handle_event(event);
        assert_eq!(app.current_tab, Tab::Overview);
    }

    #[test]
    fn test_app_help_toggle() {
        let mut app = App::default();
        assert!(!app.show_help);

        // Press '?' to show help
        let event = Event::Key(KeyEvent::new(KeyCode::Char('?'), KeyModifiers::NONE));
        app.handle_event(event.clone());
        assert!(app.show_help);

        // Press '?' again to hide help
        app.handle_event(event);
        assert!(!app.show_help);
    }

    #[test]
    fn test_d_w_m_keys_on_daily_tab() {
        let mut app = make_ready_app();
        // Navigate to Daily tab
        app.current_tab = Tab::Daily;
        assert_eq!(app.daily_view_mode, DailyViewMode::Daily);

        // Press 'w' → Weekly
        app.handle_event(Event::Key(KeyEvent::new(
            KeyCode::Char('w'),
            KeyModifiers::NONE,
        )));
        assert_eq!(app.daily_view_mode, DailyViewMode::Weekly);

        // Press 'm' → Monthly
        app.handle_event(Event::Key(KeyEvent::new(
            KeyCode::Char('m'),
            KeyModifiers::NONE,
        )));
        assert_eq!(app.daily_view_mode, DailyViewMode::Monthly);

        // Press 'd' → Daily
        app.handle_event(Event::Key(KeyEvent::new(
            KeyCode::Char('d'),
            KeyModifiers::NONE,
        )));
        assert_eq!(app.daily_view_mode, DailyViewMode::Daily);
    }

    #[test]
    fn test_d_w_m_keys_ignored_on_other_tabs() {
        let mut app = make_ready_app();
        app.current_tab = Tab::Overview;
        assert_eq!(app.daily_view_mode, DailyViewMode::Daily);

        // Press 'w' on Overview tab → should NOT change view mode
        app.handle_event(Event::Key(KeyEvent::new(
            KeyCode::Char('w'),
            KeyModifiers::NONE,
        )));
        assert_eq!(app.daily_view_mode, DailyViewMode::Daily);

        // Same for Models tab
        app.current_tab = Tab::Models;
        app.handle_event(Event::Key(KeyEvent::new(
            KeyCode::Char('m'),
            KeyModifiers::NONE,
        )));
        assert_eq!(app.daily_view_mode, DailyViewMode::Daily);
    }

    #[test]
    fn test_independent_selection_positions() {
        let mut app = make_ready_app();
        app.current_tab = Tab::Daily;

        // Initially no selection
        assert!(app.daily_selected.is_none());
        assert!(app.weekly_selected.is_none());

        // Select up in Daily mode (starts from bottom)
        app.handle_event(Event::Key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE)));
        assert!(app.daily_selected.is_some());
        let daily_selected = app.daily_selected.unwrap();

        // Weekly selection should be unchanged (None)
        assert!(app.weekly_selected.is_none());

        // Select up again in Daily mode
        app.handle_event(Event::Key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE)));
        assert_eq!(app.daily_selected, Some(daily_selected.saturating_sub(1)));

        // Switch to Weekly and select
        app.daily_view_mode = DailyViewMode::Weekly;
        app.handle_event(Event::Key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE)));
        assert!(app.weekly_selected.is_some());

        // Daily selection should remain as modified
        assert_eq!(app.daily_selected, Some(daily_selected.saturating_sub(1)));
    }

    // ========== Update overlay tests ==========

    #[test]
    fn test_app_initial_update_status() {
        let app = App::default();
        assert_eq!(app.update_status, UpdateStatus::Checking);
        assert!(app.pending_data.is_none());
    }

    /// Helper to create an app with update available overlay
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

        // ↓ to select Skip, Enter to confirm
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

        // Default selection=0 (Update now), Enter to confirm
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

        // Esc should dismiss the update overlay (skip update), not quit
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

        // Simulate data arriving while overlay is shown
        let summaries: Vec<DailySummary> = vec![DailySummary {
            date: NaiveDate::from_ymd_opt(2025, 1, 1).unwrap(),
            total_input_tokens: 100,
            total_output_tokens: 50,
            total_cache_read_tokens: 0,
            total_cache_creation_tokens: 0,
            total_cost_usd: 0.01,
            models: HashMap::new(),
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
            cache_warning: None,
        })));

        // Skip update overlay: ↓ to Skip, Enter to confirm
        let down = Event::Key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
        app.handle_update_event(down);
        let enter = Event::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        app.handle_update_event(enter);

        // Should have consumed pending_data and transitioned to Ready
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
        assert_eq!(config.initial_tab, Tab::Overview);
        assert_eq!(config.initial_view_mode, DailyViewMode::Daily);
    }

    #[test]
    fn test_app_new_with_custom_config() {
        let config = TuiConfig {
            initial_tab: Tab::Daily,
            initial_view_mode: DailyViewMode::Weekly,
        };
        let app = App::new(config, Theme::Dark);

        // Config-driven fields
        assert_eq!(app.current_tab, Tab::Daily);
        assert_eq!(app.daily_view_mode, DailyViewMode::Weekly);

        // Default initial fields
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
        // UpdateStatus::Checking.shows_overlay() == false is the guard that
        // prevents handle_update_event from being called during Checking state
        assert!(!UpdateStatus::Checking.shows_overlay());

        // Verify the production code path: when shows_overlay() is false,
        // handle_event runs instead of handle_update_event
        let mut app = App::default();
        assert_eq!(app.update_status, UpdateStatus::Checking);

        // Ctrl+C via handle_event should show quit_confirm (proving handle_event runs, not handle_update_event)
        let event = Event::Key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL));
        app.handle_event(event);
        assert!(app.quit_confirm.is_some()); // Shows quit confirmation popup
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

        // Set pending data with an error result
        app.pending_data = Some(Err("load failed".to_string()));

        // Dismiss the UpdateDone overlay (failure path consumes pending_data)
        let event = Event::Key(KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE));
        app.handle_update_event(event);

        assert_eq!(app.update_status, UpdateStatus::Resolved);
        assert!(app.pending_data.is_none());
        // Error result should have been applied → Error state with correct message
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
    fn test_quit_confirm_default_is_no() {
        let mut app = App::default();
        // Use Ctrl+C to trigger quit confirmation
        app.handle_event(Event::Key(KeyEvent::new(
            KeyCode::Char('c'),
            KeyModifiers::CONTROL,
        )));

        // Default selection should be No (1)
        assert_eq!(app.quit_confirm.as_ref().unwrap().selection, 1);
    }

    #[test]
    fn test_quit_confirm_yes_quits() {
        let mut app = App {
            quit_confirm: Some(QuitConfirmState { selection: 0 }), // Yes selected
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
            quit_confirm: Some(QuitConfirmState { selection: 1 }), // No selected
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
            quit_confirm: Some(QuitConfirmState { selection: 0 }), // Yes selected
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
            quit_confirm: Some(QuitConfirmState { selection: 1 }), // No selected
            ..App::default()
        };

        // 'y' should quit immediately regardless of selection
        let event = Event::Key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE));
        app.handle_quit_confirm_event(event);

        assert!(app.should_quit());
        assert!(app.quit_confirm.is_none());
    }

    #[test]
    fn test_quit_confirm_arrow_toggles() {
        let mut app = App {
            quit_confirm: Some(QuitConfirmState { selection: 1 }), // No selected
            ..App::default()
        };

        // Left arrow toggles to Yes
        let event = Event::Key(KeyEvent::new(KeyCode::Left, KeyModifiers::NONE));
        app.handle_quit_confirm_event(event);
        assert_eq!(app.quit_confirm.as_ref().unwrap().selection, 0);

        // Right arrow toggles back to No
        let event = Event::Key(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE));
        app.handle_quit_confirm_event(event);
        assert_eq!(app.quit_confirm.as_ref().unwrap().selection, 1);

        // Up/Down also toggle
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

        // 'y' in quit_confirm should quit, not interact with update overlay
        let event = Event::Key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE));
        app.handle_quit_confirm_event(event);

        assert!(app.should_quit());
        // Update status should be unchanged
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
    fn test_enter_without_selection_does_nothing() {
        let mut app = make_ready_app();
        app.current_tab = Tab::Daily;
        // No selection
        assert!(app.daily_selected.is_none());

        // Press Enter
        app.handle_event(Event::Key(KeyEvent::new(
            KeyCode::Enter,
            KeyModifiers::NONE,
        )));

        // Should not open model breakdown
        assert!(app.model_breakdown.is_none());
    }

    #[test]
    fn test_enter_with_selection_opens_popup() {
        let mut app = make_ready_app_with_models();
        app.current_tab = Tab::Daily;

        // Select a row first
        app.handle_event(Event::Key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE)));
        assert!(app.daily_selected.is_some());

        // Press Enter
        app.handle_event(Event::Key(KeyEvent::new(
            KeyCode::Enter,
            KeyModifiers::NONE,
        )));

        // Should open model breakdown
        assert!(app.model_breakdown.is_some());
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
    fn test_selection_starts_from_bottom() {
        let mut app = make_ready_app();
        app.current_tab = Tab::Daily;

        // First selection should be at the bottom (most recent date)
        app.handle_event(Event::Key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE)));

        // Should select last index (20 items, index 19)
        assert_eq!(app.daily_selected, Some(19));
    }

    #[test]
    fn test_selection_down_from_none_starts_from_bottom() {
        let mut app = make_ready_app();
        app.current_tab = Tab::Daily;

        // Down should also start from bottom
        app.handle_event(Event::Key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE)));

        assert_eq!(app.daily_selected, Some(19));
    }

    #[test]
    fn test_selection_wraps_at_boundaries() {
        let mut app = make_ready_app();
        app.current_tab = Tab::Daily;
        app.daily_selected = Some(0);

        // Up at index 0 should stay at 0
        app.handle_event(Event::Key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE)));
        assert_eq!(app.daily_selected, Some(0));
    }

    #[test]
    fn test_selection_adjusts_scroll() {
        let mut app = make_ready_app();
        app.current_tab = Tab::Daily;
        // Set selection above visible area
        app.daily_scroll = 10;
        app.daily_selected = Some(5);

        // Adjust scroll should bring selection into view
        app.adjust_scroll_for_selection();
        assert_eq!(app.daily_scroll, 5);
    }

    /// Helper to create a ready app with model data in summaries
    fn make_ready_app_with_models() -> App {
        use crate::types::{DailySummary, ModelUsage};
        use chrono::NaiveDate;

        let summaries: Vec<DailySummary> = (1..=20)
            .map(|d| {
                let mut models = HashMap::new();
                models.insert(
                    "claude-sonnet-4-20250514".to_string(),
                    ModelUsage {
                        input_tokens: 100,
                        output_tokens: 50,
                        cache_read_tokens: 0,
                        cache_creation_tokens: 0,
                        cost_usd: 0.01,
                        count: 1,
                    },
                );
                DailySummary {
                    date: NaiveDate::from_ymd_opt(2025, 1, d).unwrap(),
                    total_input_tokens: 100,
                    total_output_tokens: 50,
                    total_cache_read_tokens: 0,
                    total_cache_creation_tokens: 0,
                    total_cost_usd: 0.01,
                    models,
                }
            })
            .collect();

        let daily_tokens: Vec<(NaiveDate, u64)> = summaries.iter().map(|d| (d.date, 150)).collect();

        let daily_data = DailyData::from_daily_summaries(summaries.clone());
        let stats_data = crate::types::StatsData::from_daily_summaries(&summaries);
        let models_data = super::ModelsData::from_model_usage(&HashMap::new());

        let mut app = App::default();
        let daily_scroll = DailyView::max_scroll_offset(&daily_data, DailyViewMode::Daily);
        let weekly_scroll = DailyView::max_scroll_offset(&daily_data, DailyViewMode::Weekly);
        let monthly_scroll = DailyView::max_scroll_offset(&daily_data, DailyViewMode::Monthly);

        app.state = AppState::Ready {
            data: Box::new(AppData {
                total: crate::types::TotalSummary::default(),
                daily_tokens,
                models_data,
                daily_data,
                stats_data,
                source_usage: vec![],
                cache_warning: None,
            }),
        };
        app.daily_scroll = daily_scroll;
        app.weekly_scroll = weekly_scroll;
        app.monthly_scroll = monthly_scroll;
        app
    }
}
