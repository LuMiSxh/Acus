use crate::Outcome;
use crate::fmt::{Format, Out};
use acus_syntax::{Lang, outline};
use acus_walk::display_path;
use anyhow::{Context, Result};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(clap::Args)]
pub struct Args {
    /// Shell command, e.g. 'cargo test -q'; stdout and stderr are merged.
    command: String,
    /// Output lines kept (head and tail halves).
    #[arg(long, default_value_t = 200)]
    max_output: usize,
    /// Code locations shown.
    #[arg(long, default_value_t = 8)]
    max_refs: usize,
    /// Lines per location.
    #[arg(long, default_value_t = 40)]
    max_lines: usize,
    /// Lines around a location outside any known symbol.
    #[arg(long, default_value_t = 3)]
    context: usize,
}

/// Runs a command, prints its output, then the code its `path:line` references point at.
/// Exit: the command's exit code (2 if it could not start).
pub fn run(a: Args, out: &Out) -> Result<Outcome> {
    let cmd = format!("{} 2>&1", a.command);
    let res = if cfg!(windows) {
        Command::new("cmd").args(["/C", &cmd]).output()
    } else {
        Command::new("sh").args(["-c", &cmd]).output()
    }
    .with_context(|| format!("cannot run `{}`", a.command))?;
    let text = String::from_utf8_lossy(&res.stdout);
    let code = res.status.code().unwrap_or(1);
    let cwd = std::env::current_dir()?;
    let refs = refs(&text, &cwd);
    if out.format == Format::Json {
        let locs: Vec<_> = refs
            .iter()
            .take(a.max_refs)
            .filter_map(|(p, l)| snippet(p, *l, &a))
            .map(|s| serde_json::json!({"address": s.0, "start": s.1, "text": s.2.join("\n")}))
            .collect();
        println!(
            "{}",
            serde_json::json!({"exit": code, "output": text, "locations": locs})
        );
        return Ok(Outcome::Exit(code.clamp(0, 255) as u8));
    }
    let lines: Vec<&str> = text.lines().collect();
    let half = a.max_output.max(2) / 2;
    if lines.len() > half * 2 {
        lines[..half].iter().for_each(|l| println!("{l}"));
        println!("… {} output lines omitted", lines.len() - half * 2);
        lines[lines.len() - half..]
            .iter()
            .for_each(|l| println!("{l}"));
    } else {
        lines.iter().for_each(|l| println!("{l}"));
    }
    println!("{}", out.header(&format!("== exit {code}")));
    for (p, l) in refs.iter().take(a.max_refs) {
        let Some((label, start, body)) = snippet(p, *l, &a) else {
            continue;
        };
        println!("{}", out.header(&format!("== {label} (line {l})")));
        for (i, t) in body.iter().enumerate() {
            println!("{}", out.numbered(start + i, t, start + i == *l));
        }
    }
    if refs.len() > a.max_refs {
        println!(
            "… {} more locations (--max-refs {})",
            refs.len() - a.max_refs,
            refs.len()
        );
    }
    Ok(Outcome::Exit(code.clamp(0, 255) as u8))
}

/// `path:line` references to files under `cwd`, first occurrence order, one per enclosing
/// region is decided later. Handles `a.rs:3:5`, `a.ts(3,5)` and Python's `File "a.py", line 3`.
fn refs(text: &str, cwd: &Path) -> Vec<(PathBuf, usize)> {
    let mut seen = HashSet::new();
    let mut v = Vec::new();
    for raw in text.lines() {
        for (p, l) in candidates(raw) {
            let path = Path::new(p.trim_start_matches("./"));
            let full = if path.is_absolute() {
                path.to_path_buf()
            } else {
                cwd.join(path)
            };
            // ponytail: only files inside the working directory; library and toolchain paths are noise.
            let Ok(rel) = full.strip_prefix(cwd) else {
                continue;
            };
            if full.is_file() && seen.insert((rel.to_path_buf(), l)) {
                v.push((rel.to_path_buf(), l));
            }
        }
    }
    v
}

fn candidates(line: &str) -> Vec<(&str, usize)> {
    let mut out = Vec::new();
    if let Some(i) = line.find("File \"") {
        let rest = &line[i + 6..];
        if let Some((p, tail)) = rest.split_once("\", line ") {
            let n: String = tail.chars().take_while(char::is_ascii_digit).collect();
            if let Ok(n) = n.parse() {
                out.push((p, n));
            }
        }
    }
    let b = line.as_bytes();
    let path_char = |c: u8| c.is_ascii_alphanumeric() || b"._-/\\".contains(&c);
    let mut i = 0;
    while i < b.len() {
        if b[i] == b':' || b[i] == b'(' {
            let mut s = i;
            while s > 0 && path_char(b[s - 1]) {
                s -= 1;
            }
            // Windows drive letter: `C:\x\a.rs:3`.
            if s >= 2 && b[s - 1] == b':' && b[s - 2].is_ascii_alphabetic() && b[s] == b'\\' {
                s -= 2;
            }
            let p = &line[s..i];
            let digits: String = line[i + 1..]
                .chars()
                .take_while(char::is_ascii_digit)
                .collect();
            let ext = Path::new(p).extension().is_some();
            if ext && !digits.is_empty() && p.contains('.') {
                out.push((p, digits.parse().unwrap_or(0)));
            }
        }
        i += 1;
    }
    out.retain(|(_, n)| *n > 0);
    out
}

/// Enclosing symbol (capped), else a few lines around `line`.
fn snippet(path: &Path, line: usize, a: &Args) -> Option<(String, usize, Vec<String>)> {
    let src = std::fs::read_to_string(path).ok()?;
    let lines: Vec<&str> = src.lines().collect();
    if line > lines.len() {
        return None;
    }
    let shown = display_path(path);
    let sym = Lang::from_path(path)
        .and_then(|l| outline(l, &src))
        .and_then(|o| o.enclosing(line).cloned());
    let (label, start, end) = match sym {
        Some(s) if s.end_line - s.start_line < a.max_lines => {
            (format!("{shown}#{}", s.qual), s.start_line, s.end_line)
        }
        // Too long to print whole: the lines around the reference, still labelled.
        Some(s) => {
            let half = a.max_lines / 2;
            let st = line.saturating_sub(half).max(s.start_line);
            (
                format!("{shown}#{}", s.qual),
                st,
                (st + a.max_lines - 1).min(s.end_line),
            )
        }
        None => (
            shown,
            line.saturating_sub(a.context).max(1),
            (line + a.context).min(lines.len()),
        ),
    };
    let body = lines[start - 1..end]
        .iter()
        .map(|s| (*s).to_owned())
        .collect();
    Some((label, start, body))
}

#[cfg(test)]
mod tests {
    use super::candidates;

    #[test]
    fn finds_compiler_and_runtime_references() {
        assert_eq!(candidates("  --> src/lib.rs:12:5"), [("src/lib.rs", 12)]);
        assert_eq!(
            candidates("thread 'x' panicked at crates/a/src/b.rs:7:9:"),
            [("crates/a/src/b.rs", 7)]
        );
        assert_eq!(
            candidates("src/app.ts(3,14): error TS2304"),
            [("src/app.ts", 3)]
        );
        assert_eq!(
            candidates("  File \"pkg/mod.py\", line 41, in f"),
            [("pkg/mod.py", 41)]
        );
        assert_eq!(candidates("time: 12:30 ok"), []);
    }
}
