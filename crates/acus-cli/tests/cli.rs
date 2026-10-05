use std::process::Command;

/// The binary with any user config hidden.
fn bin() -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_acus"));
    c.env("ACUS_CONFIG", "/nonexistent/acus.toml")
        .env_remove("ACUS_DISABLE")
        .env_remove("ACUS_DECIDE_URL");
    c
}

fn acus(args: &[&str]) -> (i32, String, String) {
    acus_env(args, &[])
}

fn acus_env(args: &[&str], env: &[(&str, &str)]) -> (i32, String, String) {
    let out = bin()
        .args(args)
        .envs(env.iter().copied())
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
fn grep_context_flags_mean_block() {
    let block = acus(&["find", "needle", "--block", "-g", "*.rs"]).1;
    assert_eq!(acus(&["find", "needle", "-A5", "-g", "*.rs"]).1, block);
    assert_eq!(acus(&["find", "needle", "-C", "3", "-g", "*.rs"]).1, block);
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
fn find_rg_habits() {
    let (code, out, _) = acus(&["find", "needle", "-l", "-n"]);
    assert_eq!(code, 0);
    assert!(out.lines().all(|l| !l.contains('\t')), "{out}");
    assert_eq!(acus(&["find", "-w", "needl"]).0, 1);
    assert_eq!(
        acus(&["find", "needle", "-t", "rust", "-l"])
            .1
            .lines()
            .count(),
        1
    );
}

#[test]
fn find_nothing_exits_1() {
    assert_eq!(
        acus(&["find", "zzz_not_here"]),
        (
            1,
            String::new(),
            "(no matches; hidden and .gitignored files were skipped: --hidden, -u)\n".into()
        )
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
    let mut child = bin()
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
            "M a.rs +1 -1\n== a.rs 2-2\n2\t    2\n(check only, nothing written)\n".into(),
            String::new()
        )
    );
    assert_eq!(acus_in(d.path(), &["patch", "-q"], p).1, "M a.rs +1 -1\n");
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

#[test]
fn commands_can_be_disabled_at_runtime() {
    let (code, _, err) = acus_env(&["find", "needle"], &[("ACUS_DISABLE", "usage, find")]);
    assert_eq!(code, 2);
    assert!(
        err.starts_with("error: acus find is disabled by configuration\nhint: "),
        "{err}"
    );
}

#[test]
fn invalid_config_is_reported() {
    let d = tempfile::tempdir().unwrap();
    let cfg = d.path().join("c.toml");
    std::fs::write(&cfg, "[commands]\ndisabld = []\n").unwrap();
    let (code, _, err) = acus_env(&["find", "x"], &[("ACUS_CONFIG", cfg.to_str().unwrap())]);
    assert_eq!(code, 2);
    assert!(err.contains("invalid config"), "{err}");
}

#[cfg(not(feature = "cmd-decide"))]
#[test]
fn decide_without_feature_has_hint() {
    let (code, _, err) = acus(&["decide", "ok?", "--state", "x"]);
    assert_eq!(code, 2);
    assert!(err.contains("--features cmd-decide"), "{err}");
}

#[cfg(feature = "cmd-decide")]
#[test]
fn decide_without_endpoint_has_hint() {
    let (code, _, err) = acus(&["decide", "ok?", "--state", "x"]);
    assert_eq!(code, 2);
    assert!(err.contains("ACUS_DECIDE_URL"), "{err}");
}

#[cfg(feature = "cmd-decide")]
#[test]
fn ctx_log_is_readable_with_show() {
    // The acus binary itself prints enough lines and exists on every platform.
    let cmd = format!("\"{}\" skill", env!("CARGO_BIN_EXE_acus"));
    let (code, out, err) = acus(&["ctx", &cmd, "--max-output", "4"]);
    assert_eq!(code, 0, "{err}");
    let log = out
        .lines()
        .find_map(|l| l.split_once(", or acus show ")?.1.strip_suffix(":A-B"))
        .unwrap_or_else(|| panic!("no log path in {out}"));
    // On Windows the path starts with a drive letter, a colon the address parser must skip.
    let (code, shown, err) = acus(&["show", &format!("{log}:2-3")]);
    assert_eq!(code, 0, "{err}");
    assert!(shown.contains("2-3"), "{shown}");
    assert!(shown.lines().any(|l| l.starts_with("3\t")), "{shown}");
}

#[test]
fn guard_answers_hook_calls() {
    use std::io::Write;
    let dir = std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/proj"));
    let guard = |input: serde_json::Value| {
        let mut child = bin()
            .arg("guard")
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        write!(child.stdin.take().unwrap(), "{input}").unwrap();
        let out = child.wait_with_output().unwrap();
        assert_eq!(out.status.code(), Some(0));
        String::from_utf8(out.stdout).unwrap()
    };
    let out = guard(serde_json::json!({
        "tool_name": "Grep",
        "cwd": dir,
        "tool_input": {"pattern": "needle", "path": dir.join("src"), "output_mode": "content"},
    }));
    assert!(out.contains(r#""permissionDecision":"deny""#), "{out}");
    assert!(out.contains("acus find 'needle' 'src'"), "{out}");
    let out = guard(serde_json::json!({
        "tool_name": "PowerShell",
        "tool_input": {"command": "Get-Content src\\lib.rs"},
    }));
    assert!(out.contains("acus-skip"), "{out}");
    let out = guard(serde_json::json!({"tool_name": "Bash", "tool_input": {"command": "ls"}}));
    assert_eq!(out, "");
}

#[test]
fn decide_without_key_warns_and_skips() {
    let env = [
        ("ACUS_DECIDE_URL", "http://127.0.0.1:9"),
        ("ACUS_DECIDE_API_KEY_ENV", "ACUS_TEST_UNSET_KEY"),
    ];
    let (code, _, err) = acus_env(&["decide", "ok?", "--state", "x"], &env);
    assert_eq!(code, 2);
    assert!(
        err.contains("warning: no API key in $ACUS_TEST_UNSET_KEY"),
        "{err}"
    );
}

#[test]
fn skill_prints_body_and_find_path_errors_skip_regex_hint() {
    let (code, out, _) = acus(&["skill"]);
    assert_eq!(code, 0);
    assert!(out.starts_with("# acus"), "{out}");
    let (code, _, err) = acus(&["find", "x", "no/such/dir"]);
    assert_eq!(code, 2);
    assert!(
        err.contains("path not found") && !err.contains("hint: escape"),
        "{err}"
    );
}

#[test]
fn run_survives_a_bad_line() {
    let dir = std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/proj"));
    let (code, out, _) = acus_in(dir, &["run"], "find \"open\nshow src/lib.rs:1\n");
    assert_eq!(code, 2);
    assert!(
        out.contains("unclosed") && out.contains("== src/lib.rs 1-1"),
        "{out}"
    );
}

#[test]
fn run_batch() {
    let dir = std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/proj"));
    let batch = r#"[["show","src/lib.rs#new"],["find","zzz_none"],["show","nope.rs"]]"#;
    let (code, out, _) = acus_in(dir, &["run"], batch);
    assert_eq!(code, 2);
    insta::assert_snapshot!(out);
    let (code, out, _) = acus_in(dir, &["run", "--json"], r#"[["show","src/lib.rs:1"]]"#);
    assert_eq!(
        (code, out.as_str()),
        (
            0,
            "[{\"address\":\"src/lib.rs\",\"start\":1,\"end\":1,\"text\":\"pub struct Parser {\"}]\n"
        )
    );
}

#[test]
fn ctx_shows_code_behind_output_references() {
    let d = tempfile::tempdir().unwrap();
    std::fs::write(d.path().join("a.rs"), "fn f() {\n    boom()\n}\n").unwrap();
    let (code, out, _) = acus_in(
        d.path(),
        &["ctx", "echo 'panicked at a.rs:2:5' >&2 && exit 3"],
        "",
    );
    assert_eq!(code, 3);
    assert!(
        out.contains("== exit 3\n== a.rs#f (line 2)\n1\tfn f() {\n2>\t    boom()\n"),
        "{out}"
    );
}

#[test]
fn diff_groups_changes_by_symbol() {
    let d = tempfile::tempdir().unwrap();
    let git = |args: &[&str]| {
        let ok = Command::new("git")
            .args([
                "-c",
                "user.name=t",
                "-c",
                "user.email=t@t",
                "-c",
                "commit.gpgsign=false",
            ])
            .args(args)
            .current_dir(d.path())
            .status()
            .unwrap()
            .success();
        assert!(ok, "git {args:?}");
    };
    std::fs::write(
        d.path().join("a.rs"),
        "fn f() {\n    one()\n}\n\nfn gone() {}\n",
    )
    .unwrap();
    git(&["init", "-q"]);
    git(&["add", "."]);
    git(&["commit", "-qm", "x"]);
    std::fs::write(d.path().join("a.rs"), "fn f() {\n    two()\n}\n").unwrap();
    std::fs::write(d.path().join("new.md"), "# Hi\n").unwrap();
    let (code, out, _) = acus_in(d.path(), &["diff", "-p"], "");
    assert_eq!(code, 0, "{out}");
    assert_eq!(
        out,
        "M a.rs +1 -3\n  fn f 1-3 +1 -1\n\t-    one()\n2\t+    two()\n  (top level) -1\n\t-\n  fn gone (removed) -1\n\t-fn gone() {}\nA new.md +1\n1\t+# Hi\n== 2 files +2 -3\n"
    );
    git(&["add", "."]);
    git(&["commit", "-qm", "y"]);
    assert_eq!(acus_in(d.path(), &["diff"], "").0, 1);
}

#[test]
fn long_lines_and_decorators() {
    let d = tempfile::tempdir().unwrap();
    let min = format!("{}needle{}\n", "a;".repeat(5000), ";b".repeat(5000));
    std::fs::write(d.path().join("min.js"), min).unwrap();
    std::fs::write(
        d.path().join("s.py"),
        "import x\n\n\n@cache\ndef f():\n    pass\n",
    )
    .unwrap();
    let (_, out, _) = acus_in(d.path(), &["find", "needle"], "");
    assert!(
        out.len() < 400 && out.contains("needle") && out.contains("chars)"),
        "{out}"
    );
    let (_, out, _) = acus_in(d.path(), &["find", "-e", "--x", "-e", "needle", "-l"], "");
    assert_eq!(out, "min.js 1\n");
    let (_, out, _) = acus_in(d.path(), &["show", "s.py#f"], "");
    assert_eq!(out, "== s.py#f 4-6\n4\t@cache\n5\tdef f():\n6\t    pass\n");
    let (_, out, _) = acus_in(d.path(), &["run", "--json"], "find zzz\nshow s.py:1\n");
    assert_eq!(out.lines().next(), Some("null"));
}

#[test]
fn unstaged_diff_reads_the_index() {
    let d = tempfile::tempdir().unwrap();
    let git = |args: &[&str]| {
        let ok = Command::new("git")
            .args(["-c", "user.name=t", "-c", "user.email=t@t"])
            .args(args)
            .current_dir(d.path())
            .output()
            .unwrap()
            .status
            .success();
        assert!(ok, "git {args:?}");
    };
    git(&["init", "-q"]);
    std::fs::write(
        d.path().join("a.rs"),
        "fn one() {\n    1;\n}\n\nfn two() {\n    2;\n}\n",
    )
    .unwrap();
    git(&["add", "."]);
    git(&["commit", "-qm", "i"]);
    git(&["mv", "a.rs", "b.rs"]);
    std::fs::write(
        d.path().join("b.rs"),
        "fn one() {\n    1;\n}\n\nfn two() {\n    3;\n}\n",
    )
    .unwrap();
    let (code, out, _) = acus_in(d.path(), &["diff"], "");
    assert_eq!(
        (code, out.as_str()),
        (0, "M b.rs +1 -1\n  fn two 5-7 +1 -1\n== 1 file +1 -1\n")
    );
}
