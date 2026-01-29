//! Application state and event loop

use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use chrono::{Local, NaiveDate};
use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Style},
    widgets::Widget,
    DefaultTerminal, Frame,
};

use crate::parsers::ParserRegistry;
use crate::services::update_checker::{check_for_update, execute_update, UpdateCheckResult};
use crate::services::{Aggregator, DailySummaryCacheService, PricingService};
use crate::types::{CacheWarning, StatsData, TotalSummary};

use super::widgets::{
    daily::{DailyData, DailyView, DailyViewMode},
    help::HelpPopup,
    models::{ModelsData, ModelsView},
    overview::{Overview, OverviewData},
    spinner::{LoadingStage, Spinner},
    stats::StatsView,
    tabs::Tab,
    update_popup::{UpdateMessagePopup, UpdatePopup},
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
    daily_view_mode: DailyViewMode,
    show_help: bool,
    update_status: UpdateStatus,
    pending_data: Option<Result<Box<AppData>, String>>,
}

impl App {
    /// Create a new app in loading state with the given configuration
    pub fn new(config: TuiConfig) -> Self {
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
            daily_view_mode: config.initial_view_mode,
            show_help: false,
            update_status: UpdateStatus::Checking,
            pending_data: None,
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

    /// Handle keyboard events
    pub fn handle_event(&mut self, event: Event) {
        if let Event::Key(key) = event {
            if key.kind == KeyEventKind::Press {
                match key.code {
                    KeyCode::Char('q') | KeyCode::Char('Q') | KeyCode::Esc => {
                        self.should_quit = true;
                    }
                    KeyCode::Tab => {
                        self.current_tab = self.current_tab.next();
                    }
                    KeyCode::BackTab => {
                        self.current_tab = self.current_tab.prev();
                    }
                    KeyCode::Up | KeyCode::Char('k') => {
                        self.scroll_up();
                    }
                    KeyCode::Down | KeyCode::Char('j') => {
                        self.scroll_down();
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

    /// Handle keyboard events when update overlay is displayed
    pub fn handle_update_event(&mut self, event: Event) {
        if let Event::Key(key) = event {
            if key.kind == KeyEventKind::Press {
                match (&self.update_status, key.code) {
                    // Available state: u triggers update, q/Esc quits, any other key skips
                    (UpdateStatus::Available { .. }, KeyCode::Char('u') | KeyCode::Char('U')) => {
                        self.update_status = UpdateStatus::Updating;
                    }
                    (
                        UpdateStatus::Available { .. },
                        KeyCode::Char('q') | KeyCode::Char('Q') | KeyCode::Esc,
                    ) => {
                        self.should_quit = true;
                    }
                    (UpdateStatus::Available { .. }, _) => {
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

    /// Scroll up in the current view
    fn scroll_up(&mut self) {
        if self.current_tab == Tab::Daily {
            let scroll = self.active_scroll_mut();
            *scroll = scroll.saturating_sub(1);
        }
    }

    /// Scroll down in the current view
    fn scroll_down(&mut self) {
        if self.current_tab == Tab::Daily {
            let mode = self.daily_view_mode;
            if let AppState::Ready { data } = &self.state {
                let max = DailyView::max_scroll_offset(&data.daily_data, mode);
                let scroll = self.active_scroll_mut();
                *scroll = (*scroll + 1).min(max);
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
        Self::new(TuiConfig::default())
    }
}

impl Widget for &App {
    fn render(self, area: Rect, buf: &mut Buffer) {
        match &self.state {
            AppState::Loading {
                spinner_frame,
                stage,
            } => {
                let spinner = Spinner::new(*spinner_frame, *stage);
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
                        };
                        let overview =
                            Overview::new(overview_data, today).with_tab(self.current_tab);
                        overview.render(area, buf);
                    }
                    Tab::Models => {
                        let models_view =
                            ModelsView::new(&data.models_data).with_tab(self.current_tab);
                        models_view.render(area, buf);
                    }
                    Tab::Daily => {
                        let daily_view = DailyView::new(
                            &data.daily_data,
                            self.active_scroll(),
                            self.daily_view_mode,
                        )
                        .with_tab(self.current_tab);
                        daily_view.render(area, buf);
                    }
                    Tab::Stats => {
                        let stats_view =
                            StatsView::new(&data.stats_data).with_tab(self.current_tab);
                        stats_view.render(area, buf);
                    }
                }

                // Render help popup overlay if active
                if self.show_help {
                    let popup_area = HelpPopup::centered_area(area);
                    HelpPopup::new().render(popup_area, buf);
                }
            }
            AppState::Error { message } => {
                let y = area.y + area.height / 2;
                let text = format!("Error: {}", message);
                let x = area.x + (area.width.saturating_sub(text.len() as u16)) / 2;
                buf.set_string(x, y, &text, Style::default().fg(Color::Red));
            }
        }

        // Render update overlay on top of everything (works in both Loading and Ready states)
        match &self.update_status {
            UpdateStatus::Available { current, latest } => {
                let popup_area = UpdatePopup::centered_area(area);
                UpdatePopup::new(current, latest).render(popup_area, buf);
            }
            UpdateStatus::Updating => {
                let popup_area = UpdateMessagePopup::centered_area(area);
                UpdateMessagePopup::new("Running npm update -g toktrack...", Color::Yellow)
                    .render(popup_area, buf);
            }
            UpdateStatus::UpdateDone { success, message } => {
                let popup_area = UpdateMessagePopup::centered_area(area);
                let color = if *success { Color::Green } else { Color::Red };
                UpdateMessagePopup::new(message, color).render(popup_area, buf);
            }
            UpdateStatus::Checking | UpdateStatus::Resolved => {}
        }
    }
}

/// Run the TUI application with the given configuration
pub fn run(config: TuiConfig) -> anyhow::Result<()> {
    let mut terminal = ratatui::init();
    let result = run_app(&mut terminal, config);
    ratatui::restore();
    result
}

/// Load data synchronously (extracted for background thread).
/// Uses cache-first strategy: warm path parses only recent files,
/// cold path falls back to full parse_all().
fn load_data_sync() -> Result<Box<AppData>, String> {
    let registry = ParserRegistry::new();
    let cache_service = DailySummaryCacheService::new().ok();

    // Determine if we have cache available (warm vs cold path)
    let has_cache = cache_service.as_ref().is_some_and(|cs| {
        registry
            .parsers()
            .iter()
            .any(|p| cs.cache_path(p.name()).exists())
    });

    // Non-blocking pricing: load from cache only (no network call)
    let pricing = PricingService::from_cache_only();

    if has_cache {
        // === WARM PATH: cache + recent files only ===
        load_warm_path(&registry, cache_service.as_ref().unwrap(), pricing.as_ref())
    } else {
        // === COLD PATH: full parse, build cache for next run ===
        load_cold_path(&registry, cache_service.as_ref(), pricing.as_ref())
    }
}

/// Warm path: use cached DailySummaries + parse only recent files for today.
fn load_warm_path(
    registry: &ParserRegistry,
    cache_service: &DailySummaryCacheService,
    pricing: Option<&PricingService>,
) -> Result<Box<AppData>, String> {
    let since = std::time::SystemTime::now() - std::time::Duration::from_secs(24 * 3600);

    let mut all_summaries = Vec::new();
    let mut cache_warning = None;

    for parser in registry.parsers() {
        // Parse only recently modified files
        let entries = match parser.parse_recent_files(since) {
            Ok(e) => e,
            Err(e) => {
                eprintln!("[toktrack] Warning: {} failed: {}", parser.name(), e);
                continue;
            }
        };

        // Apply pricing to entries missing cost
        let entries: Vec<_> = entries
            .into_iter()
            .map(|mut entry| {
                if entry.cost_usd.is_none() {
                    if let Some(p) = pricing {
                        entry.cost_usd = Some(p.calculate_cost(&entry));
                    }
                }
                entry
            })
            .collect();

        // Merge with cache: cached past days + recomputed today
        match cache_service.load_or_compute(parser.name(), &entries) {
            Ok((summaries, warning)) => {
                if warning.is_some() && cache_warning.is_none() {
                    cache_warning = warning;
                }
                all_summaries.extend(summaries);
            }
            Err(e) => {
                eprintln!(
                    "[toktrack] Warning: cache for {} failed: {}",
                    parser.name(),
                    e
                );
            }
        }
    }

    if all_summaries.is_empty() {
        // Warm path produced nothing → fallback to cold path
        return load_cold_path(registry, Some(cache_service), pricing);
    }

    all_summaries.sort_by_key(|s| s.date);
    build_app_data_from_summaries(all_summaries, cache_warning)
}

/// Cold path: full parse_all() per parser + build cache for next run.
fn load_cold_path(
    registry: &ParserRegistry,
    cache_service: Option<&DailySummaryCacheService>,
    pricing: Option<&PricingService>,
) -> Result<Box<AppData>, String> {
    // Apply pricing (try cache-only first, fall back to full network fetch)
    let fallback_pricing;
    let pricing_ref = match pricing {
        Some(p) => Some(p),
        None => {
            fallback_pricing = PricingService::new().ok();
            fallback_pricing.as_ref()
        }
    };

    let mut all_summaries = Vec::new();
    let mut cache_warning = None;
    let mut any_entries = false;

    for parser in registry.parsers() {
        let entries = match parser.parse_all() {
            Ok(e) => e,
            Err(e) => {
                eprintln!("[toktrack] Warning: {} failed: {}", parser.name(), e);
                continue;
            }
        };

        if entries.is_empty() {
            continue;
        }
        any_entries = true;

        // Apply pricing
        let entries: Vec<_> = entries
            .into_iter()
            .map(|mut entry| {
                if entry.cost_usd.is_none() {
                    if let Some(p) = pricing_ref {
                        entry.cost_usd = Some(p.calculate_cost(&entry));
                    }
                }
                entry
            })
            .collect();

        // Build cache for this parser (for next warm path)
        if let Some(cs) = cache_service {
            match cs.load_or_compute(parser.name(), &entries) {
                Ok((summaries, warning)) => {
                    if warning.is_some() && cache_warning.is_none() {
                        cache_warning = warning;
                    }
                    all_summaries.extend(summaries);
                    continue; // Used cache-backed summaries
                }
                Err(e) => {
                    eprintln!(
                        "[toktrack] Warning: cache for {} failed: {}",
                        parser.name(),
                        e
                    );
                }
            }
        }

        // Cache unavailable: compute summaries directly from entries
        all_summaries.extend(Aggregator::daily(&entries));
    }

    if !any_entries {
        return Err("No usage data found from any CLI".to_string());
    }

    all_summaries.sort_by_key(|s| s.date);
    build_app_data_from_summaries(all_summaries, cache_warning)
}

/// Build AppData from DailySummary list (no raw entries needed).
fn build_app_data_from_summaries(
    summaries: Vec<crate::types::DailySummary>,
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
        cache_warning,
    }))
}

fn run_app(terminal: &mut DefaultTerminal, config: TuiConfig) -> anyhow::Result<()> {
    let mut app = App::new(config);

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
            if app.update_status.shows_overlay() {
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
    fn test_app_quit_on_q() {
        let mut app = App::default();
        assert!(!app.should_quit());

        let event = Event::Key(KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE));
        app.handle_event(event);

        assert!(app.should_quit());
    }

    #[test]
    fn test_app_quit_on_esc() {
        let mut app = App::default();
        let event = Event::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        app.handle_event(event);
        assert!(app.should_quit());
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
    fn test_independent_scroll_positions() {
        let mut app = make_ready_app();
        app.current_tab = Tab::Daily;

        // Record initial scroll positions (all at max)
        let initial_daily = app.daily_scroll;
        let initial_weekly = app.weekly_scroll;

        // Scroll up in Daily mode
        app.handle_event(Event::Key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE)));
        assert_eq!(app.daily_scroll, initial_daily.saturating_sub(1));
        // Weekly scroll should be unchanged
        assert_eq!(app.weekly_scroll, initial_weekly);

        // Switch to Weekly and scroll up
        app.daily_view_mode = DailyViewMode::Weekly;
        app.handle_event(Event::Key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE)));
        assert_eq!(app.weekly_scroll, initial_weekly.saturating_sub(1));
        // Daily scroll should remain as modified
        assert_eq!(app.daily_scroll, initial_daily.saturating_sub(1));
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
    fn test_update_overlay_skip_any_key() {
        let mut app = make_update_available_app();

        // Press space → should skip (resolve)
        let event = Event::Key(KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE));
        app.handle_update_event(event);

        assert_eq!(app.update_status, UpdateStatus::Resolved);
        assert!(!app.should_quit());
    }

    #[test]
    fn test_update_overlay_u_triggers_update() {
        let mut app = make_update_available_app();

        // Press 'u' → should trigger update
        let event = Event::Key(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::NONE));
        app.handle_update_event(event);

        assert_eq!(app.update_status, UpdateStatus::Updating);
    }

    #[test]
    fn test_update_overlay_quit_still_works() {
        let mut app = make_update_available_app();

        // Press 'q' → should quit
        let event = Event::Key(KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE));
        app.handle_update_event(event);

        assert!(app.should_quit());
    }

    #[test]
    fn test_update_overlay_esc_quits() {
        let mut app = make_update_available_app();

        let event = Event::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        app.handle_update_event(event);

        assert!(app.should_quit());
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
            cache_warning: None,
        })));

        // Skip update overlay
        let event = Event::Key(KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE));
        app.handle_update_event(event);

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
}
