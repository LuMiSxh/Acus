use crate::Outcome;
use crate::fmt::{Format, Out};
use anyhow::{Context, Result};
use std::io::Read;
use std::path::{Path, PathBuf};

#[derive(clap::Args)]
pub struct Args {
    /// Read the patch from this file instead of stdin.
    #[arg(short, long)]
    file: Option<PathBuf>,
    /// Validate only; write nothing.
    #[arg(long)]
    check: bool,
    /// Print only the summary, not the written lines.
    #[arg(short, long)]
    quiet: bool,
}

/// Written lines shown per region; longer regions end with a `show` hint.
const REGION_LINES: usize = 20;

pub fn run(a: Args, out: &Out, nested: bool) -> Result<Outcome> {
    if nested && a.file.is_none() {
        anyhow::bail!(
            "patch inside run needs --file\nhint: write the patch to a file or call acus patch directly"
        );
    }
    let text = match &a.file {
        Some(f) => {
            std::fs::read_to_string(f).with_context(|| format!("cannot read {}", f.display()))?
        }
        None => {
            let mut s = String::new();
            std::io::stdin()
                .read_to_string(&mut s)
                .context("cannot read stdin")?;
            s
        }
    };
    let applied = acus_edit::apply(&acus_edit::parse(&text)?, Path::new(""), a.check)?;
    let lines: Vec<String> = applied.changes.iter().map(ToString::to_string).collect();
    if out.format == Format::Json {
        let regions: Vec<_> = applied
            .regions
            .iter()
            .map(|r| {
                serde_json::json!({
                    "path": r.path,
                    "start": r.start,
                    "end": r.start + r.lines.len() - 1,
                    "text": r.lines.join("\n"),
                })
            })
            .collect();
        println!(
            "{}",
            serde_json::json!({ "changes": lines, "regions": regions, "written": !a.check })
        );
        return Ok(Outcome::Found);
    }
    for l in &lines {
        println!("{l}");
    }
    if !a.quiet {
        for r in &applied.regions {
            let end = r.start + r.lines.len() - 1;
            println!(
                "{}",
                out.header(&format!("== {} {}-{end}", r.path, r.start))
            );
            for (i, l) in r.lines.iter().take(REGION_LINES).enumerate() {
                println!("{}", out.numbered(r.start + i, l, false));
            }
            if r.lines.len() > REGION_LINES {
                println!(
                    "… {} more lines (acus show {}:{}-{end})",
                    r.lines.len() - REGION_LINES,
                    r.path,
                    r.start + REGION_LINES
                );
            }
        }
    }
    if a.check {
        println!("(check only, nothing written)");
    }
    Ok(Outcome::Found)
}
