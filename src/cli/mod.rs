//! CLI command handling

use std::path::PathBuf;

use chrono::Local;
use clap::{Parser, Subcommand};

use crate::report::data::ReportData;
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

    /// Generate a usage report (text or SVG receipt)
    Report {
        /// Last 7 days (default)
        #[arg(long, group = "period")]
        week: bool,

        /// Last 30 days
        #[arg(long, group = "period")]
        month: bool,

        /// Last N days
        #[arg(long, group = "period")]
        days: Option<u32>,

        /// Output SVG file (default: toktrack-report.svg)
        #[arg(long, num_args = 0..=1, default_missing_value = "toktrack-report.svg")]
        svg: Option<PathBuf>,
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
                        initial_view_mode: DailyViewMode::Daily,
                        initial_tab: None,
                    })
                }
            }
            Some(Commands::Stats { json }) => {
                if json {
                    Ok(run_stats_json()?)
                } else {
                    crate::tui::run(TuiConfig {
                        initial_view_mode: DailyViewMode::Daily,
                        initial_tab: Some(Tab::Stats),
                    })
                }
            }
            Some(Commands::Weekly { json }) => {
                if json {
                    Ok(run_weekly_json()?)
                } else {
                    crate::tui::run(TuiConfig {
                        initial_view_mode: DailyViewMode::Weekly,
                        initial_tab: None,
                    })
                }
            }
            Some(Commands::Monthly { json }) => {
                if json {
                    Ok(run_monthly_json()?)
                } else {
                    crate::tui::run(TuiConfig {
                        initial_view_mode: DailyViewMode::Monthly,
                        initial_tab: None,
                    })
                }
            }
            Some(Commands::Report {
                week,
                month,
                days,
                svg,
            }) => Ok(run_report(week, month, days, svg)?),
        }
    }
}

/// Load and process usage data from all CLI parsers.
/// Uses cache-first strategy via DataLoaderService.
fn load_data() -> Result<Vec<DailySummary>> {
    let result = DataLoaderService::new().load()?;
    Ok(result.summaries)
}

/// Load full result including source_usage
fn load_data_full() -> Result<crate::services::data_loader::LoadResult> {
    DataLoaderService::new().load()
}

/// Generate a usage report (text and optionally SVG)
fn run_report(_week: bool, month: bool, days: Option<u32>, svg: Option<PathBuf>) -> Result<()> {
    let n_days = if month { 30 } else { days.unwrap_or(7) };

    let today = Local::now().date_naive();
    let start = today - chrono::Duration::days(n_days as i64 - 1);
    let end = today;

    let result = load_data_full()?;
    let data = ReportData::from_summaries(&result.summaries, &result.source_usage, start, end);

    // Always print text report
    println!("{}", crate::report::text::render(&data));

    // Optionally write SVG
    if let Some(path) = svg {
        let svg_content = crate::report::svg::render(&data);
        std::fs::write(&path, svg_content)?;
        println!("\nSVG saved to: {}", path.display());
    }

    Ok(())
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

    #[test]
    fn test_cli_parse_report_default() {
        let cli = Cli::try_parse_from(["toktrack", "report"]).unwrap();
        assert!(matches!(
            cli.command,
            Some(Commands::Report {
                week: false,
                month: false,
                days: None,
                svg: None,
            })
        ));
    }

    #[test]
    fn test_cli_parse_report_week() {
        let cli = Cli::try_parse_from(["toktrack", "report", "--week"]).unwrap();
        assert!(matches!(
            cli.command,
            Some(Commands::Report {
                week: true,
                month: false,
                days: None,
                svg: None,
            })
        ));
    }

    #[test]
    fn test_cli_parse_report_month() {
        let cli = Cli::try_parse_from(["toktrack", "report", "--month"]).unwrap();
        assert!(matches!(
            cli.command,
            Some(Commands::Report {
                week: false,
                month: true,
                days: None,
                svg: None,
            })
        ));
    }

    #[test]
    fn test_cli_parse_report_days() {
        let cli = Cli::try_parse_from(["toktrack", "report", "--days", "14"]).unwrap();
        assert!(matches!(
            cli.command,
            Some(Commands::Report {
                week: false,
                month: false,
                days: Some(14),
                svg: None,
            })
        ));
    }

    #[test]
    fn test_cli_parse_report_svg_default_path() {
        let cli = Cli::try_parse_from(["toktrack", "report", "--svg"]).unwrap();
        match cli.command {
            Some(Commands::Report { svg, .. }) => {
                assert_eq!(svg, Some(PathBuf::from("toktrack-report.svg")));
            }
            _ => panic!("Expected Report command"),
        }
    }

    #[test]
    fn test_cli_parse_report_svg_custom_path() {
        let cli = Cli::try_parse_from(["toktrack", "report", "--svg", "custom.svg"]).unwrap();
        match cli.command {
            Some(Commands::Report { svg, .. }) => {
                assert_eq!(svg, Some(PathBuf::from("custom.svg")));
            }
            _ => panic!("Expected Report command"),
        }
    }

    #[test]
    fn test_cli_parse_report_week_month_conflict() {
        let result = Cli::try_parse_from(["toktrack", "report", "--week", "--month"]);
        assert!(result.is_err());
    }
}
