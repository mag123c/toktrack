mod cli;
mod parsers;
mod services;
mod tui;
mod types;

use clap::Parser;
use cli::Cli;

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    cli.run()
}
