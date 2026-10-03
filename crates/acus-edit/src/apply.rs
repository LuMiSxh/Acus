use crate::{Chunk, Op};
use acus_syntax::{Lang, Resolve, outline, resolve};
use anyhow::{Context, Result, anyhow, bail};
use std::collections::BTreeMap;
use std::fmt;
use std::fs::{self, Permissions};
use std::io::Write;
use std::path::Path;

/// One line of the summary printed after a patch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Change {
    Added {
        path: String,
        lines: usize,
    },
    Deleted {
        path: String,
        lines: usize,
    },
    Modified {
        path: String,
        added: usize,
        removed: usize,
    },
    Moved {
        from: String,
        to: String,
        added: usize,
        removed: usize,
    },
}

impl fmt::Display for Change {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Change::Added { path, lines } => write!(f, "A {path} +{lines}"),
            Change::Deleted { path, lines } => write!(f, "D {path} -{lines}"),
            Change::Modified {
                path,
                added,
                removed,
            } => write!(f, "M {path} +{added} -{removed}"),
            Change::Moved {
                from,
                to,
                added,
                removed,
            } => write!(f, "R {from} -> {to} +{added} -{removed}"),
        }
    }
}

/// Lines a patch wrote, as they read afterwards (`start` is 1-based).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Region {
    pub path: String,
    pub start: usize,
    pub lines: Vec<String>,
}

/// What `apply` did: one summary per op plus the edited regions of updated files.
#[derive(Debug, Default)]
pub struct Applied {
    pub changes: Vec<Change>,
    pub regions: Vec<Region>,
}

/// A file as the patch sees it; `lines: None` means deleted or not existing.
struct File {
    lines: Option<Vec<String>>,
    crlf: bool,
    trailing_newline: bool,
    perms: Option<Permissions>,
    existed: bool,
    dirty: bool,
    /// Edited (0-based start, length) ranges in the current `lines`.
    edits: Vec<(usize, usize)>,
}

impl File {
    /// Splices `new` over `at..at + old` and keeps earlier edit ranges pointing at their lines.
    fn splice(&mut self, at: usize, old: usize, new: &[String]) {
        let lines = self.lines.as_mut().expect("splice on a missing file");
        lines.splice(at..at + old, new.iter().cloned());
        self.edits.retain(|&(s, _)| s < at || s >= at + old);
        for (s, _) in &mut self.edits {
            if *s >= at + old {
                *s = *s + new.len() - old;
            }
        }
        self.edits.push((at, new.len()));
    }
}

struct Files<'a> {
    root: &'a Path,
    map: BTreeMap<String, File>,
}

impl Files<'_> {
    fn get(&mut self, path: &str) -> Result<&mut File> {
        if !self.map.contains_key(path) {
            let full = self.root.join(path);
            let f = match fs::read(&full) {
                Ok(bytes) => {
                    let text =
                        String::from_utf8(bytes).map_err(|_| anyhow!("{path}: not valid UTF-8"))?;
                    let crlf = text.contains("\r\n");
                    let text = if crlf {
                        text.replace("\r\n", "\n")
                    } else {
                        text
                    };
                    let trailing_newline = text.ends_with('\n');
                    let body = text.strip_suffix('\n').unwrap_or(&text);
                    File {
                        lines: Some(if text.is_empty() {
                            vec![]
                        } else {
                            body.split('\n').map(Into::into).collect()
                        }),
                        crlf,
                        trailing_newline,
                        perms: fs::metadata(&full).ok().map(|m| m.permissions()),
                        existed: true,
                        dirty: false,
                        edits: Vec::new(),
                    }
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => File {
                    lines: None,
                    crlf: false,
                    trailing_newline: true,
                    perms: None,
                    existed: false,
                    dirty: false,
                    edits: Vec::new(),
                },
                Err(e) => return Err(e).with_context(|| format!("{path}: cannot read")),
            };
            self.map.insert(path.to_owned(), f);
        }
        Ok(self.map.get_mut(path).unwrap())
    }

    fn lines(&mut self, path: &str) -> Result<&mut Vec<String>> {
        self.get(path)?
            .lines
            .as_mut()
            .ok_or_else(|| anyhow!("{path}: file not found"))
    }
}

/// Validates and applies every op in memory, then writes all files atomically.
/// With `check`, stops before writing. Relative paths resolve against `root`.
pub fn apply(ops: &[Op], root: &Path, check: bool) -> Result<Applied> {
    let mut files = Files {
        root,
        map: BTreeMap::new(),
    };
    let mut changes = Vec::new();
    for op in ops {
        changes.push(apply_op(&mut files, op)?);
    }
    let mut regions = Vec::new();
    for (path, f) in &files.map {
        let (Some(lines), true) = (&f.lines, f.existed) else {
            continue;
        };
        let mut edits = f.edits.clone();
        edits.sort_unstable();
        for (s, n) in edits.into_iter().filter(|&(_, n)| n > 0) {
            regions.push(Region {
                path: path.clone(),
                start: s + 1,
                lines: lines[s..s + n].to_vec(),
            });
        }
    }
    if !check {
        commit(root, files.map)?;
    }
    Ok(Applied { changes, regions })
}

fn apply_op(files: &mut Files, op: &Op) -> Result<Change> {
    Ok(match op {
        Op::Add { path, lines } => {
            let f = files.get(path)?;
            if f.lines.is_some() {
                bail!("{path}: already exists\nhint: use `*** Update File: {path}`");
            }
            f.lines = Some(lines.clone());
            f.dirty = true;
            Change::Added {
                path: path.clone(),
                lines: lines.len(),
            }
        }
        Op::Delete { path } => {
            let n = files.lines(path)?.len();
            let f = files.get(path)?;
            f.lines = None;
            f.dirty = true;
            Change::Deleted {
                path: path.clone(),
                lines: n,
            }
        }
        Op::Update {
            path,
            move_to,
            chunks,
        } => {
            files.lines(path)?;
            let f = files.get(path)?;
            for (i, c) in chunks.iter().enumerate() {
                let at = locate_chunk(f.lines.as_ref().unwrap(), c)
                    .map_err(|e| anyhow!("{path}: hunk {}: {e}", i + 1))?;
                f.splice(at, c.old.len(), &c.new);
            }
            let (added, removed) = chunks
                .iter()
                .fold((0, 0), |(a, r), c| (a + c.added, r + c.removed));
            files.get(path)?.dirty = true;
            match move_to {
                None => Change::Modified {
                    path: path.clone(),
                    added,
                    removed,
                },
                Some(to) => {
                    let src = files.get(path)?;
                    let (lines, crlf, trailing, perms, edits) = (
                        src.lines.take(),
                        src.crlf,
                        src.trailing_newline,
                        src.perms.clone(),
                        std::mem::take(&mut src.edits),
                    );
                    let dst = files.get(to)?;
                    if dst.lines.is_some() {
                        bail!("{to}: already exists, refusing to move {path} over it");
                    }
                    *dst = File {
                        lines,
                        crlf,
                        trailing_newline: trailing,
                        perms,
                        existed: dst.existed,
                        dirty: true,
                        edits,
                    };
                    Change::Moved {
                        from: path.clone(),
                        to: to.clone(),
                        added,
                        removed,
                    }
                }
            }
        }
        Op::ReplaceSymbol {
            path,
            symbol,
            lines: body,
        } => {
            let lines = files.lines(path)?;
            let src = lines.join("\n");
            let Some(o) = Lang::from_path(Path::new(path)).and_then(|l| outline(l, &src)) else {
                bail!(
                    "{path}: no syntax support\nhint: use `*** Update File: {path}` with context lines"
                );
            };
            let (start, end) = match resolve(&o, symbol) {
                Resolve::Found(s) => (s.start_line, s.end_line),
                Resolve::Missing => {
                    bail!("{path}: no symbol `{symbol}`\nhint: acus outline {path}")
                }
                Resolve::Ambiguous(v) => {
                    let c: Vec<_> = v.iter().map(|s| format!("{path}#{}", s.qual)).collect();
                    bail!("{path}: `{symbol}` is ambiguous\nhint: {}", c.join(", "))
                }
            };
            let f = files.get(path)?;
            f.splice(start - 1, end - start + 1, body);
            f.dirty = true;
            Change::Modified {
                path: path.clone(),
                added: body.len(),
                removed: end - start + 1,
            }
        }
    })
}

type Eq = fn(&str, &str) -> bool;
/// Exact first, then ignoring trailing, then all surrounding whitespace.
const PASSES: [Eq; 3] = [
    |a, b| a == b,
    |a, b| a.trim_end() == b.trim_end(),
    |a, b| a.trim() == b.trim(),
];

/// Where `c.old` sits in `lines` (insertion point for pure additions).
fn locate_chunk(lines: &[String], c: &Chunk) -> Result<usize> {
    let mut from = 0;
    for a in &c.anchors {
        // Anchors may also be a line's start (`@@ fn parse` for `    fn parse(&self) {`).
        let found = PASSES
            .iter()
            .chain([&((|l, a| l.trim_start().starts_with(a.trim())) as Eq)])
            .find_map(|eq| (from..lines.len()).find(|&i| eq(&lines[i], a)));
        from = found.ok_or_else(|| anyhow!("`@@ {a}` not found"))? + 1;
    }
    if c.old.is_empty() {
        Ok(if c.eof || c.anchors.is_empty() {
            lines.len()
        } else {
            from
        })
    } else {
        locate(lines, &c.old, from, c.eof)
    }
}

fn locate(lines: &[String], old: &[String], from: usize, eof: bool) -> Result<usize> {
    let last = lines.len().checked_sub(old.len());
    for eq in PASSES {
        let Some(last) = last else { break };
        let hits: Vec<usize> = (from..=last)
            .filter(|&i| !eof || i == last)
            .filter(|&i| old.iter().zip(&lines[i..]).all(|(o, l)| eq(l, o)))
            .collect();
        match hits.as_slice() {
            [] => continue,
            [i] => return Ok(*i),
            _ => {
                let at: Vec<_> = hits.iter().map(|i| (i + 1).to_string()).collect();
                bail!(
                    "context matches at lines {}\nhint: add an `@@ <enclosing line>` anchor or more context",
                    at.join(", ")
                )
            }
        }
    }
    let first = old.iter().find(|l| !l.trim().is_empty()).unwrap_or(&old[0]);
    bail!(
        "context not found, first expected line `{first}`\nhint: re-read the file (acus show) and retry"
    )
}

/// Writes every dirty file to a temp file next to it, then renames them all into place.
/// Temp files are removed on any error (NamedTempFile's Drop); originals stay untouched
/// until the first rename.
fn commit(root: &Path, map: BTreeMap<String, File>) -> Result<()> {
    let mut staged = Vec::new();
    let mut deletes = Vec::new();
    for (path, f) in map.into_iter().filter(|(_, f)| f.dirty) {
        let full = root.join(&path);
        let Some(lines) = f.lines else {
            if f.existed {
                deletes.push((path, full));
            }
            continue;
        };
        let mut text = lines.join("\n");
        if f.trailing_newline && !lines.is_empty() {
            text.push('\n');
        }
        if f.crlf {
            text = text.replace('\n', "\r\n");
        }
        let dir = full
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        fs::create_dir_all(dir).with_context(|| format!("{path}: cannot create directory"))?;
        let mut tmp = tempfile::NamedTempFile::new_in(dir)
            .with_context(|| format!("{path}: cannot create temp file"))?;
        tmp.write_all(text.as_bytes())
            .and_then(|_| tmp.as_file().sync_all())
            .with_context(|| format!("{path}: cannot write"))?;
        if let Some(p) = f.perms {
            fs::set_permissions(tmp.path(), p)
                .with_context(|| format!("{path}: cannot copy permissions"))?;
        }
        staged.push((path, full, tmp));
    }
    let mut done: Vec<String> = Vec::new();
    for (path, full, tmp) in staged {
        if let Err(e) = tmp.persist(&full) {
            bail!("{path}: rename failed: {}{}", e.error, written(&done));
        }
        done.push(path);
    }
    for (path, full) in deletes {
        if let Err(e) = fs::remove_file(&full) {
            bail!("{path}: delete failed: {e}{}", written(&done));
        }
        done.push(path);
    }
    Ok(())
}

fn written(done: &[String]) -> String {
    if done.is_empty() {
        String::new()
    } else {
        format!("\nhint: already written: {}", done.join(", "))
    }
}
