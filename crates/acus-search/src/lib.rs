//! Regex search over files with enclosing-symbol context.

use acus_syntax::{Lang, Outline, outline};
use acus_walk::{WalkOpts, display_path, walk};
use anyhow::{Context, Result};
use grep_regex::RegexMatcherBuilder;
use grep_searcher::{BinaryDetection, SearcherBuilder, sinks::Lossy};
use serde::Serialize;
use std::sync::Mutex;

pub struct FindOpts {
    pub patterns: Vec<String>,
    pub walk: WalkOpts,
    pub ignore_case: bool,
    /// Treat patterns as literal strings.
    pub fixed: bool,
    /// Parse files with hits to attach enclosing symbols.
    pub syntax: bool,
}

#[derive(Debug, Serialize)]
pub struct Hit {
    pub line: usize,
    pub text: String,
}

#[derive(Debug, Serialize)]
pub struct FileHits {
    pub path: String,
    pub lang: Option<Lang>,
    pub hits: Vec<Hit>,
    /// File contents, kept so callers can print blocks without re-reading.
    #[serde(skip)]
    pub source: String,
    #[serde(skip)]
    pub outline: Option<Outline>,
}

/// Files with at least one hit, sorted by path.
pub fn find(o: &FindOpts) -> Result<Vec<FileHits>> {
    let pats: Vec<String> = if o.fixed {
        o.patterns.iter().map(|p| escape(p)).collect()
    } else {
        o.patterns.clone()
    };
    let matcher = RegexMatcherBuilder::new()
        .case_insensitive(o.ignore_case)
        .build_many(&pats)
        .context("invalid pattern")?;
    let out = Mutex::new(Vec::new());
    walk(&o.walk, |path| {
        // ponytail: unreadable files are skipped like rg does without --debug
        let Ok(bytes) = std::fs::read(path) else {
            return;
        };
        let mut searcher = SearcherBuilder::new()
            .binary_detection(BinaryDetection::quit(0))
            .line_number(true)
            .build();
        let mut hits = Vec::new();
        let sink = Lossy(|n, line| {
            hits.push(Hit {
                line: n as usize,
                text: line.trim_end_matches(['\n', '\r']).to_owned(),
            });
            Ok(true)
        });
        if searcher.search_slice(&matcher, &bytes, sink).is_err() || hits.is_empty() {
            return;
        }
        let lang = Lang::from_path(path);
        let source = String::from_utf8_lossy(&bytes).into_owned();
        let outline = lang.filter(|_| o.syntax).and_then(|l| outline(l, &source));
        out.lock().unwrap().push(FileHits {
            path: display_path(path),
            lang,
            hits,
            source,
            outline,
        });
    })?;
    let mut v = out.into_inner().unwrap();
    v.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(v)
}

fn escape(s: &str) -> String {
    let mut e = String::with_capacity(s.len());
    for c in s.chars() {
        if r"\.+*?()|[]{}^$".contains(c) {
            e.push('\\');
        }
        e.push(c);
    }
    e
}
