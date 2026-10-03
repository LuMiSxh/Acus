mod cmd_find;
mod cmd_outline;
mod cmd_patch;
mod cmd_show;
mod cmd_usage;
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
    /// Symbols with kinds and line ranges; directories are walked.
    Outline(cmd_outline::Args),
    /// Print files, line ranges or symbols with line numbers.
    Show(cmd_show::Args),
    /// Apply a patch from stdin (Codex apply_patch format + `*** Replace Symbol: path#Sym`).
    Patch(cmd_patch::Args),
    /// Token, cost and tool-call analytics over Claude Code and Codex transcripts.
    Usage(cmd_usage::Args),
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
        Cmd::Outline(a) => cmd_outline::run(a, &out),
        Cmd::Show(a) => cmd_show::run(a, &out),
        Cmd::Patch(a) => cmd_patch::run(a, &out),
        Cmd::Usage(a) => cmd_usage::run(a, &out),
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
