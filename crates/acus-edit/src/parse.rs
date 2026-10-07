use anyhow::{Result, bail};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Op {
    Add {
        path: String,
        lines: Vec<String>,
    },
    Delete {
        path: String,
    },
    Update {
        path: String,
        move_to: Option<String>,
        chunks: Vec<Chunk>,
    },
    ReplaceSymbol {
        path: String,
        symbol: String,
        lines: Vec<String>,
    },
    /// Every occurrence of each block's search text in every listed file.
    ReplaceAll {
        paths: Vec<String>,
        blocks: Vec<Block>,
    },
    DeleteSymbol {
        path: String,
        symbol: String,
    },
    MoveSymbol {
        path: String,
        symbol: String,
        to: Option<Dest>,
    },
}

/// A `<<<<<<< SEARCH` (or `REGEX`) / `=======` / `>>>>>>> REPLACE` block of `*** Replace All`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Block {
    pub regex: bool,
    pub search: Vec<String>,
    pub replace: Vec<String>,
}

/// Where `*** Move Symbol` puts the symbol.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Dest {
    Before { path: String, symbol: String },
    After { path: String, symbol: String },
    End { path: String },
}

/// One hunk: `old` (context + removed) is replaced by `new` (context + added).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Chunk {
    /// `@@` lines: each is searched for in turn before `old`.
    pub anchors: Vec<String>,
    pub old: Vec<String>,
    pub new: Vec<String>,
    pub added: usize,
    pub removed: usize,
    /// `*** End of File`: `old` must match at the end of the file.
    pub eof: bool,
}

impl Chunk {
    fn is_empty(&self) -> bool {
        self.anchors.is_empty() && self.old.is_empty() && self.new.is_empty()
    }
}

/// Inside a SEARCH/REPLACE block: false while reading the search half, true for the replacement,
/// plus how many blocks the replacement itself opened (so it can contain example blocks).
type Stage = Option<(bool, u32)>;

fn sym_addr(rest: &str, n: usize, directive: &str) -> Result<(String, String)> {
    match rest.trim().split_once('#') {
        Some((p, s)) if !p.is_empty() && !s.is_empty() => Ok((p.into(), s.into())),
        _ => bail!("patch line {n}: expected `*** {directive}: path#Symbol`"),
    }
}

pub fn parse(text: &str) -> Result<Vec<Op>> {
    /// The accepted directives, as the unknown-directive error lists them.
    const DIRECTIVES: &str = "*** Add File: PATH | *** Update File: PATH | *** Delete File: PATH | *** Move to: PATH (after Update File) | *** Replace Symbol: PATH#Symbol | *** Delete Symbol: PATH#Symbol | *** Move Symbol: PATH#Symbol (then *** Before: PATH#Sym, *** After: PATH#Sym or *** To: PATH) | *** Replace All: PATH [PATH...] (files, directories or globs, then `<<<<<<< SEARCH` or `<<<<<<< REGEX`, `=======`, `>>>>>>> REPLACE`; see `acus patch --help`)";

    /// `did you mean …` for a directive with the right name but the wrong case or spacing, such as
    /// `*** Replace all:` or `*** Replace All:src`; empty otherwise.
    fn did_you_mean(line: &str) -> String {
        let name = line
            .trim_start_matches('*')
            .split(':')
            .next()
            .unwrap_or_default();
        let norm: String = name
            .chars()
            .filter(char::is_ascii_alphanumeric)
            .map(|c| c.to_ascii_lowercase())
            .collect();
        let form = match norm.as_str() {
            "addfile" => "*** Add File: PATH",
            "updatefile" => "*** Update File: PATH",
            "deletefile" => "*** Delete File: PATH",
            "moveto" => "*** Move to: PATH",
            "replacesymbol" => "*** Replace Symbol: PATH#Symbol",
            "deletesymbol" => "*** Delete Symbol: PATH#Symbol",
            "movesymbol" => "*** Move Symbol: PATH#Symbol",
            "replaceall" => "*** Replace All: PATH [PATH...]",
            _ => return String::new(),
        };
        format!("did you mean `{form}` (exact case, one space after the colon, a path after it)? ")
    }

    let mut ops: Vec<Op> = Vec::new();
    let mut stage: Stage = None;
    for (i, line) in text.trim_end().lines().enumerate() {
        let n = i + 1;
        // Block contents are verbatim, even lines that look like directives.
        if stage.is_some() {
            body(ops.last_mut(), line, n, &mut stage)?;
            continue;
        }
        if line == "*** Begin Patch" || line == "*** End Patch" {
            continue;
        }
        if let Some(p) = line.strip_prefix("*** Add File: ") {
            ops.push(Op::Add {
                path: p.trim().into(),
                lines: vec![],
            });
        } else if let Some(p) = line.strip_prefix("*** Delete File: ") {
            ops.push(Op::Delete {
                path: p.trim().into(),
            });
        } else if let Some(p) = line.strip_prefix("*** Update File: ") {
            ops.push(Op::Update {
                path: p.trim().into(),
                move_to: None,
                chunks: vec![Chunk::default()],
            });
        } else if let Some(p) = line.strip_prefix("*** Move to: ") {
            match ops.last_mut() {
                Some(Op::Update { move_to, .. }) => *move_to = Some(p.trim().into()),
                _ => bail!("patch line {n}: `*** Move to` must follow `*** Update File`"),
            }
        } else if let Some(rest) = line
            .strip_prefix("*** Replace Symbol: ")
            .or_else(|| line.strip_prefix("*** Replace Symbol "))
        {
            let Some((path, symbol)) = rest.trim().split_once('#') else {
                bail!("patch line {n}: expected `*** Replace Symbol: path#Symbol`");
            };
            ops.push(Op::ReplaceSymbol {
                path: path.into(),
                symbol: symbol.into(),
                lines: vec![],
            });
        } else if line == "*** End of File" {
            match ops.last_mut() {
                Some(Op::Update { chunks, .. }) => chunks.last_mut().unwrap().eof = true,
                _ => bail!("patch line {n}: `*** End of File` outside `*** Update File`"),
            }
        } else if let Some(p) = line.strip_prefix("*** Replace All: ") {
            ops.push(Op::ReplaceAll {
                paths: p.split_whitespace().map(Into::into).collect(),
                blocks: vec![],
            });
        } else if let Some(rest) = line.strip_prefix("*** Delete Symbol: ") {
            let (path, symbol) = sym_addr(rest, n, "Delete Symbol")?;
            ops.push(Op::DeleteSymbol { path, symbol });
        } else if let Some(rest) = line.strip_prefix("*** Move Symbol: ") {
            let (path, symbol) = sym_addr(rest, n, "Move Symbol")?;
            ops.push(Op::MoveSymbol {
                path,
                symbol,
                to: None,
            });
        } else if let Some((kind, rest)) = ["Before", "After", "To"]
            .iter()
            .find_map(|k| line.strip_prefix(&format!("*** {k}: ")).map(|r| (*k, r)))
        {
            let Some(Op::MoveSymbol { to, .. }) = ops.last_mut() else {
                bail!("patch line {n}: `*** {kind}:` must follow `*** Move Symbol:`");
            };
            *to = Some(match kind {
                "To" => Dest::End {
                    path: rest.trim().into(),
                },
                _ => {
                    let (path, symbol) = sym_addr(rest, n, kind)?;
                    if kind == "Before" {
                        Dest::Before { path, symbol }
                    } else {
                        Dest::After { path, symbol }
                    }
                }
            });
        } else if line.starts_with("***") {
            bail!(
                "patch line {n}: unknown directive `{line}`\nhint: {}accepted: {DIRECTIVES}",
                did_you_mean(line)
            );
        } else {
            body(ops.last_mut(), line, n, &mut stage)?;
        }
    }
    if stage.is_some() {
        bail!("unterminated SEARCH/REPLACE block\nhint: close it with `>>>>>>> REPLACE`");
    }
    ops.retain_mut(|op| {
        if let Op::Update { chunks, .. } = op {
            chunks.retain(|c| !c.is_empty());
        }
        true
    });
    for op in &ops {
        if let Op::Update {
            path,
            move_to: None,
            chunks,
        } = op
            && chunks.is_empty()
        {
            bail!("{path}: `*** Update File` without changes");
        }
        match op {
            Op::ReplaceAll { paths, blocks } if paths.is_empty() || blocks.is_empty() => {
                bail!("`*** Replace All` needs paths and at least one SEARCH/REPLACE block")
            }
            Op::MoveSymbol {
                path,
                symbol,
                to: None,
            } => bail!(
                "{path}#{symbol}: `*** Move Symbol` needs a destination\nhint: follow it with `*** Before: path#Sym`, `*** After: path#Sym` or `*** To: path`"
            ),
            _ => {}
        }
    }
    if ops.is_empty() {
        bail!("empty patch\nhint: start with `*** Update File: PATH`");
    }
    Ok(ops)
}

fn body(op: Option<&mut Op>, line: &str, n: usize, stage: &mut Stage) -> Result<()> {
    let opens = |l: &str| ["<<<<<<< SEARCH", "<<<<<<<"].contains(&l.trim_end());
    let closes = |l: &str| [">>>>>>> REPLACE", ">>>>>>>"].contains(&l.trim_end());
    let ends = matches!(stage, Some((true, 0))) && closes(line);
    if let Some((true, d)) = stage {
        if opens(line) || line.trim_end() == "<<<<<<< REGEX" {
            *d += 1;
        } else if closes(line) && *d > 0 {
            *d -= 1;
        }
    }
    match op {
        Some(Op::ReplaceAll { blocks, .. }) => match *stage {
            None if opens(line) || line.trim_end() == "<<<<<<< REGEX" => {
                blocks.push(Block {
                    regex: line.trim_end().ends_with("REGEX"),
                    ..Block::default()
                });
                *stage = Some((false, 0));
            }
            None => bail!("patch line {n}: expected `<<<<<<< SEARCH` or `<<<<<<< REGEX`"),
            Some((false, _)) if line.trim_end() == "=======" => *stage = Some((true, 0)),
            Some((true, _)) if ends => *stage = None,
            Some((r, _)) => {
                let b = blocks.last_mut().unwrap();
                if r { &mut b.replace } else { &mut b.search }.push(line.into());
            }
        },
        Some(Op::Update { chunks, .. }) if stage.is_some() || opens(line) => {
            match *stage {
                None => {
                    // Keep pending `@@` anchors; start fresh after a hunk with content.
                    let c = chunks.last().unwrap();
                    if !c.old.is_empty() || !c.new.is_empty() || c.eof {
                        chunks.push(Chunk::default());
                    }
                    *stage = Some((false, 0));
                }
                Some((false, _)) if line.trim_end() == "=======" => *stage = Some((true, 0)),
                Some((true, _)) if ends => {
                    let c = chunks.last_mut().unwrap();
                    if c.old.is_empty() {
                        bail!(
                            "patch line {n}: empty SEARCH\nhint: add an existing line to search for, or use a `@@` anchor with `+` lines"
                        );
                    }
                    // Count only the lines that really change.
                    let pre = c.old.iter().zip(&c.new).take_while(|(a, b)| a == b).count();
                    let suf = c.old[pre..]
                        .iter()
                        .rev()
                        .zip(c.new[pre..].iter().rev())
                        .take_while(|(a, b)| a == b)
                        .count();
                    c.removed = c.old.len() - pre - suf;
                    c.added = c.new.len() - pre - suf;
                    chunks.push(Chunk::default());
                    *stage = None;
                }
                Some((r, _)) => {
                    let c = chunks.last_mut().unwrap();
                    if r { &mut c.new } else { &mut c.old }.push(line.into());
                }
            }
        }
        Some(Op::Add { lines, .. } | Op::ReplaceSymbol { lines, .. }) => {
            match line.strip_prefix('+') {
                Some(l) => lines.push(l.into()),
                None => bail!("patch line {n}: every new line must start with `+`"),
            }
        }
        Some(Op::Update { chunks, .. }) => {
            let c = chunks.last_mut().unwrap();
            if let Some(anchor) = line.strip_prefix("@@") {
                if !c.old.is_empty() || !c.new.is_empty() || c.eof {
                    chunks.push(Chunk::default());
                }
                let anchor = anchor.trim();
                if !anchor.is_empty() {
                    chunks.last_mut().unwrap().anchors.push(anchor.into());
                }
                return Ok(());
            }
            let (tag, rest) = line.split_at(line.len().min(1));
            match tag {
                " " | "" => {
                    c.old.push(rest.into());
                    c.new.push(rest.into());
                }
                "-" => {
                    c.old.push(rest.into());
                    c.removed += 1;
                }
                "+" => {
                    c.new.push(rest.into());
                    c.added += 1;
                }
                _ if line.starts_with("<<<<<<< REGEX") => bail!(
                    "patch line {n}: REGEX blocks belong under `*** Replace All: PATH`, not `*** Update File`\nhint: use `<<<<<<< SEARCH` here, or move the block under `*** Replace All: PATH [PATH...]`"
                ),
                _ => bail!(
                    "patch line {n}: expected ` `, `-`, `+` or `@@` at line start, got `{line}`"
                ),
            }
        }
        Some(Op::Delete { path }) => {
            bail!("patch line {n}: unexpected content after `*** Delete File: {path}`")
        }
        Some(Op::DeleteSymbol { path, symbol } | Op::MoveSymbol { path, symbol, .. }) => {
            bail!("patch line {n}: unexpected content after `{path}#{symbol}`")
        }
        None => bail!(
            "patch line {n}: content before any `***` directive\nhint: start with `*** Update File: PATH`"
        ),
    }
    Ok(())
}
