//! CLI command handling

use clap::{Parser, Subcommand};

use crate::parsers::ParserRegistry;
use crate::services::{Aggregator, DailySummaryCacheService, PricingService};
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

    /// Show daily usage report
    Daily {
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },

    /// Show usage statistics
    Stats {
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
}

impl Cli {
    pub fn run(self) -> anyhow::Result<()> {
        match self.command {
            None | Some(Commands::Tui) => crate::tui::run(),
            Some(Commands::Daily { json }) => run_daily(json),
            Some(Commands::Stats { json }) => run_stats(json),
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

/// Format number with thousand separators
fn format_tokens(n: u64) -> String {
    let s = n.to_string();
    let mut result = String::with_capacity(s.len() + s.len() / 3);
    for (i, c) in s.chars().rev().enumerate() {
        if i > 0 && i % 3 == 0 {
            result.push(',');
        }
        result.push(c);
    }
    result.chars().rev().collect()
}

/// Run daily command
fn run_daily(json: bool) -> anyhow::Result<()> {
    let mut summaries = load_data()?;

    // Sort by date descending (newest first)
    summaries.sort_by(|a, b| b.date.cmp(&a.date));

    if json {
        println!("{}", serde_json::to_string_pretty(&summaries)?);
    } else {
        print_daily_table(&summaries);
    }

    Ok(())
}

/// Print daily table
fn print_daily_table(summaries: &[DailySummary]) {
    // Header
    println!(
        "{:<12}│{:>10}│{:>10}│{:>10}│{:>10}│{:>10}",
        "Date", "Input", "Output", "Cache", "Total", "Cost"
    );
    println!(
        "{}",
        "─".repeat(12 + 1 + 10 + 1 + 10 + 1 + 10 + 1 + 10 + 1 + 10)
    );

    for summary in summaries {
        let total = summary.total_input_tokens
            + summary.total_output_tokens
            + summary.total_cache_read_tokens
            + summary.total_cache_creation_tokens;
        let cache = summary.total_cache_read_tokens + summary.total_cache_creation_tokens;

        println!(
            "{:<12}│{:>10}│{:>10}│{:>10}│{:>10}│{:>10}",
            summary.date.format("%Y-%m-%d"),
            format_tokens(summary.total_input_tokens),
            format_tokens(summary.total_output_tokens),
            format_tokens(cache),
            format_tokens(total),
            format!("${:.2}", summary.total_cost_usd),
        );
    }
}

/// Run stats command
fn run_stats(json: bool) -> anyhow::Result<()> {
    let summaries = load_data()?;
    let stats = StatsData::from_daily_summaries(&summaries);

    if json {
        println!("{}", serde_json::to_string_pretty(&stats)?);
    } else {
        print_stats(&stats);
    }

    Ok(())
}

/// Print stats text
fn print_stats(stats: &StatsData) {
    println!("Usage Statistics");
    println!("{}", "═".repeat(40));
    println!(
        "{:<25}{:>15}",
        "Total Tokens:",
        format_tokens(stats.total_tokens)
    );
    println!(
        "{:<25}{:>15}",
        "Daily Average:",
        format_tokens(stats.daily_avg_tokens)
    );
    println!(
        "{:<25}{:>15}",
        "Peak Day:",
        stats
            .peak_day
            .map(|(date, tokens)| format!("{} ({})", date, format_tokens(tokens)))
            .unwrap_or_else(|| "N/A".to_string())
    );
    println!(
        "{:<25}{:>15}",
        "Total Cost:",
        format!("${:.2}", stats.total_cost)
    );
    println!(
        "{:<25}{:>15}",
        "Daily Avg Cost:",
        format!("${:.2}", stats.daily_avg_cost)
    );
    println!("{:<25}{:>15}", "Active Days:", stats.active_days);
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
    fn test_format_tokens() {
        assert_eq!(format_tokens(0), "0");
        assert_eq!(format_tokens(999), "999");
        assert_eq!(format_tokens(1000), "1,000");
        assert_eq!(format_tokens(12345), "12,345");
        assert_eq!(format_tokens(1234567), "1,234,567");
        assert_eq!(format_tokens(12345678901), "12,345,678,901");
    }

    #[test]
    fn test_cli_parse_backup_removed() {
        // backup subcommand should no longer exist
        let result = Cli::try_parse_from(["toktrack", "backup"]);
        assert!(result.is_err());
    }
}
