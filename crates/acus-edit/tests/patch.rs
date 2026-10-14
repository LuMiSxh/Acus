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
fn partial_line_errors_name_replace_all() {
    let d = dir(&[("a.rs", "fn a() {\n    call(x, y);\n}\n")]);
    let err = |p: &str| {
        run(
            d.path(),
            &format!("*** Begin Patch\n*** Update File: a.rs\n{p}*** End Patch\n"),
        )
        .unwrap_err()
        .to_string()
    };
    // The first `-` line is part of a line.
    // (A matching `+` count would be replaced inside the line, so add a line.)
    let e = err("-call(x\n+call(z\n+more\n");
    assert!(e.contains("line 2 only starts with it"), "{e}");
    assert!(e.contains("`*** Replace All: PATH`"), "{e}");
    assert!(e.contains("`<<<<<<< REGEX`"), "{e}");
    // A later line is part of a line, with the first one matching whole.
    let e = err(" fn a() {\n-    call(x\n+    call(z\n");
    assert!(e.contains("lines must match whole"), "{e}");
    assert!(e.contains("`*** Replace All: PATH`"), "{e}");
    // Nothing similar at all.
    let e = err("-nothing like it\n");
    assert!(e.contains("must match whole lines"), "{e}");
    assert!(e.contains("`*** Replace All: PATH`"), "{e}");
}

#[test]
fn unknown_directives_list_the_accepted_forms() {
    let e = |p: &str| parse(p).unwrap_err().to_string();
    let m = e("*** Replace all: src\n");
    assert!(
        m.contains("did you mean `*** Replace All: PATH [PATH...]`"),
        "{m}"
    );
    assert!(
        m.contains("*** Replace All: PATH [PATH...] (files, directories or globs"),
        "{m}"
    );
    assert!(m.contains("`<<<<<<< SEARCH` or `<<<<<<< REGEX`"), "{m}");
    assert!(m.contains("*** Update File: PATH"), "{m}");
    for form in [
        "*** Copy File: PATH (then *** To: PATH)",
        "*** Insert Before: PATH#Symbol or PATH:LINE",
        "*** Insert After: PATH#Symbol or PATH:LINE",
    ] {
        assert!(m.contains(form), "{m}");
    }
    assert!(e("*** insert after: a.rs#f\n").contains("did you mean `*** Insert After:"));
    assert!(e("*** copy file: a.rs\n").contains("did you mean `*** Copy File: PATH`"));
    assert!(e("*** Replace All:src\n").contains("did you mean"));
    assert!(e("*** Replace All:\n").contains("did you mean"));
    assert!(e("*** update file: a.rs\n").contains("did you mean `*** Update File: PATH`"));
    let m = e("*** Bogus: x\n");
    assert!(
        !m.contains("did you mean") && m.contains("accepted: *** Add File: PATH"),
        "{m}"
    );
    // A REGEX block only exists under Replace All.
    let m = e("*** Update File: a.rs\n<<<<<<< REGEX\nx\n=======\ny\n>>>>>>> REPLACE\n");
    assert!(
        m.contains("REGEX blocks belong under `*** Replace All: PATH`"),
        "{m}"
    );
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
    let a = apply(
        &parse("*** Update File: a.py\n def f():\n-  return 1\n+  return 2\n").unwrap(),
        d.path(),
        false,
    )
    .unwrap();
    assert_eq!(read(d.path(), "a.py"), "def f():\n    return 2\n");
    assert_eq!(a.tolerant, ["a.py hunk 1"]);
}

#[test]
fn non_adjacent_lines_apply_as_separate_pieces() {
    let src = "a\n/// doc `x`\nconst Q: u8 = 1;\nconst O: u8 = 2;\nconst A: &str = \"c\";\nz\n";
    let d = dir(&[("a.rs", src)]);
    let p = "*** Update File: a.rs\n-/// doc `x`\n+/// doc `y`\n-const A: &str = \"c\";\n+const A: &str = \"d\";\n";
    let a = apply(&parse(p).unwrap(), d.path(), false).unwrap();
    assert_eq!(
        read(d.path(), "a.rs"),
        "a\n/// doc `y`\nconst Q: u8 = 1;\nconst O: u8 = 2;\nconst A: &str = \"d\";\nz\n"
    );
    assert_eq!(a.split, ["a.rs hunk 1"]);
    assert!(a.tolerant.is_empty());
}

#[test]
fn split_keeps_context_with_its_piece() {
    let d = dir(&[("a.txt", "one\ntwo\nx\ny\nthree\nfour\n")]);
    let p = "*** Update File: a.txt\n one\n-two\n+2\n-three\n+3\n four\n";
    let a = apply(&parse(p).unwrap(), d.path(), false).unwrap();
    assert_eq!(read(d.path(), "a.txt"), "one\n2\nx\ny\n3\nfour\n");
    assert_eq!(a.split.len(), 1);
}

#[test]
fn split_fails_when_a_piece_is_missing_ambiguous_or_out_of_order() {
    let before = "a\nb\nx\nc\nb\n";
    for (src, p, want) in [
        // Second piece missing.
        ("a\nx\nc\n", "-a\n+A\n-nope\n+N\n", "context not found"),
        // First piece ambiguous (b is on lines 2 and 5): the original error stays.
        (before, "-b\n+B\n-c\n+C\n", "context not found"),
        // Pieces in the wrong order: the second must come after the first.
        ("c\nx\na\n", "-a\n+A\n-c\n+C\n", "context not found"),
    ] {
        let d = dir(&[("a.txt", src)]);
        let patch = format!("*** Update File: a.txt\n{p}");
        let e = run(d.path(), &patch).unwrap_err().to_string();
        assert!(e.contains(want), "{e}");
        assert_eq!(read(d.path(), "a.txt"), src);
    }
}

#[test]
fn partial_lines_are_replaced_when_unique() {
    let d = dir(&[(
        "r.md",
        "# T\n**[Guide](crates/c/README.md)** · [Changelog](crates/c/CHANGELOG.md)\nend\n",
    )]);
    let p =
        "*** Update File: r.md\n-[Changelog](crates/c/CHANGELOG.md)\n+[Changelog](CHANGELOG.md)\n";
    let a = apply(&parse(p).unwrap(), d.path(), false).unwrap();
    assert_eq!(
        read(d.path(), "r.md"),
        "# T\n**[Guide](crates/c/README.md)** · [Changelog](CHANGELOG.md)\nend\n"
    );
    assert_eq!(a.partial, ["r.md hunk 1"]);
}

#[test]
fn partial_lines_respect_anchors_and_refuse_ambiguity() {
    let src = "[x] foo\nfn a\n[y] foo\n";
    let d = dir(&[("a.txt", src)]);
    let ambiguous = "*** Update File: a.txt\n-foo\n+bar\n";
    let e = run(d.path(), ambiguous).unwrap_err().to_string();
    assert!(e.contains("context not found"), "{e}");
    assert_eq!(read(d.path(), "a.txt"), src);
    // The anchor scopes the search to the lines after it.
    run(d.path(), "*** Update File: a.txt\n@@ fn a\n-foo\n+bar\n").unwrap();
    assert_eq!(read(d.path(), "a.txt"), "[x] foo\nfn a\n[y] bar\n");
    // Twice in one line, a missing text and context lines are not partial replacements.
    for (src, p) in [
        ("foo foo\n", "-foo\n+bar\n"),
        ("foo\nbaz\n", "-nope\n+bar\n"),
        ("a foo\nb\n", " b\n-foo\n+bar\n"),
        ("a foo\n", "-foo\n"),
    ] {
        let d = dir(&[("a.txt", src)]);
        let patch = format!("*** Update File: a.txt\n{p}");
        assert!(run(d.path(), &patch).is_err(), "{p}");
        assert_eq!(read(d.path(), "a.txt"), src);
    }
}

#[test]
fn partial_lines_are_all_or_nothing_across_pairs() {
    let d = dir(&[("a.txt", "one two\nthree\n")]);
    let p = "*** Update File: a.txt\n-one\n-nope\n+1\n+2\n";
    assert!(run(d.path(), p).is_err());
    assert_eq!(read(d.path(), "a.txt"), "one two\nthree\n");
}

#[test]
fn tolerant_match_reindents_added_lines_and_keeps_file_context() {
    let d = dir(&[(
        "a.rs",
        "impl A {\n\tfn f() {\n\t\tone();\n\t\ttwo();\n\t}\n}\n",
    )]);
    // Spaces in the patch, tabs in the file; context keeps the file's own lines.
    run(
        d.path(),
        "*** Update File: a.rs\n     fn f() {\n         one();\n+        mid();\n+            deep();\n         two();\n",
    )
    .unwrap();
    assert_eq!(
        read(d.path(), "a.rs"),
        "impl A {\n\tfn f() {\n\t\tone();\n\t\tmid();\n\t\t\tdeep();\n\t\ttwo();\n\t}\n}\n"
    );
    // Same kind of indent, patch at column 0 while the file is nested: add the offset.
    let d = dir(&[("b.py", "class A:\n    def f(self):\n        return 1\n")]);
    run(
        d.path(),
        "*** Update File: b.py\n-def f(self):\n-    return 1\n+def f(self):\n+    return 2\n+\n+def g(self):\n+    pass\n",
    )
    .unwrap();
    assert_eq!(
        read(d.path(), "b.py"),
        "class A:\n    def f(self):\n        return 2\n\n    def g(self):\n        pass\n"
    );
}

#[test]
fn tolerant_match_must_be_unique_and_exact_matches_are_not_reported() {
    let d = dir(&[("a.txt", "  x\n    x\ny\n")]);
    let e = run(d.path(), "*** Update File: a.txt\n-x\n+z\n")
        .unwrap_err()
        .to_string();
    assert!(e.contains("context matches at lines 1, 2"), "{e}");
    let a = apply(
        &parse("*** Update File: a.txt\n-y\n+z\n").unwrap(),
        d.path(),
        true,
    )
    .unwrap();
    assert!(a.tolerant.is_empty());
    // SEARCH blocks are tolerant as well.
    let d = dir(&[("a.rs", "fn f() {\n    a();   \n}\n")]);
    let a = apply(
        &parse("*** Update File: a.rs\n<<<<<<< SEARCH\n  a();\n=======\n  b();\n>>>>>>> REPLACE\n")
            .unwrap(),
        d.path(),
        false,
    )
    .unwrap();
    assert_eq!(read(d.path(), "a.rs"), "fn f() {\n    b();\n}\n");
    assert_eq!(a.tolerant.len(), 1);
}

#[test]
fn context_not_found_names_the_nearest_region() {
    let lines: Vec<String> = (1..=30).map(|i| format!("filler line {i}")).collect();
    let mut text = lines.join("\n");
    text.push_str("\nfn target(a: u8) {\n    let value = compute(a);\n    finish(value);\n}\n");
    let d = dir(&[("src/a.rs", &text)]);
    let e = run(
        d.path(),
        "*** Update File: src/a.rs\n fn target(a: u32) {\n-    let value = compute(a, 1);\n     finish(value);\n+    done();\n",
    )
    .unwrap_err()
    .to_string();
    assert!(e.contains("nearest: src/a.rs:31-33\n31\tfn target(a: u8) {\n32\t    let value = compute(a);\n33\t    finish(value);"), "{e}");
    // Nothing alike: no nearest line.
    let e = run(d.path(), "*** Update File: src/a.rs\n-qqqq zzzz\n+x\n")
        .unwrap_err()
        .to_string();
    assert!(!e.contains("nearest:"), "{e}");
    // Long regions are capped.
    let text: String = (1..=12).map(|i| format!("row number {i}\n")).collect();
    let d = dir(&[("b.txt", &text)]);
    let patch: String = (1..=12).map(|i| format!("-row numbr {i}\n")).collect();
    let e = run(d.path(), &format!("*** Update File: b.txt\n{patch}"))
        .unwrap_err()
        .to_string();
    assert!(
        e.contains("nearest: b.txt:1-12") && e.contains("… 4 more lines"),
        "{e}"
    );
    assert!(!e.contains("\n9\t"), "{e}");
}

#[test]
fn insert_before_and_after_symbols_and_lines() {
    let d = dir(&[(
        "a.rs",
        "fn one() {}\n\n/// Two.\n#[inline]\nfn two() {}\n\nfn three() {}\n",
    )]);
    let c = run(
        d.path(),
        "*** Insert Before: a.rs#two\n+/// New.\n+fn new() {}\n+\n*** Insert After: a.rs#two\n+\n+fn after() {}\n",
    )
    .unwrap();
    assert_eq!(
        read(d.path(), "a.rs"),
        "fn one() {}\n\n/// New.\nfn new() {}\n\n/// Two.\n#[inline]\nfn two() {}\n\nfn after() {}\n\nfn three() {}\n"
    );
    let s: Vec<_> = c.iter().map(ToString::to_string).collect();
    assert_eq!(s, ["M a.rs +3 -0", "M a.rs +2 -0"]);
    run(
        d.path(),
        "*** Insert Before: a.rs:1\n+// top\n*** Insert After: a.rs:13\n+// bottom\n",
    )
    .unwrap();
    let got = read(d.path(), "a.rs");
    assert!(got.starts_with("// top\nfn one() {}\n"), "{got}");
    assert!(got.ends_with("fn three() {}\n// bottom\n"), "{got}");
    let e = run(d.path(), "*** Insert After: a.rs:99\n+x\n")
        .unwrap_err()
        .to_string();
    assert!(e.contains("a.rs:99"), "{e}");
    assert!(parse("*** Insert After: a.rs:0\n+x\n").is_err());
    assert!(parse("*** Insert After: a.rs\n+x\n").is_err());
    assert!(
        parse("*** Insert Before: a.rs#two\n")
            .unwrap_err()
            .to_string()
            .contains("needs `+` lines")
    );
}

#[test]
fn insert_mixes_with_other_ops_all_or_nothing() {
    let d = dir(&[("a.rs", "fn one() {}\n"), ("b.txt", "keep\n")]);
    let e = run(
        d.path(),
        "*** Update File: b.txt\n-keep\n+changed\n*** Insert After: a.rs#missing\n+x\n",
    )
    .unwrap_err();
    assert!(e.to_string().contains("no symbol `missing`"), "{e:#}");
    assert_eq!(read(d.path(), "b.txt"), "keep\n");
}

#[test]
fn copy_file_then_update_the_copy() {
    let d = dir(&[("a.rs", "fn one() {}\r\n"), ("b.txt", "x\n")]);
    let c = run(
        d.path(),
        "*** Copy File: a.rs\n*** To: sub/c.rs\n*** Update File: sub/c.rs\n-fn one() {}\n+fn two() {}\n",
    )
    .unwrap();
    assert_eq!(read(d.path(), "a.rs"), "fn one() {}\r\n");
    assert_eq!(read(d.path(), "sub/c.rs"), "fn two() {}\r\n");
    let s: Vec<_> = c.iter().map(ToString::to_string).collect();
    assert_eq!(s, ["A sub/c.rs +1", "M sub/c.rs +1 -1"]);
    let e = |p: &str| run(d.path(), p).unwrap_err().to_string();
    assert!(e("*** Copy File: a.rs\n*** To: b.txt\n").contains("already exists"));
    assert!(e("*** Copy File: nope.rs\n*** To: n.rs\n").contains("nope.rs"));
    assert!(
        parse("*** Copy File: a.rs\n")
            .unwrap_err()
            .to_string()
            .contains("needs a destination")
    );
    assert!(
        parse("*** To: x\n")
            .unwrap_err()
            .to_string()
            .contains("`*** Copy File:`")
    );
    assert!(!d.path().join("n.rs").exists());
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
fn replace_all_takes_dirs_whole_and_globs_anywhere() {
    let d = dir(&[
        ("web/a.ts", "old\n"),
        ("src/b.rs", "old\n"),
        ("docs/c.md", "old\n"),
        ("docs/d.txt", "old\n"),
    ]);
    run(
        d.path(),
        "*** Replace All: web *.md\n<<<<<<< SEARCH\nold\n=======\nnew\n>>>>>>> REPLACE\n",
    )
    .unwrap();
    let got = ["web/a.ts", "src/b.rs", "docs/c.md", "docs/d.txt"].map(|p| read(d.path(), p));
    assert_eq!(got, ["new\n", "old\n", "new\n", "old\n"]);
}

#[test]
fn symbols_stay_in_their_language() {
    let d = dir(&[("a.rs", "fn one() {}\n"), ("b.py", "x = 1\n")]);
    let e = run(d.path(), "*** Move Symbol: a.rs#one\n*** To: b.py\n").unwrap_err();
    assert!(e.to_string().contains("cannot move rust code"), "{e:#}");
    let e = run(d.path(), "*** Replace Symbol: a.rs#one\n").unwrap_err();
    assert!(e.to_string().contains("Delete Symbol"), "{e:#}");
}

#[test]
fn moving_a_symbol_next_to_itself_is_refused() {
    let d = dir(&[("a.rs", "fn one() {}\n\nfn two() {}\n")]);
    let e = run(d.path(), "*** Move Symbol: a.rs#one\n*** After: a.rs#one\n").unwrap_err();
    assert!(e.to_string().contains("next to itself"), "{e:#}");
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
