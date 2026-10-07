mod cmd_ctx;
mod cmd_decide;
mod cmd_diff;
mod cmd_find;
mod cmd_guard;
mod cmd_log;
mod cmd_map;
mod cmd_outline;
mod cmd_patch;
mod cmd_run;
mod cmd_show;
mod cmd_skill;
mod cmd_update;
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
    /// Directory tree with file and line counts, opened as far as a line budget allows.
    Map(cmd_map::Args),
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
    /// Recent commits, one line each with date, author and changed lines.
    Log(cmd_log::Args),
    /// Run several commands from a JSON array of argument lists on stdin.
    Run(cmd_run::Args),
    /// Print the agent skill (for a SessionStart hook) or install it with `--install`.
    Skill(cmd_skill::Args),
    /// Claude Code PreToolUse hook: refuses grep, cat, sed -n and sed -i on source files with the acus command to use.
    Guard(cmd_guard::Args),
    /// Replace this binary with the latest GitHub release and reinstall the skill.
    Update(cmd_update::Args),
}

impl Cmd {
    fn name(&self) -> &'static str {
        match self {
            Cmd::Find(_) => "find",
            Cmd::Outline(_) => "outline",
            Cmd::Map(_) => "map",
            Cmd::Show(_) => "show",
            Cmd::Patch(_) => "patch",
            Cmd::Usage(_) => "usage",
            Cmd::Decide(_) => "decide",
            Cmd::Run(_) => "run",
            Cmd::Ctx(_) => "ctx",
            Cmd::Diff(_) => "diff",
            Cmd::Log(_) => "log",
            Cmd::Skill(_) => "skill",
            Cmd::Guard(_) => "guard",
            Cmd::Update(_) => "update",
        }
    }
}

pub enum Outcome {
    Found,
    Empty,
    NoChanges,
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
    let result = match cli.cmd {
        Cmd::Find(a) => cmd_find::run(a, &out),
        Cmd::Outline(a) => cmd_outline::run(a, &out),
        Cmd::Map(a) => cmd_map::run(a),
        Cmd::Show(a) => cmd_show::run(a, &out),
        Cmd::Patch(a) => cmd_patch::run(a, &out, nested, &cfg),
        Cmd::Usage(a) => cmd_usage::run(a, &out, &cfg),
        Cmd::Decide(a) => cmd_decide::run(a, &out, &cfg),
        Cmd::Run(_) if nested => bail!("`run` cannot be nested"),
        Cmd::Run(a) => cmd_run::run(a, &out),
        Cmd::Ctx(a) => cmd_ctx::run(a, &out, &cfg),
        Cmd::Diff(a) => cmd_diff::run(a, &out),
        Cmd::Log(a) => cmd_log::run(a, &out),
        Cmd::Skill(a) => cmd_skill::run(a),
        Cmd::Guard(a) => cmd_guard::run(a, &cfg),
        Cmd::Update(a) => cmd_update::run(a),
    }?;
    if !nested
        && out.format == Format::Json
        && matches!(
            (name, &result),
            ("find" | "log", Outcome::Empty) | ("diff", Outcome::NoChanges)
        )
    {
        println!("null");
    }
    Ok(result)
}

/// A hint for a grep or rg flag that `acus find` lacks, printed after clap's own error; `args`
/// is the whole command line, `acus` first.
pub fn find_flag_hint(e: &clap::Error, args: &[String]) -> Option<String> {
    use clap::error::{ContextKind, ContextValue, ErrorKind};
    let sub = args.iter().skip(1).find(|a| !a.starts_with('-'));
    if e.kind() != ErrorKind::UnknownArgument || sub.map(String::as_str) != Some("find") {
        return None;
    }
    let Some(ContextValue::String(flag)) = e.get(ContextKind::InvalidArg) else {
        return None;
    };
    let flag = flag.split('=').next().unwrap_or(flag);
    let near = match flag {
        "-c" | "--count" => "`acus find -l PAT` lists each file with its number of hits",
        "-v" | "--invert-match" => {
            "`acus find` cannot invert a match; write the regex for the lines you want, or filter other output with `grep -v`"
        }
        "-o" | "--only-matching" => {
            "`acus find` prints whole matching lines; `acus show path:A-B` prints a range"
        }
        "-x" | "--line-regexp" => "anchor the pattern instead: 'PAT' as '^PAT$'",
        "-m" | "--max-count" => "`--max-hits N` caps the hits",
        "-s" | "-S" | "--smart-case" | "--no-messages" => "`-i` ignores case",
        "--include" => "`-g '*.ext'` includes files by glob",
        "--exclude" | "--exclude-dir" => "`-g '!GLOB'` excludes files by glob",
        "-L" | "--files-without-match" => "`acus find -l PAT` lists the files that match",
        "--no-heading" | "--color" | "--no-line-number" | "--heading" | "--line-number" => {
            "output is always grouped by file with line numbers; drop the flag"
        }
        _ => "that flag does not exist",
    };
    Some(format!(
        "{near}; every flag is listed by `acus find --help`"
    ))
}

fn main() -> ExitCode {
    // `acus … | head` should end quietly like other CLI tools, not panic on a closed pipe.
    #[cfg(unix)]
    unsafe {
        libc::signal(libc::SIGPIPE, libc::SIG_DFL);
    }
    let cli = Cli::try_parse().unwrap_or_else(|e| {
        let _ = e.print();
        let args: Vec<String> = std::env::args().collect();
        if let Some(hint) = find_flag_hint(&e, &args) {
            eprintln!("hint: {hint}");
        }
        std::process::exit(e.exit_code());
    });
    match execute(cli, false) {
        Ok(Outcome::Found | Outcome::NoChanges) => ExitCode::SUCCESS,
        Ok(Outcome::Empty) => ExitCode::from(1),
        Ok(Outcome::Exit(c)) => ExitCode::from(c),
        Err(e) => {
            // Messages carry an optional second line `hint: …`.
            eprintln!("error: {e:#}");
            ExitCode::from(2)
        }
    }
}
