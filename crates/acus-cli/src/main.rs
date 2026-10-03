mod cmd_find;
mod fmt;

use clap::{Parser, Subcommand};
use fmt::Format;
use std::process::ExitCode;

/// Agent-first code search, outline and editing. Exit: 0 results, 1 nothing found, 2 error.
#[derive(Parser)]
#[command(name = "acus", version)]
struct Cli {
    /// Output format.
    #[arg(long, value_enum, global = true, default_value_t = Format::Agent)]
    format: Format,
    /// Shorthand for --format json.
    #[arg(long, global = true)]
    json: bool,
    /// Shorthand for --format human.
    #[arg(long, global = true)]
    human: bool,
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Regex search; hits grouped by file and enclosing symbol.
    Find(cmd_find::Args),
}

pub enum Outcome {
    Found,
    Empty,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let format = if cli.json {
        Format::Json
    } else if cli.human {
        Format::Human
    } else {
        cli.format
    };
    let out = fmt::Out::new(format);
    let res = match cli.cmd {
        Cmd::Find(a) => cmd_find::run(a, &out),
    };
    match res {
        Ok(Outcome::Found) => ExitCode::SUCCESS,
        Ok(Outcome::Empty) => ExitCode::from(1),
        Err(e) => {
            // Messages carry an optional second line `hint: …`.
            eprintln!("error: {e:#}");
            ExitCode::from(2)
        }
    }
}
