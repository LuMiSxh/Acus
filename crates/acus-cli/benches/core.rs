//! `cargo bench -p acus-cli` — library hot paths on a generated, deterministic Rust corpus.
use acus_search::{FindOpts, find};
use acus_syntax::{Lang, outline};
use acus_walk::{WalkOpts, walk};
use criterion::{Criterion, criterion_group, criterion_main};
use std::hint::black_box;
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};

const FILES: usize = 400;
const ITEMS: usize = 40;

/// One file: `ITEMS` structs, each with an impl of three methods (~20 lines per item).
fn source(file: usize) -> String {
    let mut s = String::from("use std::collections::HashMap;\n\n");
    for i in 0..ITEMS {
        s += &format!(
            "pub struct Item{i} {{\n    pub id: u64,\n    map: HashMap<String, u32>,\n}}\n\n\
             impl Item{i} {{\n    pub fn new(id: u64) -> Self {{\n        Self {{ id, map: HashMap::new() }}\n    }}\n\n\
             \x20   pub fn get(&self, k: &str) -> Option<u32> {{\n        self.map.get(k).copied()\n    }}\n\n\
             \x20   fn checksum_{file}_{i}(&self) -> u64 {{\n        // TODO tune\n        self.id * {i}\n    }}\n}}\n\n"
        );
    }
    s
}

fn corpus() -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    for f in 0..FILES {
        let dir = d.path().join(format!("m{}", f % 20));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(format!("f{f}.rs")), source(f)).unwrap();
    }
    d
}

fn opts(root: &Path, pat: &str, syntax: bool) -> FindOpts {
    FindOpts {
        patterns: vec![pat.into()],
        walk: WalkOpts {
            roots: vec![root.into()],
            ..Default::default()
        },
        ignore_case: false,
        fixed: false,
        syntax,
        word: false,
    }
}

fn bench(c: &mut Criterion) {
    let d = corpus();
    let root = d.path();
    c.bench_function("walk 400 files", |b| {
        b.iter(|| {
            let n = AtomicUsize::new(0);
            walk(
                &WalkOpts {
                    roots: vec![root.into()],
                    ..Default::default()
                },
                |_| {
                    n.fetch_add(1, Ordering::Relaxed);
                },
            )
            .unwrap();
            assert_eq!(n.into_inner(), FILES);
        })
    });
    c.bench_function("find rare, no syntax", |b| {
        b.iter(|| assert_eq!(find(&opts(root, "checksum_399_7", false)).unwrap().len(), 1))
    });
    c.bench_function("find rare + enclosing symbol", |b| {
        b.iter(|| assert_eq!(find(&opts(root, "checksum_399_7", true)).unwrap().len(), 1))
    });
    // Every file hits, so every file is parsed: the worst case for `find`.
    c.bench_function("find TODO in all files + symbols", |b| {
        b.iter(|| assert_eq!(find(&opts(root, "TODO", true)).unwrap().len(), FILES))
    });

    let src = source(0).repeat(10);
    let lines = src.lines().count();
    c.bench_function(&format!("outline rust {lines} lines"), |b| {
        b.iter(|| outline(Lang::Rust, black_box(&src)).unwrap())
    });

    let file = root.join("m0/f0.rs");
    let patch: String = (0..ITEMS)
        .map(|i| {
            format!("@@ impl Item{i} {{\n-        self.id * {i}\n+        self.id * {i} + 1\n")
        })
        .collect();
    let patch = format!("*** Begin Patch\n*** Update File: m0/f0.rs\n{patch}*** End Patch\n");
    c.bench_function("patch 40 hunks, check only", |b| {
        b.iter(|| acus_edit::apply(&acus_edit::parse(&patch).unwrap(), root, true).unwrap())
    });
    let original = std::fs::read_to_string(&file).unwrap();
    c.bench_function("patch 40 hunks, atomic write", |b| {
        b.iter(|| {
            std::fs::write(&file, &original).unwrap();
            acus_edit::apply(&acus_edit::parse(&patch).unwrap(), root, false).unwrap()
        })
    });
}

criterion_group!(benches, bench);
criterion_main!(benches);
