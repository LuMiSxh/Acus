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
