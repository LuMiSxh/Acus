use crate::fmt::{Format, Out};
use crate::{Cli, Outcome, execute};
use anyhow::{Context, Result};
use clap::Parser;
use std::io::{Read, Write};

#[derive(clap::Args)]
pub struct Args {}

/// Reads `[["find","x"],["show","a.rs#f"]]` from stdin and runs each entry in order.
/// Agent/human output separates entries with `>>> acus …` lines; JSON output is one
/// line per entry. Exit: 2 if any entry failed, else 0 if any found something.
pub fn run(_: Args, out: &Out) -> Result<Outcome> {
    let mut s = String::new();
    std::io::stdin()
        .read_to_string(&mut s)
        .context("cannot read stdin")?;
    let batch: Vec<Vec<String>> = serde_json::from_str(&s)
        .context("expected a JSON array of argument arrays\nhint: [[\"find\",\"needle\"],[\"show\",\"src/a.rs#f\"]]")?;
    let (mut found, mut failed) = (false, false);
    for args in batch {
        let mut argv = vec!["acus".to_owned()];
        argv.extend(args.iter().cloned());
        if out.format == Format::Json {
            argv.push("--json".into());
        } else {
            println!("{}", out.header(&format!(">>> acus {}", args.join(" "))));
        }
        let res = Cli::try_parse_from(&argv)
            .map_err(|e| anyhow::anyhow!(e.render().to_string().trim().to_owned()))
            .and_then(|cli| execute(cli, true));
        std::io::stdout().flush().ok();
        match res {
            Ok(Outcome::Found) => found = true,
            Ok(Outcome::Empty) => {
                if out.format != Format::Json {
                    println!("(no results)");
                }
            }
            Err(e) => {
                failed = true;
                let msg = format!("{e:#}");
                if out.format == Format::Json {
                    println!("{}", serde_json::json!({ "error": msg }));
                } else {
                    println!("error: {msg}");
                }
            }
        }
    }
    if failed {
        anyhow::bail!("some commands failed (see above)");
    }
    Ok(if found {
        Outcome::Found
    } else {
        Outcome::Empty
    })
}
