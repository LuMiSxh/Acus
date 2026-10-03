use crate::{Outline, Symbol};
use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    Whole,
    /// 1-based inclusive line range.
    Lines(usize, usize),
    Symbol(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Address {
    pub path: String,
    pub target: Target,
}

impl Address {
    /// `path#Sym`, `path:START-END`, `path:LINE` or `path`. A `:` suffix counts only when it
    /// is numeric, so Windows drive letters survive.
    pub fn parse(s: &str) -> Address {
        if let Some((path, sym)) = s.split_once('#') {
            return Address {
                path: path.into(),
                target: Target::Symbol(sym.into()),
            };
        }
        if let Some((path, range)) = s.rsplit_once(':') {
            let (a, b) = range.split_once('-').unwrap_or((range, range));
            if let (Ok(a), Ok(b)) = (a.parse(), b.parse()) {
                return Address {
                    path: path.into(),
                    target: Target::Lines(a, b),
                };
            }
        }
        Address {
            path: s.into(),
            target: Target::Whole,
        }
    }
}

impl fmt::Display for Address {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match &self.target {
            Target::Whole => write!(f, "{}", self.path),
            Target::Lines(a, b) if a == b => write!(f, "{}:{a}", self.path),
            Target::Lines(a, b) => write!(f, "{}:{a}-{b}", self.path),
            Target::Symbol(s) => write!(f, "{}#{s}", self.path),
        }
    }
}

pub enum Resolve<'a> {
    Found(&'a Symbol),
    Ambiguous(Vec<&'a Symbol>),
    Missing,
}

/// Exact qualified name first, then a unique `::`-suffix or plain name.
pub fn resolve<'a>(o: &'a Outline, query: &str) -> Resolve<'a> {
    match resolve_as(o, query) {
        // `Class.method` as written in Python, TS or Swift; keys like `a.b` match first.
        Resolve::Missing if query.contains('.') => resolve_as(o, &query.replace('.', "::")),
        r => r,
    }
}

fn resolve_as<'a>(o: &'a Outline, query: &str) -> Resolve<'a> {
    let suffix = format!("::{query}");
    let exact: Vec<&Symbol> = o.symbols.iter().filter(|s| s.qual == query).collect();
    let hits = if exact.is_empty() {
        o.symbols
            .iter()
            .filter(|s| s.qual.ends_with(&suffix) || s.name == query)
            .collect()
    } else {
        exact
    };
    // A type and its impl/extension share a qualified name; the one with children wins.
    let has_children = |s: &Symbol| {
        let prefix = format!("{}::", s.qual);
        o.symbols.iter().any(|c| {
            c.qual.starts_with(&prefix) && s.start_line <= c.start_line && c.end_line <= s.end_line
        })
    };
    match hits.as_slice() {
        [] => Resolve::Missing,
        [s] => Resolve::Found(s),
        // Same-kind twins without children (two `## Usage` headings) stay ambiguous.
        [first, ..] if hits.iter().all(|s| s.qual == first.qual) => {
            match hits.iter().find(|s| has_children(s)) {
                Some(s) => Resolve::Found(s),
                None if hits.iter().all(|s| s.kind == first.kind) => Resolve::Ambiguous(hits),
                None => Resolve::Found(first),
            }
        }
        _ => Resolve::Ambiguous(hits),
    }
}
