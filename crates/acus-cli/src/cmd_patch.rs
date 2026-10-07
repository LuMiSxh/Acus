use crate::Outcome;
use crate::config::Config;
use crate::fmt::{Format, Out};
use acus_edit::{Change, Region};
use anyhow::{Context, Result};
use similar::{Algorithm, DiffOp, capture_diff_slices};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// The syntax `--help` shows after the options; `*** Replace All:` is the one agents miss.
const SYNTAX: &str = "\
PATCH FORMAT (stdin or --file; all or nothing, nothing is written on an error):
  *** Update File: PATH         then `@@ enclosing line` anchors and ` `, `-`, `+` lines, or
                                SEARCH/REPLACE blocks; context, `-` and SEARCH lines are WHOLE lines
  *** Add File: PATH            then `+` lines
  *** Delete File: PATH
  *** Replace Symbol: PATH#Symbol   then the new definition as `+` lines
  *** Delete Symbol: PATH#Symbol
  *** Move Symbol: PATH#Symbol      then *** Before: PATH#Sym, *** After: PATH#Sym or *** To: PATH

PART OF A LINE: *** Replace All: PATH [PATH...]
  Replaces every occurrence of a text in the files, directories (walked, .gitignore respected)
  and globs (*.py, matched anywhere below the current directory) named after the colon,
  separated by spaces. The text need not be a whole line and may span lines. Each block must
  match at least once, or the patch fails. Literal and regex blocks mix, and apply in order:

  *** Replace All: src *.md
  <<<<<<< SEARCH
  old_name(
  =======
  new_name(
  >>>>>>> REPLACE
  <<<<<<< REGEX
  fn (\\w+)_old\\(
  =======
  fn ${1}_new(
  >>>>>>> REPLACE

  SEARCH is literal and whitespace-exact. REGEX is a Rust regex run per file with (?m);
  `.` does not cross lines (use \\n or (?s)) and the replacement may use ${1}, ${name}.

Example (add --fmt to format the written files):
  acus patch <<'PATCH'
  *** Update File: src/lib.rs
  @@ fn parse
  -    let x = 1;
  +    let x = 2;
  PATCH";

#[derive(clap::Args)]
#[command(after_long_help = SYNTAX)]
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
    /// Run each written file through its formatter (rustfmt, ruff/black, prettier, swift-format,
    /// gofmt) if installed; the printed lines are the formatted ones.
    #[arg(long, overrides_with = "no_fmt")]
    fmt: bool,
    /// Skip formatting even if `[patch] fmt = true` is configured.
    #[arg(long)]
    no_fmt: bool,
}

/// Written lines shown per region; longer regions end with a `show` hint.
const REGION_LINES: usize = 20;

pub fn run(a: Args, out: &Out, nested: bool, cfg: &Config) -> Result<Outcome> {
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
    let mut applied = acus_edit::apply(&acus_edit::parse(&text)?, Path::new(""), a.check)?;
    let mut lines: Vec<String> = applied.changes.iter().map(ToString::to_string).collect();
    if (a.fmt || cfg.patch.fmt) && !a.no_fmt && !a.check {
        lines.extend(format_written(&applied.changes, &mut applied.regions));
    }
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

/// Formats every written file and moves `regions` onto the formatted lines.
/// Returns one note per formatted file or failing formatter; failures keep the file as written.
fn format_written(changes: &[Change], regions: &mut [Region]) -> Vec<String> {
    let mut notes = Vec::new();
    for c in changes {
        let path = match c {
            Change::Added { path, .. } | Change::Modified { path, .. } => path,
            Change::Moved { to, .. } => to,
            Change::Deleted { .. } => continue,
        };
        let Some(argv) = formatter(Path::new(path)) else {
            continue;
        };
        let Ok(old) = std::fs::read_to_string(path) else {
            continue;
        };
        let new = match pipe(&argv, &old) {
            Ok(new) => new,
            Err(e) => {
                notes.push(format!("fmt {}: {path} left as written ({e})", argv[0]));
                continue;
            }
        };
        if new == old || new.trim().is_empty() {
            continue;
        }
        if let Err(e) = std::fs::write(path, &new) {
            notes.push(format!("fmt {}: cannot write {path} ({e})", argv[0]));
            continue;
        }
        let (o, n): (Vec<&str>, Vec<&str>) = (old.lines().collect(), new.lines().collect());
        let ops = capture_diff_slices(Algorithm::Myers, &o, &n);
        let mut mine: Vec<(usize, usize)> = Vec::new();
        for r in regions.iter_mut().filter(|r| r.path == *path) {
            let (s, e) = (r.start - 1, r.start - 1 + r.lines.len());
            mine.push((s, e));
            let (ns, ne) = (map(&ops, s, false), map(&ops, e - 1, true));
            r.start = ns + 1;
            r.lines = n[ns..ne.max(ns)].iter().map(|l| l.to_string()).collect();
        }
        let elsewhere: usize = ops
            .iter()
            .filter(|op| !matches!(op, DiffOp::Equal { .. }))
            .filter(|op| {
                let o = op.old_range();
                !mine.iter().any(|&(s, e)| o.start <= e && s <= o.end)
            })
            .map(|op| op.old_range().len().max(op.new_range().len()))
            .sum();
        let extra = if elsewhere > 0 {
            format!(", also {elsewhere} lines outside the patch")
        } else {
            String::new()
        };
        notes.push(format!("fmt {}: {path}{extra}", argv[0]));
    }
    notes
}

/// Where old line `i` ended up: its own new index, or the start/end of what replaced it.
fn map(ops: &[DiffOp], i: usize, end: bool) -> usize {
    for op in ops {
        let (o, n) = (op.old_range(), op.new_range());
        if o.contains(&i) {
            return match op {
                DiffOp::Equal { .. } => n.start + (i - o.start) + usize::from(end),
                _ if end => n.end,
                _ => n.start,
            };
        }
    }
    ops.last().map_or(0, |op| op.new_range().end)
}

/// Formatter command reading the file on stdin and printing it formatted, if one is installed.
fn formatter(path: &Path) -> Option<Vec<String>> {
    let p = path.to_string_lossy().into_owned();
    let v = |a: &[&str]| Some(a.iter().map(|s| s.to_string()).collect());
    match path.extension()?.to_str()? {
        "rs" if on_path("rustfmt") => v(&["rustfmt", "--edition", &rust_edition(path)]),
        "py" if on_path("ruff") => v(&["ruff", "format", "--stdin-filename", &p, "-"]),
        "py" if on_path("black") => v(&["black", "-q", "--stdin-filename", &p, "-"]),
        "go" if on_path("gofmt") => v(&["gofmt"]),
        "swift" if on_path("swift-format") => {
            v(&["swift-format", "format", "--assume-filename", &p])
        }
        "swift" if cfg!(target_os = "macos") && on_path("xcrun") => {
            v(&["xcrun", "swift-format", "format", "--assume-filename", &p])
        }
        "ts" | "tsx" | "js" | "jsx" | "mjs" | "cjs" | "mts" | "cts" | "svelte" | "vue" | "css"
        | "scss" => {
            let bin = path
                .canonicalize()
                .ok()?
                .ancestors()
                .map(|d| d.join("node_modules/.bin/prettier"))
                .find(|b| b.is_file())
                .map(|b| b.to_string_lossy().into_owned())
                .or_else(|| on_path("prettier").then(|| "prettier".into()))?;
            Some(vec![bin, "--stdin-filepath".into(), p])
        }
        _ => None,
    }
}

fn on_path(name: &str) -> bool {
    let exts: &[&str] = if cfg!(windows) {
        &[".exe", ".cmd"]
    } else {
        &[""]
    };
    std::env::var_os("PATH").is_some_and(|ps| {
        std::env::split_paths(&ps)
            .any(|d| exts.iter().any(|e| d.join(format!("{name}{e}")).is_file()))
    })
}

/// `edition` from the nearest Cargo.toml that sets one (following `edition.workspace`).
fn rust_edition(path: &Path) -> String {
    let abs = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    for d in abs.ancestors().skip(1) {
        let Ok(t) = std::fs::read_to_string(d.join("Cargo.toml")) else {
            continue;
        };
        let ed = t.lines().map(str::trim).find_map(|l| {
            let v = l.strip_prefix("edition")?.trim_start().strip_prefix('=')?;
            Some(v.trim().trim_matches('"').to_owned())
        });
        if let Some(ed) = ed.filter(|e| e.chars().all(|c| c.is_ascii_digit())) {
            return ed;
        }
    }
    "2021".into()
}

fn pipe(argv: &[String], input: &str) -> Result<String> {
    let mut child = Command::new(&argv[0])
        .args(&argv[1..])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let mut stdin = child.stdin.take().unwrap();
    let input = input.to_owned();
    let writer = std::thread::spawn(move || stdin.write_all(input.as_bytes()));
    let out = child.wait_with_output()?;
    writer.join().ok();
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        anyhow::bail!(
            "{}",
            err.lines()
                .find(|l| !l.trim().is_empty())
                .unwrap_or("failed")
                .trim()
        );
    }
    Ok(String::from_utf8(out.stdout)?)
}

#[cfg(test)]
mod tests {
    use similar::{Algorithm, capture_diff_slices};

    #[test]
    fn regions_follow_the_formatter() {
        let old = ["a", "b", "  c", "d"];
        let new = ["x", "a", "b", "c", "d"];
        let ops = capture_diff_slices(Algorithm::Myers, &old, &new);
        assert_eq!(
            (super::map(&ops, 1, false), super::map(&ops, 2, true)),
            (2, 4)
        );
    }
}
