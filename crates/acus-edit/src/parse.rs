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

pub fn parse(text: &str) -> Result<Vec<Op>> {
    let mut ops: Vec<Op> = Vec::new();
    for (i, line) in text.trim_end().lines().enumerate() {
        let n = i + 1;
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
        } else if line.starts_with("***") {
            bail!(
                "patch line {n}: unknown directive `{line}`\nhint: use *** Add File:, *** Update File:, *** Delete File:, *** Move to:, *** Replace Symbol:"
            );
        } else {
            body(ops.last_mut(), line, n)?;
        }
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
    }
    if ops.is_empty() {
        bail!("empty patch\nhint: start with `*** Update File: PATH`");
    }
    Ok(ops)
}

fn body(op: Option<&mut Op>, line: &str, n: usize) -> Result<()> {
    match op {
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
                _ => bail!(
                    "patch line {n}: expected ` `, `-`, `+` or `@@` at line start, got `{line}`"
                ),
            }
        }
        Some(Op::Delete { path }) => {
            bail!("patch line {n}: unexpected content after `*** Delete File: {path}`")
        }
        None => bail!(
            "patch line {n}: content before any `***` directive\nhint: start with `*** Update File: PATH`"
        ),
    }
    Ok(())
}
