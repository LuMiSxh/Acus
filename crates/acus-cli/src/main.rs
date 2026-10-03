mod cmd_ctx;
mod cmd_decide;
mod cmd_diff;
mod cmd_find;
mod cmd_outline;
mod cmd_patch;
mod cmd_run;
mod cmd_show;
mod cmd_usage;
mod config;
mod fmt;

use anyhow::{Result, bail};
use clap::{Parser, Subcommand};
use fmt::Format;
use std::process::ExitCode;

/// Agent-first code search, outline and editing. Exit: 0 results, 1 nothing found, 2 error.
#[derive(Parser)]
#[command(name = "acus", version)]
pub struct Cli {
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
    /// Apply a patch from stdin (Codex apply_patch format, SEARCH/REPLACE blocks, `*** Replace Symbol:`,
    /// `*** Delete Symbol:`, `*** Move Symbol:` and `*** Replace All:`).
    Patch(cmd_patch::Args),
    /// Token, cost and tool-call analytics over Claude Code and Codex transcripts.
    Usage(cmd_usage::Args),
    /// Yes/no, choice or score judgement via a Jev-compatible API (needs `cmd-decide`).
    Decide(cmd_decide::Args),
    /// Run a shell command; print its output plus the code its `path:line` references point at.
    Ctx(cmd_ctx::Args),
    /// Uncommitted (or `REV`, `A..B`) changes grouped by enclosing symbol; `-p` adds the lines.
    Diff(cmd_diff::Args),
    /// Run several commands from a JSON array of argument lists on stdin.
    Run(cmd_run::Args),
}

impl Cmd {
    fn name(&self) -> &'static str {
        match self {
            Cmd::Find(_) => "find",
            Cmd::Outline(_) => "outline",
            Cmd::Show(_) => "show",
            Cmd::Patch(_) => "patch",
            Cmd::Usage(_) => "usage",
            Cmd::Decide(_) => "decide",
            Cmd::Run(_) => "run",
            Cmd::Ctx(_) => "ctx",
            Cmd::Diff(_) => "diff",
        }
    }
}

pub enum Outcome {
    Found,
    Empty,
    /// Pass through a child command's exit code.
    Exit(u8),
}

impl Cli {
    fn format(&self) -> Format {
        match (self.json, self.human) {
            (true, _) => Format::Json,
            (_, true) => Format::Human,
            _ => self.format,
        }
    }
}

/// Runs one parsed command line; `run` calls this for every batch entry.
pub fn execute(cli: Cli, nested: bool) -> Result<Outcome> {
    let cfg = config::load()?;
    let name = cli.cmd.name();
    if cfg.commands.disabled.iter().any(|d| d == name) {
        bail!(
            "acus {name} is disabled by configuration\nhint: remove it from [commands] disabled in {} or from ACUS_DISABLE",
            cfg.path.display()
        );
    }
    let out = fmt::Out::new(cli.format());
    match cli.cmd {
        Cmd::Find(a) => cmd_find::run(a, &out),
        Cmd::Outline(a) => cmd_outline::run(a, &out),
        Cmd::Show(a) => cmd_show::run(a, &out),
        Cmd::Patch(a) => cmd_patch::run(a, &out, nested, &cfg),
        Cmd::Usage(a) => cmd_usage::run(a, &out),
        Cmd::Decide(a) => cmd_decide::run(a, &out, &cfg),
        Cmd::Run(_) if nested => bail!("`run` cannot be nested"),
        Cmd::Run(a) => cmd_run::run(a, &out),
        Cmd::Ctx(a) => cmd_ctx::run(a, &out, &cfg),
        Cmd::Diff(a) => cmd_diff::run(a, &out),
    }
}

fn main() -> ExitCode {
    match execute(Cli::parse(), false) {
        Ok(Outcome::Found) => ExitCode::SUCCESS,
        Ok(Outcome::Empty) => ExitCode::from(1),
        Ok(Outcome::Exit(c)) => ExitCode::from(c),
        Err(e) => {
            // Messages carry an optional second line `hint: …`.
            eprintln!("error: {e:#}");
            ExitCode::from(2)
        }
    }
}
