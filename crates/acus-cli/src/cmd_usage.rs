use crate::Outcome;
use crate::fmt::{Format, Out};
use acus_usage::{UsageOpts, WEIGHTS, report};
use anyhow::Result;
use std::path::PathBuf;

#[derive(clap::Args)]
pub struct Args {
    /// Only transcripts modified in the last N days.
    #[arg(long)]
    days: Option<u64>,
    /// Only sessions whose working directory contains this text.
    #[arg(long)]
    project: Option<String>,
    /// Rows per table.
    #[arg(long, default_value_t = 10)]
    top: usize,
    /// Claude Code projects dir (default ~/.claude/projects).
    #[arg(long)]
    claude_dir: Option<PathBuf>,
    /// Codex sessions dir (default ~/.codex/sessions).
    #[arg(long)]
    codex_dir: Option<PathBuf>,
}

/// 1234567 → "1.2M".
fn k(n: f64) -> String {
    match n {
        n if n >= 1e9 => format!("{:.1}G", n / 1e9),
        n if n >= 1e6 => format!("{:.1}M", n / 1e6),
        n if n >= 1e3 => format!("{:.0}k", n / 1e3),
        n => format!("{n:.0}"),
    }
}

pub fn run(a: Args, out: &Out) -> Result<Outcome> {
    let r = report(&UsageOpts {
        claude_root: a.claude_dir,
        codex_root: a.codex_dir,
        days: a.days,
        project: a.project,
    })?;
    if r.sessions.is_empty() {
        return Ok(Outcome::Empty);
    }
    if out.format == Format::Json {
        println!("{}", serde_json::to_string(&r)?);
        return Ok(Outcome::Found);
    }
    let w = r.weighted.max(1.0);
    let pct = |x: f64| format!("{:.1}%", 100.0 * x / w);
    println!(
        "usage: {} sessions, {} requests, {} prompts ({:.1} req/prompt), {} weighted input-token equivalents",
        r.sessions.len(),
        r.total.requests,
        r.prompts,
        r.total.requests as f64 / r.prompts.max(1) as f64,
        k(r.weighted)
    );
    let mut share: Vec<_> = r
        .total
        .classes()
        .into_iter()
        .zip(WEIGHTS)
        .map(|(n, (name, wt))| (name, n as f64 * wt))
        .collect();
    share.sort_by(|a, b| b.1.total_cmp(&a.1));
    let share: Vec<_> = share
        .iter()
        .map(|(n, x)| format!("{n} {}", pct(*x)))
        .collect();
    println!("cost share: {}", share.join(", "));
    println!(
        "context per request: p50 {} p90 {} p99 {}",
        k(r.context[0] as f64),
        k(r.context[1] as f64),
        k(r.context[2] as f64)
    );
    println!("{}", out.header("models"));
    for (m, t) in r.models.iter().take(a.top) {
        println!(
            "  {m} reqs={} w={} ({})",
            t.requests,
            k(t.weighted()),
            pct(t.weighted())
        );
    }
    println!("{}", out.header("sessions"));
    for s in r.sessions.iter().take(a.top) {
        println!(
            "  {} {} {} reqs={} w={} ({}) maxctx={} compact={} {}",
            s.agent,
            if s.project.is_empty() {
                "?"
            } else {
                &s.project
            },
            &s.id[..s.id.len().min(8)],
            s.tokens.requests,
            k(s.weighted),
            pct(s.weighted),
            k(s.max_context as f64),
            s.compactions,
            s.model
        );
    }
    println!("{}", out.header("tools (calls, input chars, result chars)"));
    for t in r.tools.iter().take(a.top) {
        println!(
            "  {} {} in={} out={}",
            t.name,
            t.calls,
            k(t.input_chars as f64),
            k(t.result_chars as f64)
        );
    }
    let c = &r.chains;
    println!(
        "search/read chains (>=3 in a row): {} chains, {:.0}% of {} tool calls, longest {}",
        c.count,
        100.0 * c.calls_in_chains as f64 / c.tool_calls.max(1) as f64,
        c.tool_calls,
        c.longest
    );
    let hidden = r.sessions.len().saturating_sub(a.top);
    if hidden > 0 {
        println!("… {hidden} more sessions (raise --top or filter with --project/--days)");
    }
    Ok(Outcome::Found)
}
