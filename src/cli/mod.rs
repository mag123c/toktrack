//! CLI command handling

use clap::{Parser, Subcommand};

use crate::parsers::ParserRegistry;
use crate::services::{Aggregator, DailySummaryCacheService, PricingService};
use crate::tui::widgets::daily::DailyViewMode;
use crate::tui::widgets::tabs::Tab;
use crate::tui::TuiConfig;
use crate::types::{DailySummary, StatsData};

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
                    run_daily_json()
                } else {
                    crate::tui::run(TuiConfig {
                        initial_tab: Tab::Daily,
                        initial_view_mode: DailyViewMode::Daily,
                    })
                }
            }
            Some(Commands::Stats { json }) => {
                if json {
                    run_stats_json()
                } else {
                    crate::tui::run(TuiConfig {
                        initial_tab: Tab::Stats,
                        initial_view_mode: DailyViewMode::default(),
                    })
                }
            }
            Some(Commands::Weekly { json }) => {
                if json {
                    run_weekly_json()
                } else {
                    crate::tui::run(TuiConfig {
                        initial_tab: Tab::Daily,
                        initial_view_mode: DailyViewMode::Weekly,
                    })
                }
            }
            Some(Commands::Monthly { json }) => {
                if json {
                    run_monthly_json()
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
/// Uses cache-first strategy matching the TUI pipeline.
fn load_data() -> anyhow::Result<Vec<DailySummary>> {
    let registry = ParserRegistry::new();
    let cache_service = DailySummaryCacheService::new().ok();
    let pricing = PricingService::from_cache_only();

    let has_cache = cache_service.as_ref().is_some_and(|cs| {
        registry
            .parsers()
            .iter()
            .any(|p| cs.cache_path(p.name()).exists())
    });

    let mut all_summaries = Vec::new();

    if has_cache {
        // Warm path: recent files + cache
        let since = std::time::SystemTime::now() - std::time::Duration::from_secs(24 * 3600);

        for parser in registry.parsers() {
            let entries = match parser.parse_recent_files(since) {
                Ok(e) => e,
                Err(e) => {
                    eprintln!("[toktrack] Warning: {} failed: {}", parser.name(), e);
                    continue;
                }
            };

            let entries: Vec<_> = entries
                .into_iter()
                .map(|mut entry| {
                    if entry.cost_usd.is_none() {
                        if let Some(ref p) = pricing {
                            entry.cost_usd = Some(p.calculate_cost(&entry));
                        }
                    }
                    entry
                })
                .collect();

            if let Some(ref cs) = cache_service {
                match cs.load_or_compute(parser.name(), &entries) {
                    Ok((summaries, _)) => all_summaries.extend(summaries),
                    Err(e) => {
                        eprintln!(
                            "[toktrack] Warning: cache for {} failed: {}",
                            parser.name(),
                            e
                        );
                    }
                }
            }
        }

        if !all_summaries.is_empty() {
            all_summaries.sort_by_key(|s| s.date);
            return Ok(all_summaries);
        }
        // Fall through to cold path if warm produced nothing
    }

    // Cold path: full parse
    let fallback_pricing = if pricing.is_none() {
        PricingService::new().ok()
    } else {
        None
    };
    let pricing_ref = pricing.as_ref().or(fallback_pricing.as_ref());

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

        if let Some(ref cs) = cache_service {
            match cs.load_or_compute(parser.name(), &entries) {
                Ok((summaries, _)) => {
                    all_summaries.extend(summaries);
                    continue;
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

        all_summaries.extend(Aggregator::daily(&entries));
    }

    all_summaries.sort_by_key(|s| s.date);
    Ok(all_summaries)
}

/// Output daily summaries as JSON
fn run_daily_json() -> anyhow::Result<()> {
    let mut summaries = load_data()?;
    summaries.sort_by(|a, b| b.date.cmp(&a.date));
    println!("{}", serde_json::to_string_pretty(&summaries)?);
    Ok(())
}

/// Output weekly summaries as JSON
fn run_weekly_json() -> anyhow::Result<()> {
    let summaries = load_data()?;
    let mut weekly = Aggregator::weekly(&summaries);
    weekly.sort_by(|a, b| b.date.cmp(&a.date));
    println!("{}", serde_json::to_string_pretty(&weekly)?);
    Ok(())
}

/// Output monthly summaries as JSON
fn run_monthly_json() -> anyhow::Result<()> {
    let summaries = load_data()?;
    let mut monthly = Aggregator::monthly(&summaries);
    monthly.sort_by(|a, b| b.date.cmp(&a.date));
    println!("{}", serde_json::to_string_pretty(&monthly)?);
    Ok(())
}

/// Output stats as JSON
fn run_stats_json() -> anyhow::Result<()> {
    let summaries = load_data()?;
    let stats = StatsData::from_daily_summaries(&summaries);
    println!("{}", serde_json::to_string_pretty(&stats)?);
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
