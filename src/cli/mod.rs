//! CLI command handling

use clap::{Parser, Subcommand};

use crate::parsers::ParserRegistry;
use crate::services::{Aggregator, PricingService};
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

/// Load and process usage data from all CLI parsers
fn load_data() -> anyhow::Result<Vec<DailySummary>> {
    let registry = ParserRegistry::new();
    let mut all_entries = Vec::new();

    for parser in registry.parsers() {
        match parser.parse_all() {
            Ok(entries) => all_entries.extend(entries),
            Err(e) => {
                eprintln!("[toktrack] Warning: {} failed: {}", parser.name(), e);
            }
        }
    }

    // Calculate cost (graceful fallback)
    let pricing = PricingService::new().ok();
    let entries: Vec<_> = all_entries
        .into_iter()
        .map(|mut e| {
            if e.cost_usd.is_none() {
                if let Some(ref p) = pricing {
                    e.cost_usd = Some(p.calculate_cost(&e));
                }
            }
            e
        })
        .collect();

    Ok(Aggregator::daily(&entries))
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
