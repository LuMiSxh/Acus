use crate::parse::{Block, Dest};
use crate::{Chunk, Op};
use acus_syntax::{Lang, Resolve, outline, resolve};
use anyhow::{Context, Result, anyhow, bail};
use regex::{Captures, Regex};
use std::collections::BTreeMap;
use std::fmt;
use std::fs::{self, Permissions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

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
        changes.extend(apply_op(&mut files, op)?);
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

fn apply_op(files: &mut Files, op: &Op) -> Result<Vec<Change>> {
    Ok(vec![match op {
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
            let (start, end) = symbol_lines(files, path, symbol)?;
            let f = files.get(path)?;
            f.splice(start, end - start, body);
            f.dirty = true;
            Change::Modified {
                path: path.clone(),
                added: body.len(),
                removed: end - start,
            }
        }
        Op::DeleteSymbol { path, symbol } => {
            let (start, end) = symbol_lines(files, path, symbol)?;
            let lines = files.lines(path)?;
            let (start, end) = with_attached(lines, path, start, end);
            let (s, e) = with_blank(lines, start, end);
            let f = files.get(path)?;
            f.splice(s, e - s, &[]);
            f.dirty = true;
            Change::Modified {
                path: path.clone(),
                added: 0,
                removed: end - start,
            }
        }
        Op::MoveSymbol { path, symbol, to } => {
            return move_symbol(files, path, symbol, to.as_ref().unwrap());
        }
        Op::ReplaceAll { paths, blocks } => return replace_all(files, paths, blocks),
    }])
}

/// 0-based, end-exclusive line range of `path#symbol`.
fn symbol_lines(files: &mut Files, path: &str, symbol: &str) -> Result<(usize, usize)> {
    let src = files.lines(path)?.join("\n");
    let Some(o) = Lang::from_path(Path::new(path)).and_then(|l| outline(l, &src)) else {
        bail!("{path}: no syntax support\nhint: use `*** Update File: {path}` with context lines");
    };
    match resolve(&o, symbol) {
        Resolve::Found(s) => Ok((s.start_line - 1, s.end_line)),
        Resolve::Missing => bail!("{path}: no symbol `{symbol}`\nhint: acus outline {path}"),
        Resolve::Ambiguous(v) => {
            let c: Vec<_> = v.iter().map(|s| format!("{path}#{}", s.qual)).collect();
            bail!("{path}: `{symbol}` is ambiguous\nhint: {}", c.join(", "))
        }
    }
}

/// Grows a symbol's range upwards over the doc comments, attributes and decorators that belong to it.
fn with_attached(lines: &[String], path: &str, mut start: usize, end: usize) -> (usize, usize) {
    let hash = matches!(
        Path::new(path).extension().and_then(|e| e.to_str()),
        Some("py" | "toml" | "yaml" | "yml" | "sh")
    );
    let markdown = path.ends_with(".md");
    while !markdown && start > 0 {
        let l = lines[start - 1].trim_start();
        let attached = ["///", "//", "#[", "#![", "@", "/*", "*"]
            .iter()
            .any(|p| l.starts_with(p))
            || (hash && l.starts_with('#'));
        if !attached {
            break;
        }
        start -= 1;
    }
    (start, end)
}

/// Adds one blank line next to a removed range so no double gap remains.
fn with_blank(lines: &[String], start: usize, end: usize) -> (usize, usize) {
    let blank = |i: usize| lines.get(i).is_some_and(|l| l.trim().is_empty());
    if blank(end) && (start == 0 || blank(start - 1) || end + 1 < lines.len()) {
        (start, end + 1)
    } else if start > 0 && blank(start - 1) {
        (start - 1, end)
    } else {
        (start, end)
    }
}

fn move_symbol(files: &mut Files, path: &str, symbol: &str, to: &Dest) -> Result<Vec<Change>> {
    let (start, end) = symbol_lines(files, path, symbol)?;
    let (start, end) = with_attached(files.lines(path)?, path, start, end);
    let lines = files.lines(path)?;
    let block = lines[start..end].to_vec();
    let (s, e) = with_blank(lines, start, end);
    let f = files.get(path)?;
    f.splice(s, e - s, &[]);
    f.dirty = true;
    let dest = match to {
        Dest::Before { path, .. } | Dest::After { path, .. } | Dest::End { path } => path,
    };
    let mut created = false;
    let at = match to {
        Dest::Before { path, symbol } => {
            let (s, e) = symbol_lines(files, path, symbol)?;
            let lines = files.lines(path)?;
            let mut ins = reindent(&block, indent(&lines[s]));
            ins.push(String::new());
            (with_attached(lines, path, s, e).0, ins)
        }
        Dest::After { path, symbol } => {
            let (s, e) = symbol_lines(files, path, symbol)?;
            let ins = reindent(&block, indent(&files.lines(path)?[s]));
            (e, [vec![String::new()], ins].concat())
        }
        Dest::End { path } => {
            let f = files.get(path)?;
            let lines = f.lines.get_or_insert_with(|| {
                created = true;
                vec![]
            });
            let gap = lines.last().is_some_and(|l| !l.trim().is_empty());
            let block = reindent(&block, "");
            let ins = if gap {
                [vec![String::new()], block].concat()
            } else {
                block
            };
            (lines.len(), ins)
        }
    };
    let (at, ins) = at;
    let f = files.get(dest)?;
    f.splice(at, 0, &ins);
    f.dirty = true;
    Ok(if dest == path {
        vec![Change::Modified {
            path: path.into(),
            added: block.len(),
            removed: block.len(),
        }]
    } else {
        vec![
            Change::Modified {
                path: path.into(),
                added: 0,
                removed: block.len(),
            },
            if created {
                Change::Added {
                    path: dest.clone(),
                    lines: ins.len(),
                }
            } else {
                Change::Modified {
                    path: dest.clone(),
                    added: block.len(),
                    removed: 0,
                }
            },
        ]
    })
}

fn indent(line: &str) -> &str {
    &line[..line.len() - line.trim_start().len()]
}

/// Shifts a moved block so its least indented line gets `to`, e.g. a method moved out of a class.
fn reindent(block: &[String], to: &str) -> Vec<String> {
    let from = block
        .iter()
        .filter(|l| !l.trim().is_empty())
        .map(|l| indent(l))
        .min_by_key(|i| i.len())
        .unwrap_or("");
    block
        .iter()
        .map(|l| match l.strip_prefix(from) {
            _ if l.trim().is_empty() => String::new(),
            Some(rest) => format!("{to}{rest}"),
            None => l.clone(),
        })
        .collect()
}

/// Files named by `*** Replace All`: plain files, directories (walked, respecting `.gitignore`) and globs.
fn expand_paths(root: &Path, specs: &[String]) -> Result<Vec<(String, bool)>> {
    let (globs, plain): (Vec<_>, Vec<_>) = specs.iter().partition(|s| s.contains(['*', '?', '[']));
    let mut out = Vec::new();
    let mut dirs = Vec::new();
    for p in plain {
        if root.join(p).is_dir() {
            dirs.push(root.join(p));
        } else {
            out.push((p.clone(), true));
        }
    }
    if !globs.is_empty() && dirs.is_empty() {
        dirs.push(root.to_path_buf());
    }
    if !dirs.is_empty() {
        let found = Mutex::new(Vec::new());
        let opts = acus_walk::WalkOpts {
            roots: dirs,
            include: globs.into_iter().cloned().collect(),
            ..Default::default()
        };
        acus_walk::walk(&opts, |p| {
            let rel = p.strip_prefix(root).map(PathBuf::from).unwrap_or(p.into());
            found.lock().unwrap().push(acus_walk::display_path(&rel));
        })?;
        let mut found = found.into_inner().unwrap();
        found.sort();
        out.extend(found.into_iter().map(|p| (p, false)));
    }
    Ok(out)
}

fn replace_all(files: &mut Files, specs: &[String], blocks: &[Block]) -> Result<Vec<Change>> {
    let mut res = Vec::new();
    for (i, b) in blocks.iter().enumerate() {
        let (search, replace) = (b.search.join("\n"), b.replace.join("\n"));
        let pat = if b.regex {
            format!("(?m){search}")
        } else {
            regex::escape(&search)
        };
        let re = Regex::new(&pat).map_err(|e| anyhow!("block {}: {e}", i + 1))?;
        res.push((re, replace, b.regex));
    }
    let mut hits = vec![0; blocks.len()];
    let mut changes = Vec::new();
    for (path, explicit) in expand_paths(files.root, specs)? {
        let lines = match files.lines(&path) {
            Ok(l) => l.clone(),
            Err(e) if explicit => return Err(e),
            // Binary or unreadable files met while walking a directory.
            Err(_) => continue,
        };
        let (mut lines, before) = (lines.clone(), lines.len());
        let mut hunks = Vec::new();
        for (k, (re, rep, expand)) in res.iter().enumerate() {
            let h = replace_in(&lines, re, rep, *expand);
            hits[k] += h.len();
            for (at, old, new) in h.into_iter().rev() {
                lines.splice(at..at + old, new.iter().cloned());
                hunks.push((at, old, new));
            }
        }
        if hunks.is_empty() {
            continue;
        }
        // Replay the hunks through `splice` so the printed regions are right.
        let f = files.get(&path)?;
        let marked = |f: &File| f.edits.iter().map(|e| e.1).sum::<usize>();
        let m0 = marked(f);
        for (at, old, new) in hunks {
            f.splice(at, old, &new);
        }
        f.dirty = true;
        // Lines touched by several blocks count once.
        let added = marked(f).saturating_sub(m0);
        changes.push(Change::Modified {
            path,
            added,
            removed: (added + before).saturating_sub(lines.len()),
        });
    }
    if let Some(k) = hits.iter().position(|&n| n == 0) {
        let first = blocks[k].search.iter().find(|l| !l.trim().is_empty());
        bail!(
            "Replace All block {}: `{}` matches nothing in {}\nhint: search text is literal and whitespace-exact; use `<<<<<<< REGEX` for patterns",
            k + 1,
            first.map_or("", |s| s.trim()),
            specs.join(" ")
        );
    }
    Ok(changes)
}

/// Replaces every match of `re` and returns `(line, old count, new lines)` per run of touched lines.
fn replace_in(
    lines: &[String],
    re: &Regex,
    rep: &str,
    expand: bool,
) -> Vec<(usize, usize, Vec<String>)> {
    let text = lines.join("\n");
    let starts: Vec<usize> = std::iter::once(0)
        .chain(text.match_indices('\n').map(|(i, _)| i + 1))
        .collect();
    let line_of = |b: usize| starts.partition_point(|&s| s <= b) - 1;
    let end_of = |l: usize| starts.get(l + 1).map_or(text.len(), |&s| s - 1);
    let mut hunks: Vec<(usize, usize, String)> = Vec::new();
    let mut pos = 0;
    for c in re.captures_iter(&text) {
        let m = c.get(0).unwrap();
        if m.is_empty() {
            continue;
        }
        let (l0, l1) = (line_of(m.start()), line_of(m.end()));
        let r = sub(&c, rep, expand);
        match hunks.last_mut() {
            Some((_, e, s)) if *e >= l0 => {
                s.push_str(&text[pos..m.start()]);
                s.push_str(&r);
                *e = (*e).max(l1);
            }
            last => {
                if let Some((_, e, s)) = last {
                    s.push_str(&text[pos..end_of(*e)]);
                }
                hunks.push((l0, l1, format!("{}{r}", &text[starts[l0]..m.start()])));
            }
        }
        pos = m.end();
    }
    if let Some((_, e, s)) = hunks.last_mut() {
        s.push_str(&text[pos..end_of(*e)]);
    }
    hunks
        .into_iter()
        .map(|(a, b, s)| {
            (
                a,
                b - a + 1,
                s.split('\n').map(Into::into).collect::<Vec<String>>(),
            )
        })
        .filter(|(a, n, new)| lines[*a..*a + n] != new[..])
        .collect()
}

fn sub(c: &Captures, rep: &str, expand: bool) -> String {
    let mut r = String::new();
    if expand {
        c.expand(rep, &mut r);
    } else {
        r.push_str(rep);
    }
    r
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
    // `from` stays on the anchor line itself: agents often repeat it as the first context line.
    let (mut from, mut after) = (0, 0);
    for a in &c.anchors {
        // Anchors may also be part of a line (`@@ fn parse` for `    pub fn parse(&self) {`).
        let found = PASSES
            .iter()
            .chain([&((|l, a| l.trim_start().starts_with(a.trim())) as Eq)])
            .chain([&((|l, a| l.contains(a.trim())) as Eq)])
            .find_map(|eq| (after..lines.len()).find(|&i| eq(&lines[i], a)));
        from = found.ok_or_else(|| anyhow!("`@@ {a}` not found"))?;
        after = from + 1;
    }
    if c.old.is_empty() {
        Ok(if c.eof || c.anchors.is_empty() {
            lines.len()
        } else {
            after
        })
    } else {
        // Hunks often edit the doc comment or attributes just above their `@@ fn …` anchor.
        locate(lines, &c.old, from, c.eof).or_else(|e| {
            let back = from.saturating_sub(c.old.len());
            if back < from {
                locate(
                    &lines[..from + c.old.len().min(lines.len() - from)],
                    &c.old,
                    back,
                    c.eof,
                )
                .map_err(|_| e)
            } else {
                Err(e)
            }
        })
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
    let j = old.iter().position(|l| !l.trim().is_empty()).unwrap_or(0);
    let first = old[j].trim();
    bail!(
        "context not found, first expected line `{first}`\n{}",
        near_miss(lines, old, j)
    )
}

/// Why the closest candidate failed, so the agent can fix the hunk without re-reading the file.
fn near_miss(lines: &[String], old: &[String], j: usize) -> String {
    let first = old[j].trim();
    for i in (j..lines.len()).filter(|&i| lines[i].trim() == first) {
        let start = i - j;
        if let Some(k) = (0..old.len()).find(|&k| {
            lines
                .get(start + k)
                .is_none_or(|l| l.trim() != old[k].trim())
        }) {
            let found = lines.get(start + k).map_or("end of file", |l| l.as_str());
            return format!(
                "hint: the first line matches at {}, but line {} is `{found}`, not `{}`",
                i + 1,
                start + k + 1,
                old[k]
            );
        }
    }
    if let Some(i) = lines
        .iter()
        .position(|l| !first.is_empty() && l.contains(first))
    {
        let how = if lines[i].trim().starts_with(first) {
            "only starts with"
        } else {
            "only contains"
        };
        return format!(
            "hint: line {} {how} it; context, `-` and SEARCH lines must be whole lines (for part of a line use `*** Replace All: path`): `{}`",
            i + 1,
            lines[i].trim()
        );
    }
    "hint: re-read the file (acus show) and retry".into()
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
