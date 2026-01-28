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
use crate::services::{Aggregator, PricingService};
use crate::types::{CacheWarning, StatsData, TotalSummary};

use super::widgets::{
    daily::{DailyData, DailyView},
    help::HelpPopup,
    models::{ModelsData, ModelsView},
    overview::{Overview, OverviewData},
    spinner::{LoadingStage, Spinner},
    stats::StatsView,
    tabs::Tab,
};

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

/// Main application
pub struct App {
    state: AppState,
    should_quit: bool,
    current_tab: Tab,
    daily_scroll: usize,
    show_help: bool,
}

impl App {
    /// Create a new app in loading state
    pub fn new() -> Self {
        Self {
            state: AppState::Loading {
                spinner_frame: 0,
                stage: LoadingStage::Scanning,
            },
            should_quit: false,
            current_tab: Tab::default(),
            daily_scroll: 0,
            show_help: false,
        }
    }

    /// Load data from parser
    pub fn load_data(&mut self) {
        // Update stage to Parsing
        self.state = AppState::Loading {
            spinner_frame: 0,
            stage: LoadingStage::Parsing,
        };

        let registry = ParserRegistry::new();
        let mut entries = Vec::new();

        for parser in registry.parsers() {
            match parser.parse_all() {
                Ok(parser_entries) => entries.extend(parser_entries),
                Err(e) => {
                    eprintln!("[toktrack] Warning: {} failed: {}", parser.name(), e);
                }
            }
        }

        if entries.is_empty() {
            self.state = AppState::Error {
                message: "No usage data found from any CLI".to_string(),
            };
            return;
        }

        // Calculate costs using PricingService (graceful fallback if unavailable)
        let pricing = PricingService::new().ok();
        let entries: Vec<_> = entries
            .into_iter()
            .map(|mut entry| {
                if entry.cost_usd.is_none() {
                    if let Some(ref pricing) = pricing {
                        entry.cost_usd = Some(pricing.calculate_cost(&entry));
                    }
                }
                entry
            })
            .collect();

        // Update stage to Aggregating
        self.state = AppState::Loading {
            spinner_frame: 0,
            stage: LoadingStage::Aggregating,
        };

        // Get total summary
        let total = Aggregator::total(&entries);

        // Get daily summaries
        let daily_summaries = Aggregator::daily(&entries);

        // Convert to daily tokens for heatmap (all tokens including cache)
        let daily_tokens: Vec<(NaiveDate, u64)> = daily_summaries
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

        // Get model breakdown for Models view
        let model_map = Aggregator::by_model(&entries);
        let models_data = ModelsData::from_model_usage(&model_map);

        // Create StatsData for Stats view (must be before daily_data since summaries are moved)
        let stats_data = StatsData::from_daily_summaries(&daily_summaries);

        // Create DailyData for Daily view (summaries are moved here)
        let daily_data = DailyData::from_daily_summaries(daily_summaries);

        self.state = AppState::Ready {
            data: Box::new(AppData {
                total,
                daily_tokens,
                models_data,
                daily_data,
                stats_data,
                cache_warning: None, // TODO: Integrate with DailySummaryCacheService
            }),
        };
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
                    _ => {}
                }
            }
        }
    }

    /// Scroll up in the current view
    fn scroll_up(&mut self) {
        if self.current_tab == Tab::Daily {
            self.daily_scroll = self.daily_scroll.saturating_sub(1);
        }
    }

    /// Scroll down in the current view
    fn scroll_down(&mut self) {
        if self.current_tab == Tab::Daily {
            if let AppState::Ready { data } = &self.state {
                let max = DailyView::max_scroll_offset(&data.daily_data);
                self.daily_scroll = (self.daily_scroll + 1).min(max);
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
        Self::new()
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
                        let daily_view = DailyView::new(&data.daily_data, self.daily_scroll)
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
    }
}

/// Run the TUI application
pub fn run() -> anyhow::Result<()> {
    let mut terminal = ratatui::init();
    let result = run_app(&mut terminal);
    ratatui::restore();
    result
}

/// Load data synchronously (extracted for background thread)
fn load_data_sync() -> Result<Box<AppData>, String> {
    let registry = ParserRegistry::new();
    let mut entries = Vec::new();

    for parser in registry.parsers() {
        match parser.parse_all() {
            Ok(parser_entries) => entries.extend(parser_entries),
            Err(e) => {
                eprintln!("[toktrack] Warning: {} failed: {}", parser.name(), e);
            }
        }
    }

    if entries.is_empty() {
        return Err("No usage data found from any CLI".to_string());
    }

    // Calculate costs using PricingService (graceful fallback if unavailable)
    let pricing = PricingService::new().ok();
    let entries: Vec<_> = entries
        .into_iter()
        .map(|mut entry| {
            if entry.cost_usd.is_none() {
                if let Some(ref pricing) = pricing {
                    entry.cost_usd = Some(pricing.calculate_cost(&entry));
                }
            }
            entry
        })
        .collect();

    // Get total summary
    let total = Aggregator::total(&entries);

    // Get daily summaries
    let daily_summaries = Aggregator::daily(&entries);

    // Convert to daily tokens for heatmap (all tokens including cache)
    let daily_tokens: Vec<(NaiveDate, u64)> = daily_summaries
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

    // Get model breakdown for Models view
    let model_map = Aggregator::by_model(&entries);
    let models_data = ModelsData::from_model_usage(&model_map);

    // Create StatsData for Stats view (must be before daily_data since summaries are moved)
    let stats_data = StatsData::from_daily_summaries(&daily_summaries);

    // Create DailyData for Daily view (summaries are moved here)
    let daily_data = DailyData::from_daily_summaries(daily_summaries);

    Ok(Box::new(AppData {
        total,
        daily_tokens,
        models_data,
        daily_data,
        stats_data,
        cache_warning: None,
    }))
}

fn run_app(terminal: &mut DefaultTerminal) -> anyhow::Result<()> {
    let mut app = App::new();

    // Spawn background thread for data loading
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let result = load_data_sync();
        let _ = tx.send(result);
    });

    loop {
        terminal.draw(|frame| app.draw(frame))?;

        if app.should_quit() {
            break;
        }

        // Check for data loading completion (non-blocking)
        if matches!(app.state, AppState::Loading { .. }) {
            if let Ok(result) = rx.try_recv() {
                match result {
                    Ok(data) => app.state = AppState::Ready { data },
                    Err(message) => app.state = AppState::Error { message },
                }
            }
        }

        // Poll for events with 100ms timeout for spinner animation
        if event::poll(Duration::from_millis(100))? {
            app.handle_event(event::read()?);
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

    #[test]
    fn test_app_initial_state() {
        let app = App::new();
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
        let mut app = App::new();
        assert!(!app.should_quit());

        let event = Event::Key(KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE));
        app.handle_event(event);

        assert!(app.should_quit());
    }

    #[test]
    fn test_app_quit_on_esc() {
        let mut app = App::new();
        let event = Event::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        app.handle_event(event);
        assert!(app.should_quit());
    }

    #[test]
    fn test_app_tick_updates_spinner() {
        let mut app = App::new();
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
        let mut app = App::new();
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
        let mut app = App::new();
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
        let mut app = App::new();
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
        let mut app = App::new();
        assert!(!app.show_help);

        // Press '?' to show help
        let event = Event::Key(KeyEvent::new(KeyCode::Char('?'), KeyModifiers::NONE));
        app.handle_event(event.clone());
        assert!(app.show_help);

        // Press '?' again to hide help
        app.handle_event(event);
        assert!(!app.show_help);
    }
}
