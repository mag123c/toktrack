//! CLI command handling

use std::path::PathBuf;

use chrono::Local;
use clap::{Parser, Subcommand};

use crate::parsers::ParserRegistry;
use crate::report::data::ReportData;
use crate::services::config::{ConfigService, RemoteConfig as StoredRemoteConfig};
use crate::services::{
    audit, Aggregator, DailySummaryCacheService, DataLoaderService, RemoteOptions,
    RemoteSourceService,
};
use crate::tui::widgets::daily::DailyViewMode;
use crate::tui::widgets::tabs::Tab;
use crate::tui::TuiConfig;
use crate::types::{DailySummary, Result, StatsData, ToktrackError};

/// Ultra-fast AI CLI token usage tracker
#[derive(Parser)]
#[command(name = "toktrack")]
#[command(version, about, long_about = None)]
pub struct Cli {
    /// Include a configured SSH remote source. Can be repeated.
    #[arg(long = "remote", global = true, value_name = "NAME")]
    remotes: Vec<String>,

    /// Include every configured remote source.
    #[arg(long, global = true)]
    all_remotes: bool,

    /// Read only local usage data, ignoring configured default remotes.
    #[arg(long, global = true)]
    local_only: bool,

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

    /// Audit data preservation: which days are live on disk vs cache-only
    Audit {
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },

    /// Manage configured SSH remote sources
    Remote {
        #[command(subcommand)]
        command: RemoteCommand,
    },
}

#[derive(Subcommand)]
enum RemoteCommand {
    /// Add a configured SSH remote source
    Add {
        /// Remote name used by toktrack
        name: String,

        /// SSH target, for example user@host or an SSH config alias
        target: String,

        /// Remote Codex sessions path
        #[arg(long, value_name = "PATH")]
        codex_path: Option<String>,

        /// Include this remote by default in normal commands
        #[arg(long = "default")]
        make_default: bool,
    },

    /// Remove a configured SSH remote source
    Remove {
        /// Remote name used by toktrack
        name: String,
    },

    /// List configured SSH remote sources
    List,

    /// Manage the default remote list
    Default {
        #[command(subcommand)]
        command: RemoteDefaultCommand,
    },
}

#[derive(Subcommand)]
enum RemoteDefaultCommand {
    /// Include a configured remote by default
    Add {
        /// Remote name used by toktrack
        name: String,
    },

    /// Stop including a remote by default
    Remove {
        /// Remote name used by toktrack
        name: String,
    },
}

impl Cli {
    pub fn run(self) -> anyhow::Result<()> {
        let remote_options = RemoteOptions {
            remote_names: self.remotes,
            all_remotes: self.all_remotes,
            local_only: self.local_only,
        };

        match self.command {
            None | Some(Commands::Tui) => crate::tui::run(TuiConfig {
                remote_options,
                ..TuiConfig::default()
            }),
            Some(Commands::Daily { json }) => {
                if json {
                    Ok(run_daily_json(&remote_options)?)
                } else {
                    crate::tui::run(TuiConfig {
                        initial_view_mode: DailyViewMode::Daily,
                        initial_tab: None,
                        remote_options,
                    })
                }
            }
            Some(Commands::Stats { json }) => {
                if json {
                    Ok(run_stats_json(&remote_options)?)
                } else {
                    crate::tui::run(TuiConfig {
                        initial_view_mode: DailyViewMode::Daily,
                        initial_tab: Some(Tab::Stats),
                        remote_options,
                    })
                }
            }
            Some(Commands::Weekly { json }) => {
                if json {
                    Ok(run_weekly_json(&remote_options)?)
                } else {
                    crate::tui::run(TuiConfig {
                        initial_view_mode: DailyViewMode::Weekly,
                        initial_tab: None,
                        remote_options,
                    })
                }
            }
            Some(Commands::Monthly { json }) => {
                if json {
                    Ok(run_monthly_json(&remote_options)?)
                } else {
                    crate::tui::run(TuiConfig {
                        initial_view_mode: DailyViewMode::Monthly,
                        initial_tab: None,
                        remote_options,
                    })
                }
            }
            Some(Commands::Report {
                week,
                month,
                days,
                svg,
            }) => Ok(run_report(week, month, days, svg, &remote_options)?),
            Some(Commands::Audit { json }) => Ok(run_audit(json, &remote_options)?),
            Some(Commands::Remote { command }) => Ok(run_remote_config(command)?),
        }
    }
}

fn run_remote_config(command: RemoteCommand) -> Result<()> {
    let path = ConfigService::default_config_path()?;

    match command {
        RemoteCommand::List => {
            let config = ConfigService::load_or_default(&path)?;
            if config.remotes.is_empty() {
                println!("No remote sources configured at {}", path.display());
                return Ok(());
            }

            for remote in &config.remotes {
                let default_marker = if config.default_remotes.contains(&remote.name) {
                    " (default)"
                } else {
                    ""
                };
                println!(
                    "{}{} -> {} (codex: {})",
                    remote.name,
                    default_marker,
                    remote.target,
                    remote.codex_sessions_path()
                );
            }
            Ok(())
        }
        RemoteCommand::Add {
            name,
            target,
            codex_path,
            make_default,
        } => {
            let mut config = ConfigService::load_or_default(&path)?;
            config.add_remote(
                StoredRemoteConfig::new(name.clone(), target.clone(), codex_path),
                make_default,
            )?;
            ConfigService::save(&path, &config)?;
            if make_default {
                println!("Added remote '{}' ({}) as a default remote", name, target);
            } else {
                println!("Added remote '{}' ({})", name, target);
            }
            Ok(())
        }
        RemoteCommand::Remove { name } => {
            let mut config = ConfigService::load_or_default(&path)?;
            config.remove_remote(&name)?;
            ConfigService::save(&path, &config)?;
            println!("Removed remote '{}'", name);
            Ok(())
        }
        RemoteCommand::Default { command } => {
            let mut config = ConfigService::load_or_default(&path)?;
            match command {
                RemoteDefaultCommand::Add { name } => {
                    if config.add_default_remote(&name)? {
                        ConfigService::save(&path, &config)?;
                        println!("Added '{}' to default remotes", name);
                    } else {
                        println!("Remote '{}' is already a default remote", name);
                    }
                }
                RemoteDefaultCommand::Remove { name } => {
                    if config.remove_default_remote(&name)? {
                        ConfigService::save(&path, &config)?;
                        println!("Removed '{}' from default remotes", name);
                    } else {
                        println!("Remote '{}' is not a default remote", name);
                    }
                }
            }
            Ok(())
        }
    }
}

/// Load and process usage data from all CLI parsers.
/// Uses cache-first strategy via DataLoaderService.
fn load_data(remote_options: &RemoteOptions) -> Result<Vec<DailySummary>> {
    let result = load_data_full(remote_options)?;
    Ok(result.summaries)
}

/// Load full result including source_usage
fn load_data_full(
    remote_options: &RemoteOptions,
) -> Result<crate::services::data_loader::LoadResult> {
    let extra_sources = RemoteSourceService::sync_and_build_sources(remote_options)?;
    DataLoaderService::with_extra_sources(extra_sources).load()
}

/// Generate a usage report (text and optionally SVG)
fn run_report(
    _week: bool,
    month: bool,
    days: Option<u32>,
    svg: Option<PathBuf>,
    remote_options: &RemoteOptions,
) -> Result<()> {
    let n_days = if month { 30 } else { days.unwrap_or(7) };

    let today = Local::now().date_naive();
    let start = today - chrono::Duration::days(n_days as i64 - 1);
    let end = today;

    let result = load_data_full(remote_options)?;
    let mut data =
        ReportData::from_summaries(&result.summaries, &result.source_summaries, start, end);
    // Carry per-source "estimated" (LiteLLM-calculated cost) onto the report.
    let estimated: std::collections::HashMap<&str, bool> = result
        .source_usage
        .iter()
        .map(|s| (s.source.as_str(), s.estimated))
        .collect();
    for sr in &mut data.by_source {
        sr.estimated = estimated.get(sr.name.as_str()).copied().unwrap_or(false);
    }

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

/// Audit data preservation across all sources (text, or JSON with --json).
fn run_audit(json: bool, remote_options: &RemoteOptions) -> Result<()> {
    let extra_sources = RemoteSourceService::sync_and_build_sources(remote_options)?;
    let registry = ParserRegistry::with_extra_sources(extra_sources);
    let cache = DailySummaryCacheService::new()?;
    let today = Local::now().date_naive();
    let report = audit::build_report(registry.sources(), &cache, today);

    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&report)
                .map_err(|e| ToktrackError::Parse(e.to_string()))?
        );
    } else {
        print!("{}", crate::report::audit::render(&report));
    }
    Ok(())
}

/// Output daily summaries as JSON
fn run_daily_json(remote_options: &RemoteOptions) -> Result<()> {
    let mut summaries = load_data(remote_options)?;
    summaries.sort_by_key(|b| std::cmp::Reverse(b.date));
    println!(
        "{}",
        serde_json::to_string_pretty(&summaries)
            .map_err(|e| ToktrackError::Parse(e.to_string()))?
    );
    Ok(())
}

/// Output weekly summaries as JSON
fn run_weekly_json(remote_options: &RemoteOptions) -> Result<()> {
    let summaries = load_data(remote_options)?;
    let mut weekly = Aggregator::weekly(&summaries);
    weekly.sort_by_key(|b| std::cmp::Reverse(b.date));
    println!(
        "{}",
        serde_json::to_string_pretty(&weekly).map_err(|e| ToktrackError::Parse(e.to_string()))?
    );
    Ok(())
}

/// Output monthly summaries as JSON
fn run_monthly_json(remote_options: &RemoteOptions) -> Result<()> {
    let summaries = load_data(remote_options)?;
    let mut monthly = Aggregator::monthly(&summaries);
    monthly.sort_by_key(|b| std::cmp::Reverse(b.date));
    println!(
        "{}",
        serde_json::to_string_pretty(&monthly).map_err(|e| ToktrackError::Parse(e.to_string()))?
    );
    Ok(())
}

/// Output stats as JSON
fn run_stats_json(remote_options: &RemoteOptions) -> Result<()> {
    let summaries = load_data(remote_options)?;
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
        assert!(cli.remotes.is_empty());
        assert!(!cli.all_remotes);
        assert!(!cli.local_only);
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

    #[test]
    fn test_cli_parse_audit() {
        let cli = Cli::try_parse_from(["toktrack", "audit"]).unwrap();
        assert!(matches!(cli.command, Some(Commands::Audit { json: false })));
    }

    #[test]
    fn test_cli_parse_audit_json() {
        let cli = Cli::try_parse_from(["toktrack", "audit", "--json"]).unwrap();
        assert!(matches!(cli.command, Some(Commands::Audit { json: true })));
    }

    #[test]
    fn test_cli_parse_remote_global_before_command() {
        let cli = Cli::try_parse_from(["toktrack", "--remote", "devbox", "report"]).unwrap();
        assert_eq!(cli.remotes, vec!["devbox"]);
        assert!(matches!(cli.command, Some(Commands::Report { .. })));
    }

    #[test]
    fn test_cli_parse_remote_global_after_command() {
        let cli = Cli::try_parse_from(["toktrack", "report", "--remote", "devbox"]).unwrap();
        assert_eq!(cli.remotes, vec!["devbox"]);
        assert!(matches!(cli.command, Some(Commands::Report { .. })));
    }

    #[test]
    fn test_cli_parse_all_remotes() {
        let cli = Cli::try_parse_from(["toktrack", "--all-remotes", "daily", "--json"]).unwrap();
        assert!(cli.all_remotes);
        assert!(matches!(cli.command, Some(Commands::Daily { json: true })));
    }

    #[test]
    fn test_cli_parse_local_only() {
        let cli = Cli::try_parse_from(["toktrack", "--local-only", "stats"]).unwrap();
        assert!(cli.local_only);
        assert!(matches!(cli.command, Some(Commands::Stats { json: false })));
    }

    #[test]
    fn test_cli_parse_remote_add_default() {
        let cli = Cli::try_parse_from([
            "toktrack",
            "remote",
            "add",
            "devbox",
            "ubuntu@devbox",
            "--codex-path",
            "/home/ubuntu/.codex/sessions",
            "--default",
        ])
        .unwrap();

        assert!(matches!(
            cli.command,
            Some(Commands::Remote {
                command: RemoteCommand::Add {
                    name,
                    target,
                    codex_path: Some(path),
                    make_default: true,
                },
            }) if name == "devbox"
                && target == "ubuntu@devbox"
                && path == "/home/ubuntu/.codex/sessions"
        ));
    }

    #[test]
    fn test_cli_parse_remote_remove() {
        let cli = Cli::try_parse_from(["toktrack", "remote", "remove", "devbox"]).unwrap();
        assert!(matches!(
            cli.command,
            Some(Commands::Remote {
                command: RemoteCommand::Remove { name },
            }) if name == "devbox"
        ));
    }

    #[test]
    fn test_cli_parse_remote_default_add() {
        let cli = Cli::try_parse_from(["toktrack", "remote", "default", "add", "devbox"]).unwrap();
        assert!(matches!(
            cli.command,
            Some(Commands::Remote {
                command: RemoteCommand::Default {
                    command: RemoteDefaultCommand::Add { name },
                },
            }) if name == "devbox"
        ));
    }

    #[test]
    fn test_cli_parse_remote_default_remove() {
        let cli =
            Cli::try_parse_from(["toktrack", "remote", "default", "remove", "devbox"]).unwrap();
        assert!(matches!(
            cli.command,
            Some(Commands::Remote {
                command: RemoteCommand::Default {
                    command: RemoteDefaultCommand::Remove { name },
                },
            }) if name == "devbox"
        ));
    }
}
