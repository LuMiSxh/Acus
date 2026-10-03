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
fn context_errors_point_at_the_near_miss() {
    let d = dir(&[(
        "a.rs",
        "/// Long doc line, cut short.\nfn a() {\n    x();\n}\n",
    )]);
    let err = |p: &str| {
        run(
            d.path(),
            &format!("*** Begin Patch\n*** Update File: a.rs\n{p}*** End Patch\n"),
        )
        .unwrap_err()
        .to_string()
    };
    assert!(err(" /// Long doc line\n+fn b() {}\n").contains("line 1 only starts with it"));
    assert!(err(" fn a() {\n-    y();\n").contains("line 3 is `    x();`, not `    y();`"));
}

#[test]
fn hunk_may_start_just_above_its_anchor() {
    let d = dir(&[("a.rs", "/// Old.\nfn a() {}\n")]);
    run(
        d.path(),
        "*** Begin Patch\n*** Update File: a.rs\n@@ fn a\n-/// Old.\n+/// New.\n fn a() {}\n*** End Patch\n",
    )
    .unwrap();
    assert_eq!(read(d.path(), "a.rs"), "/// New.\nfn a() {}\n");
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

#[test]
fn search_replace_blocks_inside_update() {
    let d = dir(&[(
        "a.rs",
        "fn one() {\n    a();\n    b();\n}\n\nfn two() {\n    a();\n}\n",
    )]);
    let c = run(
        d.path(),
        "*** Update File: a.rs\n@@ fn two\n<<<<<<< SEARCH\n    a();\n=======\n    z();\n>>>>>>> REPLACE\n<<<<<<<\n    a();\n    b();\n=======\n    b();\n>>>>>>>\n",
    )
    .unwrap();
    assert_eq!(
        read(d.path(), "a.rs"),
        "fn one() {\n    b();\n}\n\nfn two() {\n    z();\n}\n"
    );
    assert_eq!(c[0].to_string(), "M a.rs +1 -2");
}

#[test]
fn replace_all_literal_and_regex_across_files() {
    let d = dir(&[
        ("src/a.rs", "use old::X;\nfn f() { old::y(); }\n"),
        ("src/b.rs", "let v = old::z;\n"),
        ("c.txt", "old::keep\n"),
    ]);
    let c = run(
        d.path(),
        "*** Replace All: src\n<<<<<<< SEARCH\nold::\n=======\nnew::\n>>>>>>> REPLACE\n<<<<<<< REGEX\nfn (\\w+)\\(\\)\n=======\nfn ${1}_v2()\n>>>>>>> REPLACE\n",
    )
    .unwrap();
    assert_eq!(
        read(d.path(), "src/a.rs"),
        "use new::X;\nfn f_v2() { new::y(); }\n"
    );
    assert_eq!(read(d.path(), "src/b.rs"), "let v = new::z;\n");
    assert_eq!(read(d.path(), "c.txt"), "old::keep\n");
    let s: Vec<_> = c.iter().map(ToString::to_string).collect();
    assert_eq!(s, ["M src/a.rs +2 -2", "M src/b.rs +1 -1"]);

    let e = run(
        d.path(),
        "*** Replace All: src/a.rs\n<<<<<<< SEARCH\nmissing\n=======\nx\n>>>>>>> REPLACE\n",
    )
    .unwrap_err()
    .to_string();
    assert!(e.contains("`missing` matches nothing in src/a.rs"), "{e}");
}

#[test]
fn replace_all_multiline_and_globs() {
    let d = dir(&[
        ("a.py", "x = 1\nif a:\n    go()\n\nif a:\n    go()\n"),
        ("b.rs", "if a:\n    go()\n"),
    ]);
    run(
        d.path(),
        "*** Replace All: *.py\n<<<<<<< SEARCH\nif a:\n    go()\n=======\ngo_if(a)\n>>>>>>> REPLACE\n",
    )
    .unwrap();
    assert_eq!(read(d.path(), "a.py"), "x = 1\ngo_if(a)\n\ngo_if(a)\n");
    assert_eq!(read(d.path(), "b.rs"), "if a:\n    go()\n");
}

#[test]
fn delete_symbol_takes_docs_and_one_blank() {
    let d = dir(&[(
        "a.rs",
        "fn one() {}\n\n/// Two.\n#[inline]\nfn two() {\n    x();\n}\n\nfn three() {}\n",
    )]);
    let c = run(d.path(), "*** Delete Symbol: a.rs#two\n").unwrap();
    assert_eq!(read(d.path(), "a.rs"), "fn one() {}\n\nfn three() {}\n");
    assert_eq!(c[0].to_string(), "M a.rs +0 -5");
}

#[test]
fn move_symbol_within_and_across_files() {
    let d = dir(&[
        (
            "a.rs",
            "fn one() {}\n\n/// Two.\nfn two() {}\n\nfn three() {}\n",
        ),
        ("b.rs", "fn b() {}\n"),
    ]);
    run(
        d.path(),
        "*** Move Symbol: a.rs#three\n*** Before: a.rs#one\n",
    )
    .unwrap();
    assert_eq!(
        read(d.path(), "a.rs"),
        "fn three() {}\n\nfn one() {}\n\n/// Two.\nfn two() {}\n"
    );
    let c = run(
        d.path(),
        "*** Move Symbol: a.rs#two\n*** To: b.rs\n*** Move Symbol: a.rs#one\n*** To: c.rs\n",
    )
    .unwrap();
    assert_eq!(read(d.path(), "a.rs"), "fn three() {}\n");
    assert_eq!(
        read(d.path(), "b.rs"),
        "fn b() {}\n\n/// Two.\nfn two() {}\n"
    );
    assert_eq!(read(d.path(), "c.rs"), "fn one() {}\n");
    let s: Vec<_> = c.iter().map(ToString::to_string).collect();
    assert_eq!(
        s,
        ["M a.rs +0 -2", "M b.rs +2 -0", "M a.rs +0 -1", "A c.rs +1"]
    );
    run(d.path(), "*** Move Symbol: b.rs#b\n*** After: b.rs#two\n").unwrap();
    assert_eq!(
        read(d.path(), "b.rs"),
        "/// Two.\nfn two() {}\n\nfn b() {}\n"
    );
    assert!(
        parse("*** Move Symbol: a.rs#x\n")
            .unwrap_err()
            .to_string()
            .contains("needs a destination")
    );
}

#[test]
fn replacement_may_contain_example_blocks() {
    let d = dir(&[("a.md", "# Docs\nTODO\n")]);
    run(
        d.path(),
        "*** Update File: a.md\n<<<<<<< SEARCH\nTODO\n=======\n<<<<<<< SEARCH\nold\n=======\nnew\n>>>>>>> REPLACE\n>>>>>>> REPLACE\n",
    )
    .unwrap();
    assert_eq!(
        read(d.path(), "a.md"),
        "# Docs\n<<<<<<< SEARCH\nold\n=======\nnew\n>>>>>>> REPLACE\n"
    );
}

#[test]
fn moved_symbols_take_the_indent_of_their_new_place() {
    let d = dir(&[(
        "a.py",
        "class A:\n    def m(self):\n        return 1\n\n    def n(self):\n        pass\n\n\ndef top():\n    pass\n",
    )]);
    run(d.path(), "*** Move Symbol: a.py#A.m\n*** After: a.py#top\n").unwrap();
    assert_eq!(
        read(d.path(), "a.py"),
        "class A:\n    def n(self):\n        pass\n\n\ndef top():\n    pass\n\ndef m(self):\n    return 1\n"
    );
    run(d.path(), "*** Move Symbol: a.py#m\n*** Before: a.py#A.n\n").unwrap();
    assert_eq!(
        read(d.path(), "a.py"),
        "class A:\n    def m(self):\n        return 1\n\n    def n(self):\n        pass\n\n\ndef top():\n    pass\n"
    );
}
