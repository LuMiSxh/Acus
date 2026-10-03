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
}

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
    let changes = acus_edit::apply(&acus_edit::parse(&text)?, Path::new(""), a.check)?;
    let lines: Vec<String> = changes.iter().map(ToString::to_string).collect();
    if out.format == Format::Json {
        println!(
            "{}",
            serde_json::json!({ "changes": lines, "written": !a.check })
        );
    } else {
        for l in &lines {
            println!("{l}");
        }
        if a.check {
            println!("(check only, nothing written)");
        }
    }
    Ok(Outcome::Found)
}
