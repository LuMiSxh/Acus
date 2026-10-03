use std::process::Command;

fn acus(args: &[&str]) -> (i32, String, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_acus"))
        .args(args)
        .current_dir(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/proj"))
        .output()
        .unwrap();
    (
        out.status.code().unwrap(),
        String::from_utf8(out.stdout).unwrap(),
        String::from_utf8(out.stderr).unwrap(),
    )
}

#[test]
fn find_agent() {
    let (code, out, _) = acus(&["find", "needle"]);
    assert_eq!(code, 0);
    insta::assert_snapshot!(out);
}

#[test]
fn find_block() {
    insta::assert_snapshot!(acus(&["find", "needle", "--block", "-g", "*.rs"]).1);
}

#[test]
fn find_json() {
    insta::assert_snapshot!(acus(&["find", "-e", "greet", "-e", "Usage", "--json"]).1);
}

#[test]
fn find_caps_hits() {
    insta::assert_snapshot!(acus(&["find", "needle", "--max-hits", "1"]).1);
}

#[test]
fn find_nothing_exits_1() {
    assert_eq!(
        acus(&["find", "zzz_not_here"]),
        (1, String::new(), String::new())
    );
}

#[test]
fn bad_regex_exits_2_with_error_line() {
    let (code, out, err) = acus(&["find", "("]);
    assert_eq!((code, out.as_str()), (2, ""));
    assert!(err.starts_with("error: "), "{err}");
}

#[test]
fn outline_file_and_dir() {
    insta::assert_snapshot!("outline_file", acus(&["outline", "src/lib.rs"]).1);
    insta::assert_snapshot!("outline_dir", acus(&["outline", ".", "--depth", "1"]).1);
}

#[test]
fn show_symbol_suffix_range_and_markdown() {
    insta::assert_snapshot!(
        acus(&[
            "show",
            "src/lib.rs#parse",
            "src/lib.rs:1-3",
            "README.md#Usage"
        ])
        .1
    );
}

#[test]
fn show_errors_have_hints() {
    let (code, _, err) = acus(&["show", "src/lib.rs#nope"]);
    assert_eq!(code, 2);
    assert_eq!(
        err,
        "error: no symbol `nope` in src/lib.rs\nhint: acus outline src/lib.rs\n"
    );
    let (code, _, err) = acus(&["show", "src/lib.rs:99"]);
    assert_eq!(
        (code, err.as_str()),
        (2, "error: src/lib.rs has 17 lines\n")
    );
}

#[test]
fn show_caps_whole_files() {
    insta::assert_snapshot!(acus(&["show", "src/lib.rs", "--max-lines", "3"]).1);
}

#[test]
fn show_json() {
    insta::assert_snapshot!(acus(&["show", "src/lib.rs#Parser::new", "--json"]).1);
}

fn acus_in(dir: &std::path::Path, args: &[&str], stdin: &str) -> (i32, String, String) {
    use std::io::Write;
    let mut child = Command::new(env!("CARGO_BIN_EXE_acus"))
        .args(args)
        .current_dir(dir)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(stdin.as_bytes())
        .unwrap();
    let out = child.wait_with_output().unwrap();
    (
        out.status.code().unwrap(),
        String::from_utf8(out.stdout).unwrap(),
        String::from_utf8(out.stderr).unwrap(),
    )
}

#[test]
fn patch_from_stdin() {
    let d = tempfile::tempdir().unwrap();
    std::fs::write(d.path().join("a.rs"), "fn f() {\n    1\n}\n").unwrap();
    let p = "*** Begin Patch\n*** Update File: a.rs\n-    1\n+    2\n*** End Patch\n";
    assert_eq!(
        acus_in(d.path(), &["patch", "--check"], p),
        (
            0,
            "M a.rs +1 -1\n(check only, nothing written)\n".into(),
            String::new()
        )
    );
    assert_eq!(acus_in(d.path(), &["patch"], p).1, "M a.rs +1 -1\n");
    assert_eq!(
        std::fs::read_to_string(d.path().join("a.rs")).unwrap(),
        "fn f() {\n    2\n}\n"
    );
    let (code, _, err) = acus_in(d.path(), &["patch"], p);
    assert_eq!(code, 2);
    assert!(
        err.starts_with("error: a.rs: hunk 1: context not found"),
        "{err}"
    );
}
