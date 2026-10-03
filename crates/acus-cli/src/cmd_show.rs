use crate::Outcome;
use crate::fmt::{Format, Out};
use acus_syntax::{Address, Lang, Resolve, Target, outline, resolve};
use anyhow::{Context, Result, bail};

#[derive(clap::Args)]
pub struct Args {
    /// path, path:START-END, path:LINE or path#Symbol (a unique suffix of the qualified name is enough).
    #[arg(required = true)]
    addrs: Vec<String>,
    /// Cap per address.
    #[arg(long, default_value_t = 400)]
    max_lines: usize,
    /// Extra lines around symbol and range targets.
    #[arg(long, default_value_t = 0)]
    context: usize,
}

pub fn run(a: Args, out: &Out) -> Result<Outcome> {
    let mut json = Vec::new();
    for raw in &a.addrs {
        let addr = Address::parse(raw);
        let path = &addr.path;
        let src =
            std::fs::read_to_string(path).with_context(|| format!("path not found: {path}"))?;
        let lines: Vec<&str> = src.lines().collect();
        let (label, start, end) = match &addr.target {
            Target::Whole => (path.clone(), 1, lines.len()),
            Target::Lines(s, e) => (path.clone(), *s, *e),
            Target::Symbol(q) => {
                let Some(o) = Lang::from_path(path.as_ref()).and_then(|l| outline(l, &src)) else {
                    bail!("no syntax support for {path}\nhint: use {path}:START-END");
                };
                match resolve(&o, q) {
                    Resolve::Found(s) => (format!("{path}#{}", s.qual), s.start_line, s.end_line),
                    Resolve::Missing => {
                        bail!("no symbol `{q}` in {path}\nhint: acus outline {path}")
                    }
                    Resolve::Ambiguous(v) => {
                        let c: Vec<_> = v
                            .iter()
                            .map(|s| format!("{path}#{} ({}-{})", s.qual, s.start_line, s.end_line))
                            .collect();
                        bail!("`{q}` is ambiguous in {path}\nhint: {}", c.join(", "))
                    }
                }
            }
        };
        if start == 0 || start > lines.len().max(1) || start > end {
            bail!("{path} has {} lines", lines.len());
        }
        let widen = if addr.target == Target::Whole {
            0
        } else {
            a.context
        };
        let start = start.saturating_sub(widen).max(1);
        let end = (end + widen).min(lines.len());
        let shown_end = end.min(start + a.max_lines.max(1) - 1);
        if out.format == Format::Json {
            json.push(serde_json::json!({
                "address": label,
                "start": start,
                "end": shown_end,
                "text": lines.get(start - 1..shown_end).unwrap_or_default().join("\n"),
            }));
            continue;
        }
        println!("{}", out.header(&format!("== {label} {start}-{end}")));
        for (i, line) in lines.iter().enumerate().take(shown_end).skip(start - 1) {
            println!("{}", out.numbered(i + 1, line, false));
        }
        if shown_end < end {
            println!(
                "… {} more lines (acus show {path}:{}-{end})",
                end - shown_end,
                shown_end + 1
            );
        }
    }
    if out.format == Format::Json {
        println!("{}", serde_json::Value::Array(json));
    }
    Ok(Outcome::Found)
}
