use crate::Outcome;
use crate::fmt::{Format, Out, clip, hit_text};
use acus_search::{FileHits, FindOpts, find};
use acus_syntax::Symbol;
use acus_walk::WalkOpts;
use anyhow::{Result, bail};
use std::collections::BTreeSet;

#[derive(clap::Args)]
pub struct Args {
    /// Pattern, then paths (like rg). With -e, all positionals are paths.
    positional: Vec<String>,
    /// Pattern (repeatable); may start with `-`.
    #[arg(short = 'e', long = "regexp", allow_hyphen_values = true)]
    patterns: Vec<String>,
    /// Include glob; prefix with ! to exclude (repeatable).
    #[arg(short = 'g', long = "glob")]
    globs: Vec<String>,
    #[arg(short = 'i', long)]
    ignore_case: bool,
    #[arg(short = 'F', long)]
    fixed_strings: bool,
    #[arg(long)]
    hidden: bool,
    /// Also search files excluded by .gitignore.
    #[arg(short = 'u', long)]
    no_ignore: bool,
    /// Whole words only.
    #[arg(short = 'w', long = "word-regexp")]
    word: bool,
    /// Only list the files with hits.
    #[arg(short = 'l', long = "files-with-matches")]
    files_only: bool,
    /// File type like rg: rs, rust, py, ts, js, swift, md, … (repeatable).
    #[arg(short = 't', long = "type")]
    types: Vec<String>,
    /// Accepted for rg/grep habits; lines are always numbered, the search is always recursive.
    #[arg(short = 'n', hide = true)]
    _line_numbers: bool,
    #[arg(short = 'r', short_alias = 'R', hide = true)]
    _recursive: bool,
    #[arg(short = 'H', hide = true)]
    _with_filename: bool,
    /// Print enclosing symbol bodies instead of single lines.
    #[arg(long)]
    block: bool,
    /// grep-style context (-A/-B/-C N): prints the enclosing symbol like --block.
    // ponytail: N is ignored; the symbol body is the context agents want from grep -A.
    #[arg(short = 'C', short_aliases = ['A', 'B'], value_name = "N", hide = true)]
    context: Option<usize>,
    #[arg(long, default_value_t = 50)]
    max_hits: usize,
    /// Per printed block.
    #[arg(long, default_value_t = 80)]
    max_lines: usize,
    /// Skip tree-sitter (faster, no symbols).
    #[arg(long)]
    no_syntax: bool,
}

pub fn run(a: Args, out: &Out) -> Result<Outcome> {
    let mut positional = a.positional.into_iter();
    let patterns = if a.patterns.is_empty() {
        positional.next().into_iter().collect()
    } else {
        a.patterns
    };
    if patterns.is_empty() {
        bail!("no pattern given\nhint: acus find PATTERN [PATH...] or -e PAT -e PAT");
    }
    let (exclude, mut include): (Vec<String>, Vec<String>) =
        a.globs.into_iter().partition(|g| g.starts_with('!'));
    for t in &a.types {
        let exts: &[&str] = match t.as_str() {
            "rust" => &["rs"],
            "py" | "python" => &["py", "pyi"],
            "ts" | "typescript" => &["ts", "tsx", "mts", "cts"],
            "js" | "javascript" => &["js", "jsx", "mjs", "cjs"],
            "md" | "markdown" => &["md", "markdown"],
            "yaml" | "yml" => &["yaml", "yml"],
            "cpp" | "c++" => &["cpp", "cc", "cxx", "hpp", "hh", "h"],
            other => {
                include.push(format!("*.{other}"));
                continue;
            }
        };
        include.extend(exts.iter().map(|e| format!("*.{e}")));
    }
    let fixed = a.fixed_strings;
    let opts = FindOpts {
        patterns,
        walk: WalkOpts {
            roots: positional.map(Into::into).collect(),
            include,
            exclude: exclude.into_iter().map(|g| g[1..].to_owned()).collect(),
            hidden: a.hidden,
            no_ignore: a.no_ignore,
        },
        ignore_case: a.ignore_case,
        fixed,
        syntax: !a.no_syntax && !a.files_only,
        word: a.word,
    };
    let files = find(&opts).map_err(|e| {
        let msg = format!("{e:#}");
        if fixed || !msg.starts_with("invalid pattern") {
            e
        } else if msg.contains("look-around") {
            anyhow::anyhow!("{msg}\nhint: match the surrounding text directly, or narrow with -w / a second -e")
        } else {
            anyhow::anyhow!("{msg}\nhint: escape regex characters like ( [ {{ . with \\, or use -F for literal text")
        }
    })?;
    if files.is_empty() {
        if out.format != Format::Json && !(a.hidden && a.no_ignore) {
            eprintln!("(no matches; hidden and .gitignored files were skipped: --hidden, -u)");
        }
        return Ok(Outcome::Empty);
    }
    if a.files_only && out.format != Format::Json {
        files
            .iter()
            .for_each(|f| println!("{} {}", f.path, f.hits.len()));
        return Ok(Outcome::Found);
    }
    // Spend the hit budget in path order.
    let mut budget = a.max_hits;
    let mut shown = Vec::new();
    let (mut rest, mut rest_files) = (0, 0);
    for f in &files {
        let n = f.hits.len().min(budget);
        budget -= n;
        if n > 0 {
            shown.push((f, n));
        }
        if n < f.hits.len() {
            rest += f.hits.len() - n;
            rest_files += 1;
        }
    }
    match out.format {
        Format::Json => print_json(&shown, rest),
        _ if a.block || a.context.is_some() => print_blocks(&shown, out, a.max_lines),
        _ => print_lines(&shown, out),
    }
    if rest > 0 && out.format != Format::Json {
        println!(
            "… {rest} more hits in {rest_files} files (narrow with -g/-e or raise --max-hits)"
        );
    }
    Ok(Outcome::Found)
}

fn sym(f: &FileHits, line: usize) -> Option<&Symbol> {
    f.outline.as_ref()?.enclosing(line)
}

fn print_lines(shown: &[(&FileHits, usize)], out: &Out) {
    for (f, n) in shown {
        println!("{}", out.header(&f.path));
        let mut last = None;
        for h in &f.hits[..*n] {
            let s = sym(f, h.line);
            let q = s.map(|s| s.qual.as_str());
            if q != last
                && let Some(s) = s
            {
                println!(
                    "{}",
                    out.dim(&format!("@{} {}-{}", s.qual, s.start_line, s.end_line))
                );
            }
            last = q;
            println!("{}\t{}", h.line, hit_text(&h.text, h.col));
        }
    }
}

fn print_blocks(shown: &[(&FileHits, usize)], out: &Out, max_lines: usize) {
    for (f, n) in shown {
        let lines: Vec<&str> = f.source.lines().collect();
        let hit_lines: BTreeSet<usize> = f.hits[..*n].iter().map(|h| h.line).collect();
        let col = |i: usize| f.hits.iter().find(|h| h.line == i).map_or(0, |h| h.col);
        let mut done = BTreeSet::new();
        for &l in &hit_lines {
            let (title, a, b) = match sym(f, l) {
                Some(s) => (
                    format!("== {}#{} {}-{}", f.path, s.qual, s.start_line, s.end_line),
                    s.start_line,
                    s.end_line,
                ),
                // No symbol: the hit with two lines of context.
                None => (
                    format!("== {}:{l}", f.path),
                    l.saturating_sub(2).max(1),
                    (l + 2).min(lines.len()),
                ),
            };
            if !done.insert((a, b)) {
                continue;
            }
            println!("{}", out.header(&title));
            let end = b.min(a + max_lines.max(1) - 1);
            for i in a..=end {
                let line = lines.get(i - 1).copied().unwrap_or("");
                let line = clip(line, col(i));
                println!("{}", out.numbered(i, &line, hit_lines.contains(&i)));
            }
            if end < b {
                println!(
                    "… {} more lines (acus show {}:{}-{b})",
                    b - end,
                    f.path,
                    end + 1
                );
            }
        }
    }
}

fn print_json(shown: &[(&FileHits, usize)], rest: usize) {
    let files: Vec<_> = shown
        .iter()
        .map(|(f, n)| {
            let hits: Vec<_> = f.hits[..*n]
                .iter()
                .map(|h| {
                    serde_json::json!({
                        "line": h.line,
                        "text": h.text,
                        "symbol": sym(f, h.line).map(|s| &s.qual),
                    })
                })
                .collect();
            serde_json::json!({ "path": f.path, "lang": f.lang, "hits": hits })
        })
        .collect();
    println!(
        "{}",
        serde_json::json!({ "files": files, "truncated": rest })
    );
}
