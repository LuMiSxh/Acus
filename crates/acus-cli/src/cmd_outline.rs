use crate::Outcome;
use crate::fmt::{Format, Out};
use acus_syntax::{Lang, Outline, outline};
use acus_walk::{WalkOpts, display_path, walk};
use anyhow::Result;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

#[derive(clap::Args)]
pub struct Args {
    /// Files or directories (directories are walked).
    #[arg(default_value = ".")]
    paths: Vec<PathBuf>,
    /// Only symbols nested less than N deep (1 = top level).
    #[arg(long)]
    depth: Option<usize>,
    /// Include glob for directories; prefix with ! to exclude (repeatable).
    #[arg(short = 'g', long = "glob")]
    globs: Vec<String>,
    #[arg(long)]
    hidden: bool,
    /// Cap on printed lines.
    #[arg(long, default_value_t = 400)]
    max_lines: usize,
}

pub fn run(a: Args, out: &Out) -> Result<Outcome> {
    let (exclude, include): (Vec<String>, Vec<String>) =
        a.globs.into_iter().partition(|g| g.starts_with('!'));
    let found = Mutex::new(Vec::new());
    let parse = |p: &Path| {
        let o = Lang::from_path(p)
            .zip(std::fs::read_to_string(p).ok())
            .and_then(|(l, src)| outline(l, &src));
        (display_path(p), o)
    };
    for p in &a.paths {
        if p.is_file() {
            // Named explicitly: report it even without syntax support.
            found.lock().unwrap().push(parse(p));
            continue;
        }
        let opts = WalkOpts {
            roots: vec![p.clone()],
            include: include.clone(),
            exclude: exclude.iter().map(|g| g[1..].to_owned()).collect(),
            hidden: a.hidden,
            no_ignore: false,
        };
        walk(&opts, |f| {
            if Lang::from_path(f).is_some() {
                // Parse before locking, or the threads take turns.
                let r = parse(f);
                found.lock().unwrap().push(r);
            }
        })?;
    }
    let mut files = found.into_inner().unwrap();
    if files.is_empty() {
        return Ok(Outcome::Empty);
    }
    files.sort_by(|a, b| a.0.cmp(&b.0));
    let depth = a.depth.unwrap_or(usize::MAX);
    if out.format == Format::Json {
        let v: Vec<_> = files
            .iter()
            .map(|(path, o)| {
                let syms: Vec<_> = o
                    .iter()
                    .flat_map(|o| &o.symbols)
                    .filter(|s| s.depth < depth)
                    .collect();
                serde_json::json!({
                    "path": path,
                    "lang": o.as_ref().map(|o| o.lang),
                    "lines": o.as_ref().map(|o| o.lines),
                    "symbols": syms,
                })
            })
            .collect();
        println!("{}", serde_json::Value::Array(v));
        return Ok(Outcome::Found);
    }
    let mut printed = 0;
    for (i, (path, o)) in files.iter().enumerate() {
        let block = render(path, o.as_ref(), depth, out);
        if printed > 0 && printed + block.len() > a.max_lines {
            println!(
                "… {} more files (narrow with -g, a subdirectory, or --depth 1)",
                files.len() - i
            );
            break;
        }
        printed += block.len();
        for l in block {
            println!("{l}");
        }
    }
    Ok(Outcome::Found)
}

fn render(path: &str, o: Option<&Outline>, depth: usize, out: &Out) -> Vec<String> {
    let Some(o) = o else {
        return vec![format!("{path} (no outline)")];
    };
    let mut v = vec![out.header(&format!("{path} {} {}", o.lang.name(), o.lines))];
    v.extend(o.symbols.iter().filter(|s| s.depth < depth).map(|s| {
        format!(
            "{}{} {} {}-{}",
            "  ".repeat(s.depth),
            s.kind,
            s.detail.as_deref().unwrap_or(&s.name),
            s.start_line,
            s.end_line
        )
    }));
    v
}
