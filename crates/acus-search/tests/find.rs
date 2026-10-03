use acus_search::{FindOpts, find};
use acus_walk::WalkOpts;
use std::fs;

fn project() -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    fs::write(
        d.path().join("a.rs"),
        "fn alpha() {\n    let needle = 1;\n}\n// needle at top\n",
    )
    .unwrap();
    fs::write(d.path().join("b.txt"), "no match\nNEEDLE upper\n").unwrap();
    fs::write(d.path().join("bin.dat"), b"needle\0binary").unwrap();
    d
}

fn opts(d: &tempfile::TempDir, pats: &[&str]) -> FindOpts {
    FindOpts {
        patterns: pats.iter().map(|s| s.to_string()).collect(),
        walk: WalkOpts {
            roots: vec![d.path().into()],
            ..Default::default()
        },
        ignore_case: false,
        fixed: false,
        syntax: true,
    }
}

#[test]
fn finds_with_enclosing_symbols_sorted() {
    let d = project();
    let r = find(&opts(&d, &["needle"])).unwrap();
    assert_eq!(r.len(), 1, "binary files are skipped, case matters");
    let f = &r[0];
    assert!(f.path.ends_with("a.rs"));
    assert_eq!(f.hits.iter().map(|h| h.line).collect::<Vec<_>>(), [2, 4]);
    assert_eq!(f.hits[0].text, "    let needle = 1;");
    let o = f.outline.as_ref().unwrap();
    assert_eq!(o.enclosing(2).unwrap().qual, "alpha");
    assert!(o.enclosing(4).is_none());
}

#[test]
fn ignore_case_and_multiple_patterns() {
    let d = project();
    let mut o = opts(&d, &["upper", "alpha"]);
    o.ignore_case = true;
    let r = find(&o).unwrap();
    assert_eq!(r.len(), 2);
    assert!(r[0].path.ends_with("a.rs") && r[1].path.ends_with("b.txt"));
}

#[test]
fn fixed_strings_escape_regex() {
    let d = project();
    fs::write(d.path().join("c.rs"), "a.b()\naxb()\n").unwrap();
    let mut o = opts(&d, &["a.b("]);
    o.fixed = true;
    let r = find(&o).unwrap();
    assert_eq!(r.len(), 1);
    assert_eq!(r[0].hits.len(), 1);
}

#[test]
fn invalid_regex_is_an_error() {
    let d = project();
    assert!(find(&opts(&d, &["("])).is_err());
}
