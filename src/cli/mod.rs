//! CLI command handling

use clap::{Parser, Subcommand};

use crate::services::{Aggregator, DataLoaderService};
use crate::tui::widgets::daily::DailyViewMode;
use crate::tui::widgets::tabs::Tab;
use crate::tui::TuiConfig;
use crate::types::{DailySummary, Result, StatsData, ToktrackError};

/// Ultra-fast AI CLI token usage tracker
#[derive(Parser)]
#[command(name = "toktrack")]
#[command(version, about, long_about = None)]
pub struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Launch interactive TUI (default)
    Tui,

    /// Show daily usage (TUI daily tab, or JSON with --json)
    Daily {
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },

    /// Show usage statistics (TUI stats tab, or JSON with --json)
    Stats {
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },

    /// Show weekly usage (TUI daily tab weekly mode, or JSON with --json)
    Weekly {
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },

    /// Show monthly usage (TUI daily tab monthly mode, or JSON with --json)
    Monthly {
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
}

impl Cli {
    pub fn run(self) -> anyhow::Result<()> {
        match self.command {
            None | Some(Commands::Tui) => crate::tui::run(TuiConfig::default()),
            Some(Commands::Daily { json }) => {
                if json {
                    Ok(run_daily_json()?)
                } else {
                    crate::tui::run(TuiConfig {
                        initial_tab: Tab::Daily,
                        initial_view_mode: DailyViewMode::Daily,
                    })
                }
            }
            Some(Commands::Stats { json }) => {
                if json {
                    Ok(run_stats_json()?)
                } else {
                    crate::tui::run(TuiConfig {
                        initial_tab: Tab::Stats,
                        initial_view_mode: DailyViewMode::default(),
                    })
                }
            }
            Some(Commands::Weekly { json }) => {
                if json {
                    Ok(run_weekly_json()?)
                } else {
                    crate::tui::run(TuiConfig {
                        initial_tab: Tab::Daily,
                        initial_view_mode: DailyViewMode::Weekly,
                    })
                }
            }
            Some(Commands::Monthly { json }) => {
                if json {
                    Ok(run_monthly_json()?)
                } else {
                    crate::tui::run(TuiConfig {
                        initial_tab: Tab::Daily,
                        initial_view_mode: DailyViewMode::Monthly,
                    })
                }
            }
        }
    }
}

/// Load and process usage data from all CLI parsers.
/// Uses cache-first strategy via DataLoaderService.
fn load_data() -> Result<Vec<DailySummary>> {
    let result = DataLoaderService::new().load()?;
    Ok(result.summaries)
}

/// Output daily summaries as JSON
fn run_daily_json() -> Result<()> {
    let mut summaries = load_data()?;
    summaries.sort_by(|a, b| b.date.cmp(&a.date));
    println!(
        "{}",
        serde_json::to_string_pretty(&summaries)
            .map_err(|e| ToktrackError::Parse(e.to_string()))?
    );
    Ok(())
}

/// Output weekly summaries as JSON
fn run_weekly_json() -> Result<()> {
    let summaries = load_data()?;
    let mut weekly = Aggregator::weekly(&summaries);
    weekly.sort_by(|a, b| b.date.cmp(&a.date));
    println!(
        "{}",
        serde_json::to_string_pretty(&weekly).map_err(|e| ToktrackError::Parse(e.to_string()))?
    );
    Ok(())
}

/// Output monthly summaries as JSON
fn run_monthly_json() -> Result<()> {
    let summaries = load_data()?;
    let mut monthly = Aggregator::monthly(&summaries);
    monthly.sort_by(|a, b| b.date.cmp(&a.date));
    println!(
        "{}",
        serde_json::to_string_pretty(&monthly).map_err(|e| ToktrackError::Parse(e.to_string()))?
    );
    Ok(())
}

/// Output stats as JSON
fn run_stats_json() -> Result<()> {
    let summaries = load_data()?;
    let stats = StatsData::from_daily_summaries(&summaries);
    println!(
        "{}",
        serde_json::to_string_pretty(&stats).map_err(|e| ToktrackError::Parse(e.to_string()))?
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cli_parse_no_args() {
        let cli = Cli::try_parse_from(["toktrack"]).unwrap();
        assert!(cli.command.is_none());
    }

    #[test]
    fn test_cli_parse_daily() {
        let cli = Cli::try_parse_from(["toktrack", "daily"]).unwrap();
        assert!(matches!(cli.command, Some(Commands::Daily { json: false })));
    }

    #[test]
    fn test_cli_parse_daily_json() {
        let cli = Cli::try_parse_from(["toktrack", "daily", "--json"]).unwrap();
        assert!(matches!(cli.command, Some(Commands::Daily { json: true })));
    }

    #[test]
    fn test_cli_parse_stats() {
        let cli = Cli::try_parse_from(["toktrack", "stats"]).unwrap();
        assert!(matches!(cli.command, Some(Commands::Stats { json: false })));
    }

    #[test]
    fn test_cli_parse_stats_json() {
        let cli = Cli::try_parse_from(["toktrack", "stats", "--json"]).unwrap();
        assert!(matches!(cli.command, Some(Commands::Stats { json: true })));
    }

    #[test]
    fn test_cli_parse_weekly() {
        let cli = Cli::try_parse_from(["toktrack", "weekly"]).unwrap();
        assert!(matches!(
            cli.command,
            Some(Commands::Weekly { json: false })
        ));
    }

    #[test]
    fn test_cli_parse_weekly_json() {
        let cli = Cli::try_parse_from(["toktrack", "weekly", "--json"]).unwrap();
        assert!(matches!(cli.command, Some(Commands::Weekly { json: true })));
    }

    #[test]
    fn test_cli_parse_monthly() {
        let cli = Cli::try_parse_from(["toktrack", "monthly"]).unwrap();
        assert!(matches!(
            cli.command,
            Some(Commands::Monthly { json: false })
        ));
    }

    #[test]
    fn test_cli_parse_monthly_json() {
        let cli = Cli::try_parse_from(["toktrack", "monthly", "--json"]).unwrap();
        assert!(matches!(
            cli.command,
            Some(Commands::Monthly { json: true })
        ));
    }

    #[test]
    fn test_cli_parse_backup_removed() {
        // backup subcommand should no longer exist
        let result = Cli::try_parse_from(["toktrack", "backup"]);
        assert!(result.is_err());
    }
}
