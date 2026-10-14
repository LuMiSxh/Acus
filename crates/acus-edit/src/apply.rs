use crate::parse::{Block, Dest, InsertAt};
use crate::{Chunk, Op};
use acus_syntax::{Lang, Resolve, attached_start, outline, resolve};
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
    /// `path block N` of every SEARCH block that matched only after its line-number prefixes were removed.
    pub numbered: Vec<String>,
    /// `path block N` of every SEARCH block that matched only ignoring trailing whitespace.
    pub trailing: Vec<String>,
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
    numbered: Vec<String>,
    trailing: Vec<String>,
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
        numbered: Vec::new(),
        trailing: Vec::new(),
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
        commit(root, std::mem::take(&mut files.map))?;
    }
    Ok(Applied {
        changes,
        regions,
        numbered: files.numbered,
        trailing: files.trailing,
    })
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
            let (mut numbered, mut trailing) = (Vec::new(), Vec::new());
            let (mut added, mut removed) = (0, 0);
            for (i, c) in chunks.iter().enumerate() {
                let (edit, fix) = search_block(f.lines.as_ref().unwrap(), c, path, i + 1)?;
                let name = format!("{path} block {}", i + 1);
                match fix {
                    Some(Fix::Numbers) => numbered.push(name),
                    Some(Fix::Trailing) => trailing.push(name),
                    None => {}
                }
                added += edit.added;
                removed += edit.removed;
                f.splice(edit.at, edit.old, &edit.new);
            }
            files.numbered.extend(numbered);
            files.trailing.extend(trailing);
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
            if body.is_empty() {
                bail!(
                    "{path}#{symbol}: Replace Symbol needs `+` lines\nhint: to remove it use `*** Delete Symbol: {path}#{symbol}`"
                );
            }
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
        Op::Insert {
            path,
            at,
            after,
            lines: body,
        } => {
            let at_line = match at {
                InsertAt::Symbol(s) => {
                    let (start, end) = symbol_lines(files, path, s)?;
                    if *after {
                        end
                    } else {
                        with_attached(files.lines(path)?, path, start, end).0
                    }
                }
                InsertAt::Line(n) => {
                    let len = files.lines(path)?.len();
                    if *n > len {
                        bail!(
                            "{path}:{n}: the file has {len} lines at this point of the patch\nhint: acus show {path}"
                        );
                    }
                    if *after { *n } else { n - 1 }
                }
            };
            let f = files.get(path)?;
            f.splice(at_line, 0, body);
            f.dirty = true;
            Change::Modified {
                path: path.clone(),
                added: body.len(),
                removed: 0,
            }
        }
        Op::Copy { from, to } => {
            let to = to.as_ref().expect("parse checks the destination");
            let src = files.get(from)?;
            let Some(lines) = src.lines.clone() else {
                bail!("{from}: file not found");
            };
            let (crlf, trailing_newline, perms) =
                (src.crlf, src.trailing_newline, src.perms.clone());
            let dst = files.get(to)?;
            if dst.lines.is_some() {
                bail!("{to}: already exists, refusing to copy {from} over it");
            }
            let n = lines.len();
            *dst = File {
                lines: Some(lines),
                crlf,
                trailing_newline,
                perms,
                existed: dst.existed,
                dirty: true,
                edits: Vec::new(),
            };
            Change::Added {
                path: to.clone(),
                lines: n,
            }
        }
    }])
}

/// 0-based, end-exclusive line range of `path#symbol`.
fn symbol_lines(files: &mut Files, path: &str, symbol: &str) -> Result<(usize, usize)> {
    let src = files.lines(path)?.join("\n");
    let Some(o) = Lang::from_path(Path::new(path)).and_then(|l| outline(l, &src)) else {
        bail!(
            "{path}: no syntax support\nhint: use `*** Update File: {path}` with SEARCH/REPLACE blocks"
        );
    };
    match resolve(&o, symbol) {
        Resolve::Found(s) => Ok((s.start_line - 1, s.end_line)),
        Resolve::Missing => bail!("{path}: no symbol `{symbol}`\nhint: acus outline {path}"),
        Resolve::Ambiguous(v) => {
            let c: Vec<_> = v
                .iter()
                .map(|s| format!("{path}#{} ({}-{})", s.qual, s.start_line, s.end_line))
                .collect();
            bail!(
                "{path}: `{symbol}` is ambiguous\nhint: {}; or edit the text with `*** Update File:` SEARCH blocks",
                c.join(", ")
            )
        }
    }
}

/// Grows a symbol's range upwards over the doc comments, attributes and decorators that belong to it.
fn with_attached(lines: &[String], path: &str, start: usize, end: usize) -> (usize, usize) {
    (attached_start(lines, path, start), end)
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

/// Languages a symbol may move between; TS, JS and Svelte scripts mix freely.
fn family(path: &str) -> Option<&'static str> {
    Some(match Lang::from_path(Path::new(path))? {
        Lang::TypeScript | Lang::Tsx | Lang::JavaScript | Lang::Svelte => "js",
        l => l.name(),
    })
}

fn move_symbol(files: &mut Files, path: &str, symbol: &str, to: &Dest) -> Result<Vec<Change>> {
    let dest = match to {
        Dest::Before { path, .. } | Dest::After { path, .. } | Dest::End { path } => path,
    };
    if let (Some(a), Some(b)) = (family(path), family(dest))
        && a != b
    {
        bail!("{path}#{symbol}: cannot move {a} code into {dest} ({b})");
    }
    let (start, end) = symbol_lines(files, path, symbol)?;
    if let Dest::Before { path: p, symbol: s } | Dest::After { path: p, symbol: s } = to
        && p == path
    {
        let (ds, _) = symbol_lines(files, p, s)?;
        if (start..end).contains(&ds) {
            bail!("{path}#{symbol}: cannot move a symbol next to itself or into its own body");
        }
    }
    let (start, end) = with_attached(files.lines(path)?, path, start, end);
    let lines = files.lines(path)?;
    let block = lines[start..end].to_vec();
    let (s, e) = with_blank(lines, start, end);
    let f = files.get(path)?;
    f.splice(s, e - s, &[]);
    f.dirty = true;
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
    // The CLI passes "" for the working directory, which the walker cannot open.
    let root = if root.as_os_str().is_empty() {
        Path::new(".")
    } else {
        root
    };
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
    // Directories are taken whole; globs select files anywhere below the root.
    let mut walks = Vec::new();
    if !dirs.is_empty() {
        walks.push((dirs, Vec::new()));
    }
    if !globs.is_empty() {
        walks.push((
            vec![root.to_path_buf()],
            globs.into_iter().cloned().collect(),
        ));
    }
    let mut found = Vec::new();
    for (roots, include) in walks {
        let hits = Mutex::new(Vec::new());
        let opts = acus_walk::WalkOpts {
            roots,
            include,
            ..Default::default()
        };
        acus_walk::walk(&opts, |p| {
            let rel = p.strip_prefix(root).map(PathBuf::from).unwrap_or(p.into());
            hits.lock().unwrap().push(acus_walk::display_path(&rel));
        })?;
        found.extend(hits.into_inner().unwrap());
    }
    found.sort();
    found.dedup();
    found.retain(|p| !out.iter().any(|(o, _)| o == p));
    out.extend(found.into_iter().map(|p| (p, false)));
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
        let re = Regex::new(&pat).map_err(|e| {
            anyhow!(
                "Replace All block {}: {e}\nhint: escape ( [ {{ . with \\, or use a SEARCH block for literal text",
                i + 1
            )
        })?;
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
        let hint = if blocks[k].regex {
            "the regex runs per file with (?m); `.` does not cross lines, use \\n or (?s)"
        } else {
            "search text is literal and whitespace-exact; use `<<<<<<< REGEX` for patterns"
        };
        bail!(
            "Replace All block {}: `{}` matches nothing in {}\nhint: {hint}",
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
/// A fallback that a SEARCH block needed to match.
enum Fix {
    /// The `acus show` line-number prefixes were removed from the block.
    Numbers,
    /// Trailing whitespace was ignored.
    Trailing,
}

/// One replacement of a SEARCH block: the `old` lines from `at` become `new`.
struct Edit {
    at: usize,
    old: usize,
    new: Vec<String>,
    added: usize,
    removed: usize,
}

/// Byte offsets where `needle` occurs in `text`, overlapping occurrences included.
fn occurrences(text: &str, needle: &str) -> Vec<usize> {
    (0..text.len())
        .filter(|&i| text.is_char_boundary(i) && text[i..].starts_with(needle))
        .collect()
}

/// Every occurrence of `search` in `lines`, each as the edit that replaces it by `replace`.
/// With `trim`, lines are compared without trailing whitespace, so `search` must be given that
/// way too.
fn search_edits(lines: &[String], search: &str, replace: &str, trim: bool) -> Vec<Edit> {
    let view: Vec<&str> = lines
        .iter()
        .map(|l| if trim { l.trim_end() } else { l.as_str() })
        .collect();
    let text = view.join("\n");
    let starts: Vec<usize> = std::iter::once(0)
        .chain(text.match_indices('\n').map(|(i, _)| i + 1))
        .collect();
    let line_of = |b: usize| starts.partition_point(|&s| s <= b) - 1;
    occurrences(&text, search)
        .into_iter()
        .map(|ms| {
            let me = ms + search.len();
            let (first, last) = (line_of(ms), line_of(me));
            let head = &lines[first][..ms - starts[first]];
            let tail = &lines[last][me - starts[last]..];
            let new: Vec<String> = format!("{head}{replace}{tail}")
                .split('\n')
                .map(Into::into)
                .collect();
            let old = &lines[first..=last];
            let (added, removed) = change_stats(old, &new);
            Edit {
                at: first,
                old: old.len(),
                new,
                added,
                removed,
            }
        })
        .collect()
}

/// Added and removed line counts of replacing `old` by `new`, not counting the lines they
/// share at either end.
fn change_stats(old: &[String], new: &[String]) -> (usize, usize) {
    let pre = old.iter().zip(new).take_while(|(a, b)| a == b).count();
    let suf = old[pre..]
        .iter()
        .rev()
        .zip(new[pre..].iter().rev())
        .take_while(|(a, b)| a == b)
        .count();
    (new.len() - pre - suf, old.len() - pre - suf)
}

/// `line` without the `acus show` line-number prefix (`12<TAB>`, with leading spaces or a `>`
/// marker allowed), if it has one.
fn strip_number(line: &str) -> Option<&str> {
    let t = line.trim_start_matches(' ');
    let t = t.strip_prefix('>').unwrap_or(t);
    let digits = t.len() - t.trim_start_matches(|c: char| c.is_ascii_digit()).len();
    if digits == 0 {
        return None;
    }
    let rest = &t[digits..];
    rest.strip_prefix('>').unwrap_or(rest).strip_prefix('\t')
}

/// `lines` without their line-number prefixes, if every non-empty line has one.
fn unnumbered(lines: &[String]) -> Option<Vec<String>> {
    lines
        .iter()
        .map(|l| match strip_number(l) {
            Some(rest) => Some(rest.to_owned()),
            None if l.is_empty() => Some(String::new()),
            None => None,
        })
        .collect()
}

/// Applies the SEARCH block `c` (block `n` of its file) to `lines`: its text must occur exactly
/// once in the file. The fallbacks run only when it does not occur at all, and each must also
/// give exactly one match.
fn search_block(lines: &[String], c: &Chunk, path: &str, n: usize) -> Result<(Edit, Option<Fix>)> {
    let (search, replace) = (c.old.join("\n"), c.new.join("\n"));
    let mut tries = vec![(None, search_edits(lines, &search, &replace, false))];
    if let Some(old) = unnumbered(&c.old) {
        let new = unnumbered(&c.new).unwrap_or_else(|| c.new.clone());
        let found = search_edits(lines, &old.join("\n"), &new.join("\n"), false);
        tries.push((Some(Fix::Numbers), found));
    }
    let trimmed = c
        .old
        .iter()
        .map(|l| l.trim_end())
        .collect::<Vec<_>>()
        .join("\n");
    tries.push((
        Some(Fix::Trailing),
        search_edits(lines, &trimmed, &replace, true),
    ));
    for (fix, mut found) in tries {
        match found.len() {
            0 => {}
            1 => return Ok((found.swap_remove(0), fix)),
            k => {
                let at: Vec<String> = found.iter().map(|e| (e.at + 1).to_string()).collect();
                bail!(
                    "{path}: block {n}: SEARCH matches {k} times (lines {}); add surrounding lines to make it unique, or use *** Replace All: {path}",
                    at.join(", ")
                );
            }
        }
    }
    let first = c
        .old
        .iter()
        .find(|l| !l.trim().is_empty())
        .map_or("", |l| l.trim());
    bail!(
        "{path}: block {n}: SEARCH not found, first line `{first}`\nhint: copy the text as the file has it, a part of a line is fine; re-read it with acus show{}",
        nearest(lines, &c.old, path)
    )
}

/// The window of `lines` most like `old`, as `nearest: PATH:A-B` plus its first lines, so a
/// failed SEARCH block can be fixed in one step.
fn nearest(lines: &[String], old: &[String], path: &str) -> String {
    const SHOWN: usize = 8;
    let n = old.len().min(lines.len());
    if n == 0 {
        return String::new();
    }
    let want: Vec<_> = old.iter().map(|l| bigrams(l)).collect();
    let have: Vec<_> = lines.iter().map(|l| bigrams(l)).collect();
    let mut best: Option<(f32, usize)> = None;
    for i in 0..=lines.len() - n {
        let score: f32 = (0..n)
            .map(|k| similarity(&old[k], &lines[i + k], &want[k], &have[i + k]))
            .sum();
        if best.is_none_or(|(b, _)| score > b) {
            best = Some((score, i));
        }
    }
    let Some((_, i)) = best.filter(|&(s, _)| s / n as f32 >= 0.4) else {
        return String::new();
    };
    let span = if n == 1 {
        format!("{}", i + 1)
    } else {
        format!("{}-{}", i + 1, i + n)
    };
    let mut out = format!("\nnearest: {path}:{span}");
    for k in 0..n.min(SHOWN) {
        out.push_str(&format!("\n{}\t{}", i + k + 1, lines[i + k]));
    }
    if n > SHOWN {
        out.push_str(&format!("\n… {} more lines", n - SHOWN));
    }
    out
}

/// Sorted character pairs of the trimmed line.
fn bigrams(line: &str) -> Vec<(char, char)> {
    let cs: Vec<char> = line.trim().chars().collect();
    let mut v: Vec<_> = cs.windows(2).map(|w| (w[0], w[1])).collect();
    v.sort_unstable();
    v
}

/// 0..=1: 1 for equal text ignoring surrounding whitespace, else the Dice coefficient of the bigrams.
fn similarity(a: &str, b: &str, ba: &[(char, char)], bb: &[(char, char)]) -> f32 {
    if a.trim() == b.trim() {
        return 1.0;
    }
    if ba.is_empty() || bb.is_empty() {
        return 0.0;
    }
    let (mut i, mut j, mut common) = (0, 0, 0);
    while i < ba.len() && j < bb.len() {
        match ba[i].cmp(&bb[j]) {
            std::cmp::Ordering::Less => i += 1,
            std::cmp::Ordering::Greater => j += 1,
            std::cmp::Ordering::Equal => {
                common += 1;
                i += 1;
                j += 1;
            }
        }
    }
    2.0 * common as f32 / (ba.len() + bb.len()) as f32
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
