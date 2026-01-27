use clap::{Parser, Subcommand};

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

    /// Manually backup data
    Backup,
}

impl Cli {
    pub fn run(self) -> anyhow::Result<()> {
        match self.command {
            None | Some(Commands::Tui) => crate::tui::run(),
            Some(Commands::Daily { json }) => {
                if json {
                    println!("{{\"message\": \"Daily JSON not yet implemented\"}}");
                } else {
                    println!("Daily report not yet implemented");
                }
                Ok(())
            }
            Some(Commands::Stats { json }) => {
                if json {
                    println!("{{\"message\": \"Stats JSON not yet implemented\"}}");
                } else {
                    println!("Stats not yet implemented");
                }
                Ok(())
            }
            Some(Commands::Backup) => {
                println!("Backup not yet implemented");
                Ok(())
            }
        }
    }
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
}
