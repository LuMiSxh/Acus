//! Parallel file discovery: honours .gitignore/.ignore, skips hidden entries unless
//! asked, filters by include/exclude globs relative to the walked root.

use anyhow::{Context, Result, bail};
use globset::{Glob, GlobSet, GlobSetBuilder};
use ignore::{WalkBuilder, WalkState};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Default)]
pub struct WalkOpts {
    /// Files or directories to visit; empty means `.`.
    pub roots: Vec<PathBuf>,
    /// Keep only paths matching one of these globs (empty keeps all).
    pub include: Vec<String>,
    /// Drop paths matching one of these globs.
    pub exclude: Vec<String>,
    /// Visit hidden files and directories too.
    pub hidden: bool,
}

fn glob_set(globs: &[String]) -> Result<Option<GlobSet>> {
    if globs.is_empty() {
        return Ok(None);
    }
    let mut b = GlobSetBuilder::new();
    for g in globs {
        b.add(Glob::new(g).with_context(|| format!("invalid glob `{g}`"))?);
    }
    Ok(Some(b.build()?))
}

/// Calls `visit` once per matching file, concurrently from several threads.
pub fn walk<F: Fn(&Path) + Sync>(opts: &WalkOpts, visit: F) -> Result<()> {
    let include = glob_set(&opts.include)?;
    let exclude = glob_set(&opts.exclude)?;
    let roots = if opts.roots.is_empty() {
        vec![PathBuf::from(".")]
    } else {
        opts.roots.clone()
    };
    for r in &roots {
        if !r.exists() {
            bail!("path not found: {}", display_path(r));
        }
    }
    let keep = {
        let roots = roots.clone();
        move |p: &Path| include.as_ref().is_none_or(|s| s.is_match(rel(&roots, p)))
    };
    let mut b = WalkBuilder::new(&roots[0]);
    for r in &roots[1..] {
        b.add(r);
    }
    b.hidden(!opts.hidden).require_git(false);
    if let Some(ex) = exclude {
        // Like rg: `!dist` drops any file or directory named `dist`, `!src/gen` that path;
        // excluded directories are not descended into.
        let roots = roots.clone();
        b.filter_entry(move |e| {
            let r = rel(&roots, e.path());
            !(ex.is_match(r) || r.file_name().is_some_and(|n| ex.is_match(n)))
        });
    }
    b.build_parallel().run(|| {
        let (keep, visit) = (&keep, &visit);
        Box::new(move |entry| {
            // ponytail: unreadable entries are skipped silently; count and report them if it ever confuses an agent.
            if let Ok(e) = entry
                && e.file_type().is_some_and(|t| t.is_file())
                && keep(e.path())
            {
                visit(e.path());
            }
            WalkState::Continue
        })
    });
    Ok(())
}

/// Globs see the path relative to its root; a file given as root is matched as-is.
fn rel<'a>(roots: &[PathBuf], p: &'a Path) -> &'a Path {
    roots
        .iter()
        .find_map(|r| p.strip_prefix(r).ok())
        .filter(|r| !r.as_os_str().is_empty())
        .unwrap_or(p)
}

/// How every acus output prints a path: `/`-separated, without a leading `./`.
pub fn display_path(p: &Path) -> String {
    let s = p.to_string_lossy().replace('\\', "/");
    s.strip_prefix("./").map(str::to_owned).unwrap_or(s)
}
