use acus_edit::{Change, apply, parse};
use std::fs;
use std::path::Path;

fn dir(files: &[(&str, &str)]) -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    for (p, c) in files {
        let p = d.path().join(p);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, c).unwrap();
    }
    d
}

fn run(root: &Path, patch: &str) -> anyhow::Result<Vec<Change>> {
    Ok(apply(&parse(patch)?, root, false)?.changes)
}

fn read(root: &Path, p: &str) -> String {
    fs::read_to_string(root.join(p)).unwrap()
}

fn no_temp_files(root: &Path) {
    for e in fs::read_dir(root).unwrap() {
        let n = e.unwrap().file_name();
        assert!(!n.to_string_lossy().starts_with(".tmp"), "leftover {n:?}");
    }
}

#[test]
fn update_with_anchor_and_context() {
    let d = dir(&[(
        "a.rs",
        "fn one() {\n    let x = 1;\n}\n\nfn two() {\n    let x = 1;\n}\n",
    )]);
    let c = run(
        d.path(),
        "*** Begin Patch\n*** Update File: a.rs\n@@ fn two\n-    let x = 1;\n+    let x = 2;\n+    let y = 3;\n*** End Patch\n",
    )
    .unwrap();
    assert_eq!(
        read(d.path(), "a.rs"),
        "fn one() {\n    let x = 1;\n}\n\nfn two() {\n    let x = 2;\n    let y = 3;\n}\n"
    );
    assert_eq!(c[0].to_string(), "M a.rs +2 -1");
    no_temp_files(d.path());
}

#[test]
fn anchor_may_be_the_first_context_line_or_part_of_a_line() {
    let d = dir(&[(
        "a.rs",
        "const A: u8 = 1;\nconst B: u8 = 2;\n\npub fn solve() {\n    go(1);\n}\n",
    )]);
    run(
        d.path(),
        "*** Begin Patch\n*** Update File: a.rs\n@@ const B\n const B: u8 = 2;\n+const C: u8 = 3;\n@@ fn solve\n-    go(1);\n+    go(2);\n*** End Patch\n",
    )
    .unwrap();
    assert_eq!(
        read(d.path(), "a.rs"),
        "const A: u8 = 1;\nconst B: u8 = 2;\nconst C: u8 = 3;\n\npub fn solve() {\n    go(2);\n}\n"
    );
}

#[test]
fn ambiguous_context_is_rejected_and_nothing_written() {
    let d = dir(&[("a.rs", "x\ny\nx\n"), ("b.rs", "keep\n")]);
    let err = run(
        d.path(),
        "*** Update File: b.rs\n-keep\n+changed\n*** Update File: a.rs\n-x\n+z\n",
    )
    .unwrap_err()
    .to_string();
    assert!(err.contains("a.rs") && err.contains("lines 1, 3"), "{err}");
    assert_eq!(read(d.path(), "b.rs"), "keep\n", "all or nothing");
    no_temp_files(d.path());
}

#[test]
fn missing_context_names_the_line() {
    let d = dir(&[("a.rs", "a\n")]);
    let err = run(d.path(), "*** Update File: a.rs\n-nope\n+x\n")
        .unwrap_err()
        .to_string();
    assert!(err.contains("a.rs") && err.contains("`nope`"), "{err}");
}

#[test]
fn whitespace_tolerant_match() {
    let d = dir(&[("a.py", "def f():\n    return 1   \n")]);
    run(
        d.path(),
        "*** Update File: a.py\n def f():\n-  return 1\n+    return 2\n",
    )
    .unwrap();
    assert_eq!(read(d.path(), "a.py"), "def f():\n    return 2\n");
}

#[test]
fn add_delete_move() {
    let d = dir(&[("old.txt", "bye\n"), ("m.txt", "a\nb\n")]);
    let c = run(
        d.path(),
        "*** Begin Patch\n*** Add File: new/n.txt\n+hello\n+world\n*** Delete File: old.txt\n*** Update File: m.txt\n*** Move to: moved.txt\n a\n-b\n+c\n*** End Patch",
    )
    .unwrap();
    assert_eq!(read(d.path(), "new/n.txt"), "hello\nworld\n");
    assert!(!d.path().join("old.txt").exists());
    assert!(!d.path().join("m.txt").exists());
    assert_eq!(read(d.path(), "moved.txt"), "a\nc\n");
    let s: Vec<String> = c.iter().map(ToString::to_string).collect();
    assert_eq!(
        s,
        [
            "A new/n.txt +2",
            "D old.txt -1",
            "R m.txt -> moved.txt +1 -1"
        ]
    );
}

#[test]
fn preserves_crlf_and_missing_trailing_newline() {
    let d = dir(&[("w.txt", "a\r\nb\r\nc")]);
    run(d.path(), "*** Update File: w.txt\n-b\n+B\n").unwrap();
    assert_eq!(read(d.path(), "w.txt"), "a\r\nB\r\nc");
}

#[test]
fn pure_insertions_and_end_of_file() {
    let d = dir(&[("a.txt", "a\nb\n")]);
    run(d.path(), "*** Update File: a.txt\n@@ a\n+after a\n").unwrap();
    run(d.path(), "*** Update File: a.txt\n+tail\n*** End of File\n").unwrap();
    assert_eq!(read(d.path(), "a.txt"), "a\nafter a\nb\ntail\n");
}

#[test]
fn replace_symbol() {
    let d = dir(&[(
        "a.rs",
        "struct A;\n\nimpl A {\n    fn f() -> u8 {\n        1\n    }\n}\n",
    )]);
    let c = run(
        d.path(),
        "*** Replace Symbol: a.rs#A::f\n+    fn f() -> u8 {\n+        2\n+    }\n",
    )
    .unwrap();
    assert_eq!(
        read(d.path(), "a.rs"),
        "struct A;\n\nimpl A {\n    fn f() -> u8 {\n        2\n    }\n}\n"
    );
    assert_eq!(c[0].to_string(), "M a.rs +3 -3");
}

#[test]
fn check_mode_writes_nothing() {
    let d = dir(&[("a.txt", "a\n")]);
    let ops = parse("*** Update File: a.txt\n-a\n+b\n").unwrap();
    apply(&ops, d.path(), true).unwrap();
    assert_eq!(read(d.path(), "a.txt"), "a\n");
}

#[test]
fn parse_errors() {
    assert!(parse("*** Frobnicate: x\n").is_err());
    assert!(parse("*** Add File: x\nnot plus\n").is_err());
    assert!(parse("").is_err());
}

#[cfg(unix)]
#[test]
fn keeps_permissions() {
    use std::os::unix::fs::PermissionsExt;
    let d = dir(&[("s.sh", "echo a\n")]);
    let p = d.path().join("s.sh");
    fs::set_permissions(&p, fs::Permissions::from_mode(0o755)).unwrap();
    run(d.path(), "*** Update File: s.sh\n-echo a\n+echo b\n").unwrap();
    assert_eq!(
        fs::metadata(&p).unwrap().permissions().mode() & 0o777,
        0o755
    );
}

#[test]
fn reports_written_regions_with_final_line_numbers() {
    let d = dir(&[("a.txt", "1\n2\n3\n4\n5\n6\n")]);
    // Second hunk sits above the first one; its insertion shifts the first region down.
    let p = "*** Begin Patch\n*** Update File: a.txt\n@@ 4\n-5\n+five\n+FIVE\n*** Update File: a.txt\n@@ 1\n+one-and-a-half\n*** End Patch\n";
    let r = apply(&parse(p).unwrap(), d.path(), true).unwrap().regions;
    let got: Vec<_> = r.iter().map(|r| (r.start, r.lines.join(","))).collect();
    assert_eq!(got, [(2, "one-and-a-half".into()), (6, "five,FIVE".into())]);
    assert_eq!(
        read(d.path(), "a.txt"),
        "1\n2\n3\n4\n5\n6\n",
        "check wrote nothing"
    );
}
