//! Application state and event loop

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

use crate::parsers::{CLIParser, ClaudeCodeParser};
use crate::services::{Aggregator, PricingService};
use crate::types::TotalSummary;

use super::widgets::{
    overview::{Overview, OverviewData},
    spinner::{LoadingStage, Spinner},
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
}

/// Main application
pub struct App {
    state: AppState,
    should_quit: bool,
    current_tab: Tab,
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
        }
    }

    /// Load data from parser
    pub fn load_data(&mut self) {
        // Update stage to Parsing
        self.state = AppState::Loading {
            spinner_frame: 0,
            stage: LoadingStage::Parsing,
        };

        let parser = ClaudeCodeParser::new();
        let entries = match parser.parse_all() {
            Ok(e) => e,
            Err(e) => {
                self.state = AppState::Error {
                    message: format!("Failed to parse data: {}", e),
                };
                return;
            }
        };

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

        self.state = AppState::Ready {
            data: Box::new(AppData {
                total,
                daily_tokens,
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
                    _ => {}
                }
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
                let today = Local::now().date_naive();
                let overview_data = OverviewData {
                    total: &data.total,
                    daily_tokens: &data.daily_tokens,
                };
                let overview = Overview::new(overview_data, today).with_tab(self.current_tab);
                overview.render(area, buf);
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

fn run_app(terminal: &mut DefaultTerminal) -> anyhow::Result<()> {
    let mut app = App::new();

    // Load data (blocking)
    app.load_data();

    loop {
        terminal.draw(|frame| app.draw(frame))?;

        if app.should_quit() {
            break;
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
}
