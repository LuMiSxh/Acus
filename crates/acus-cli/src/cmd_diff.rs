use crate::Outcome;
use crate::fmt::{Format, Out};
use acus_syntax::{Lang, Outline, outline};
use anyhow::{Result, bail};
use std::path::Path;
use std::process::Command;

/// Changed lines printed per symbol with `-p`.
const GROUP_LINES: usize = 40;
/// Label for changed lines outside any symbol.
const TOP: &str = "(top level)";

#[derive(clap::Args)]
pub struct Args {
    /// Optional revision (default HEAD, or `A..B`), then paths to limit to.
    args: Vec<String>,
    /// Staged changes only (index against the revision).
    #[arg(long, alias = "cached")]
    staged: bool,
    /// Also print the changed lines under each symbol.
    #[arg(short = 'p', long)]
    lines: bool,
    /// Cap on printed lines.
    #[arg(long, default_value_t = 300)]
    max_lines: usize,
}

#[derive(Default)]
struct FileDiff {
    /// None for added files.
    old: Option<String>,
    /// None for deleted files.
    new: Option<String>,
    binary: bool,
    /// (added, line number on that side, text).
    lines: Vec<(bool, usize, String)>,
}

struct Group {
    label: String,
    range: Option<(usize, usize)>,
    add: usize,
    del: usize,
    lines: Vec<(bool, usize, String)>,
}

/// `git diff` summarised per enclosing symbol, plus untracked files.
pub fn run(a: Args, out: &Out) -> Result<Outcome> {
    // Like git: the first argument is a revision unless it names an existing path.
    let (rev, paths) = match a.args.split_first() {
        Some((r, rest)) if !Path::new(r).exists() => (Some(r.as_str()), rest),
        _ => (None, &a.args[..]),
    };
    let range = rev.and_then(|r| r.split_once(".."));
    // ponytail: `A...B` compares against A, not the merge base; only symbol labels of removed lines are affected.
    let old_rev = range.map_or(
        rev.unwrap_or("HEAD"),
        |(l, _)| if l.is_empty() { "HEAD" } else { l },
    );
    let new_rev = range
        .map(|(_, r)| r.trim_start_matches('.'))
        .map(|r| if r.is_empty() { "HEAD" } else { r });

    let mut cmd = vec![
        "-c",
        "core.quotepath=off",
        "diff",
        "--no-color",
        "--no-ext-diff",
        "--no-textconv",
        "-U0",
        "-M",
        "--relative",
        "--src-prefix=a/",
        "--dst-prefix=b/",
    ];
    if a.staged {
        cmd.push("--cached");
    }
    cmd.extend(rev);
    cmd.push("--");
    cmd.extend(paths.iter().map(String::as_str));
    let mut files = parse(&git(&cmd)?);
    if !a.staged && range.is_none() {
        let mut ls = vec!["ls-files", "--others", "--exclude-standard", "--"];
        ls.extend(paths.iter().map(String::as_str));
        for p in git(&ls)?.lines() {
            let Ok(src) = std::fs::read(p) else { continue };
            let mut f = FileDiff {
                new: Some(p.into()),
                ..FileDiff::default()
            };
            if src.contains(&0) {
                f.binary = true;
            } else {
                let src = String::from_utf8_lossy(&src);
                f.lines = src
                    .lines()
                    .enumerate()
                    .map(|(i, l)| (true, i + 1, l.into()))
                    .collect();
            }
            files.push(f);
        }
    }
    if files.is_empty() {
        return Ok(Outcome::Empty);
    }

    let show = |rev: &str, p: &str| git(&["show", &format!("{rev}:./{p}")]).ok();
    let rendered: Vec<_> = files
        .iter()
        .map(|f| {
            let modified = f.old.is_some() && f.new.is_some();
            let parse = |p: &Option<String>, src: Option<String>| {
                p.as_deref()
                    .and_then(|p| Lang::from_path(Path::new(p)))
                    .zip(src)
                    .and_then(|(l, s)| outline(l, &s))
            };
            let new_src =
                f.new
                    .as_deref()
                    .filter(|_| modified)
                    .and_then(|p| match (a.staged, new_rev) {
                        (_, Some(r)) => show(r, p),
                        (true, None) => show("", p),
                        (false, None) => std::fs::read_to_string(p).ok(),
                    });
            let old_src = f
                .old
                .as_deref()
                .filter(|_| modified && f.lines.iter().any(|l| !l.0))
                .and_then(|p| show(old_rev, p));
            let (new_o, old_o) = (parse(&f.new, new_src), parse(&f.old, old_src));
            (f, groups(f, new_o.as_ref(), old_o.as_ref()))
        })
        .collect();

    let total = |f: &FileDiff, add: bool| f.lines.iter().filter(|l| l.0 == add).count();
    if out.format == Format::Json {
        let v: Vec<_> = rendered
            .iter()
            .map(|(f, gs)| {
                let syms: Vec<_> = gs
                    .iter()
                    .map(|g| {
                        let mut s = serde_json::json!({
                            "symbol": g.label, "range": g.range, "added": g.add, "removed": g.del,
                        });
                        if a.lines {
                            s["lines"] = g.lines.iter().map(|(add, n, t)| serde_json::json!({
                                "op": if *add { "+" } else { "-" }, "line": n, "text": t,
                            })).collect();
                        }
                        s
                    })
                    .collect();
                serde_json::json!({
                    "status": status(f), "path": f.new.as_ref().or(f.old.as_ref()), "old_path": f.old,
                    "binary": f.binary, "added": total(f, true), "removed": total(f, false), "symbols": syms,
                })
            })
            .collect();
        println!("{}", serde_json::Value::Array(v));
        return Ok(Outcome::Found);
    }

    let mut printed = 0;
    for (i, (f, gs)) in rendered.iter().enumerate() {
        let path = match (&f.old, &f.new) {
            (Some(o), Some(n)) if o != n => format!("{o} -> {n}"),
            (o, n) => n.clone().or(o.clone()).unwrap_or_default(),
        };
        let mut block = vec![
            out.header(&format!(
                "{} {path} {}{}",
                status(f),
                counts(total(f, true), total(f, false)),
                if f.binary { "(binary)" } else { "" }
            ))
            .trim_end()
            .to_owned(),
        ];
        for g in gs {
            if !g.label.is_empty() {
                let range = match g.range {
                    Some((s, e)) => format!(" {s}-{e}"),
                    None if g.label == TOP => String::new(),
                    None => " (removed)".into(),
                };
                block.push(format!("  {}{range} {}", g.label, counts(g.add, g.del)));
            }
            if a.lines {
                for (add, n, t) in g.lines.iter().take(GROUP_LINES) {
                    block.push(if *add {
                        out.numbered(*n, &format!("+{t}"), false)
                    } else {
                        format!("\t-{t}")
                    });
                }
                if g.lines.len() > GROUP_LINES {
                    block.push(out.dim(&format!(
                        "… {} more changed lines",
                        g.lines.len() - GROUP_LINES
                    )));
                }
            }
        }
        if printed > 0 && printed + block.len() > a.max_lines {
            println!(
                "… {} more files (limit with -- PATH, or drop -p)",
                rendered.len() - i
            );
            break;
        }
        printed += block.len();
        block.iter().for_each(|l| println!("{l}"));
    }
    let (add, del) = files.iter().fold((0, 0), |(x, y), f| {
        (x + total(f, true), y + total(f, false))
    });
    println!(
        "{}",
        out.header(&format!(
            "== {} file{} {}",
            files.len(),
            if files.len() == 1 { "" } else { "s" },
            counts(add, del)
        ))
    );
    Ok(Outcome::Found)
}

fn git(args: &[&str]) -> Result<String> {
    let o = Command::new("git").args(args).output()?;
    if !o.status.success() {
        bail!(
            "git {}: {}",
            args.iter()
                .find(|a| !a.starts_with('-') && !a.contains('='))
                .unwrap_or(&""),
            String::from_utf8_lossy(&o.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&o.stdout).into_owned())
}

fn status(f: &FileDiff) -> &'static str {
    match (&f.old, &f.new) {
        (None, _) => "A",
        (_, None) => "D",
        (Some(o), Some(n)) if o != n => "R",
        _ => "M",
    }
}

fn counts(add: usize, del: usize) -> String {
    match (add, del) {
        (0, 0) => String::new(),
        (a, 0) => format!("+{a}"),
        (0, d) => format!("-{d}"),
        (a, d) => format!("+{a} -{d}"),
    }
}

/// Changed lines bucketed by innermost enclosing symbol, in diff order. Whole-file adds and
/// deletes form one unlabelled group.
fn groups(f: &FileDiff, new: Option<&Outline>, old: Option<&Outline>) -> Vec<Group> {
    let mut v: Vec<Group> = Vec::new();
    let whole = f.old.is_none() || f.new.is_none();
    for (add, n, t) in &f.lines {
        let o = if *add { new } else { old };
        let sym = o.filter(|_| !whole).and_then(|o| o.enclosing(*n));
        let label = match sym {
            Some(s) => format!("{} {}", s.kind, s.qual),
            None if whole => String::new(),
            None => TOP.into(),
        };
        let i = match v.iter().position(|g| g.label == label) {
            Some(i) => i,
            None => {
                // Range in the new file, so `acus show path:A-B` works; None if the symbol is gone.
                let range = sym.and_then(|s| {
                    new?.symbols
                        .iter()
                        .find(|x| x.qual == s.qual && x.kind == s.kind)
                        .map(|x| (x.start_line, x.end_line))
                });
                v.push(Group {
                    label,
                    range,
                    add: 0,
                    del: 0,
                    lines: Vec::new(),
                });
                v.len() - 1
            }
        };
        let g = &mut v[i];
        if *add {
            g.add += 1
        } else {
            g.del += 1
        }
        g.lines.push((*add, *n, t.clone()));
    }
    v
}

/// Parses `git diff -U0` with `a/` and `b/` prefixes.
fn parse(text: &str) -> Vec<FileDiff> {
    let mut files: Vec<FileDiff> = Vec::new();
    let (mut o, mut n, mut in_hunk) = (0, 0, false);
    for l in text.lines() {
        if let Some(h) = l.strip_prefix("diff --git a/") {
            let (a, b) = h.split_once(" b/").unwrap_or((h, h));
            files.push(FileDiff {
                old: Some(a.into()),
                new: Some(b.into()),
                ..FileDiff::default()
            });
            in_hunk = false;
            continue;
        }
        let Some(f) = files.last_mut() else { continue };
        if in_hunk {
            if let Some(t) = l.strip_prefix('-') {
                f.lines.push((false, o, t.into()));
                o += 1;
            } else if let Some(t) = l.strip_prefix('+') {
                f.lines.push((true, n, t.into()));
                n += 1;
            }
        }
        if let Some(h) = l.strip_prefix("@@ -") {
            in_hunk = true;
            let num = |s: &str| {
                s.split(',')
                    .next()
                    .and_then(|x| x.parse().ok())
                    .unwrap_or(0)
            };
            let mut it = h.split(' ');
            o = num(it.next().unwrap_or(""));
            n = num(it.next().unwrap_or("").trim_start_matches('+'));
        } else if in_hunk {
        } else if l.starts_with("new file mode") {
            f.old = None;
        } else if l.starts_with("deleted file mode") {
            f.new = None;
        } else if let Some(p) = l.strip_prefix("rename from ") {
            f.old = Some(p.into());
        } else if let Some(p) = l.strip_prefix("rename to ") {
            f.new = Some(p.into());
        } else if let Some(p) = l.strip_prefix("--- a/") {
            f.old = Some(p.trim_end_matches('\t').into());
        } else if let Some(p) = l.strip_prefix("+++ b/") {
            f.new = Some(p.trim_end_matches('\t').into());
        } else if l.starts_with("Binary files ") {
            f.binary = true;
        }
    }
    files
}

#[cfg(test)]
mod tests {
    use super::parse;

    #[test]
    fn parses_hunks_renames_and_headers_inside_hunks() {
        let d = "diff --git a/x.rs b/x.rs\nindex 1..2 100644\n--- a/x.rs\n+++ b/x.rs\n@@ -3,2 +3 @@ fn f\n--- dashes\n-b\n+c\n@@ -9,0 +10,2 @@\n+d\n++e\n\
                 diff --git a/o.rs b/n.rs\nsimilarity index 100%\nrename from o.rs\nrename to n.rs\n\
                 diff --git a/new.md b/new.md\nnew file mode 100644\n--- /dev/null\n+++ b/new.md\n@@ -0,0 +1 @@\n+hi\n";
        let f = parse(d);
        assert_eq!(f.len(), 3);
        let l: Vec<_> = f[0]
            .lines
            .iter()
            .map(|(a, n, t)| (*a, *n, t.as_str()))
            .collect();
        assert_eq!(
            l,
            [
                (false, 3, "-- dashes"),
                (false, 4, "b"),
                (true, 3, "c"),
                (true, 10, "d"),
                (true, 11, "+e")
            ]
        );
        assert_eq!(
            (f[1].old.as_deref(), f[1].new.as_deref()),
            (Some("o.rs"), Some("n.rs"))
        );
        assert_eq!((f[2].old.as_deref(), f[2].lines.len()), (None, 1));
    }
}
