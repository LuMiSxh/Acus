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
fn find_accepts_recursive_grep_habits_and_explains_the_rest() {
    for flags in [["-rn"], ["-rnH"], ["-R"], ["-nH"]] {
        let (code, _, err) = acus(&["find", "needle", flags[0]]);
        assert_eq!(code, 0, "{flags:?}: {err}");
    }
    for (flag, near) in [
        ("-x", "'^PAT$'"),
        ("-L", "acus find -l PAT"),
        ("--bogus", "that flag does not exist"),
    ] {
        let (code, out, err) = acus(&["find", "needle", flag]);
        assert_eq!((code, out.as_str()), (2, ""), "{flag}");
        assert!(err.contains("unexpected argument"), "{flag}: {err}");
        assert!(
            err.contains("hint: ") && err.contains(near),
            "{flag}: {err}"
        );
        assert!(err.contains("`acus find --help`"), "{flag}: {err}");
    }
    // Other commands keep clap's plain error, and `run` carries the hint into its entry.
    let (_, _, err) = acus(&["outline", "src", "-c"]);
    assert!(!err.contains("hint:"), "{err}");
    let (_, out, _) = acus_in(
        std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/proj")),
        &["run"],
        "[[\"find\", \"needle\", \"-x\"]]",
    );
    assert!(out.contains("hint: anchor the pattern"), "{out}");
}

#[test]
fn find_count_lists_files_and_total() {
    let (code, out, _) = acus(&["find", "needle", "-c"]);
    assert_eq!(code, 0);
    assert_eq!(
        out,
        "README.md 1\nsrc/lib.rs 3\nweb/App.svelte 1\ntotal 5 in 3 files\n"
    );
    assert_eq!(
        acus(&["find", "needle", "--count", "-m", "1", "-g", "*.rs"]).1,
        "src/lib.rs 1\ntotal 1 in 1 files\n"
    );
    let (_, json, _) = acus(&["find", "needle", "-c", "--json"]);
    let v: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(v["total"], 5);
    assert_eq!(v["files"][1]["path"], "src/lib.rs");
    assert_eq!(v["files"][1]["count"], 3);
    assert_eq!(acus(&["find", "zzz-none", "-c"]).0, 1);
}

#[test]
fn find_only_matching_prints_the_matched_text() {
    let (code, out, _) = acus(&["find", "-o", r"need\w+", "-g", "*.rs"]);
    assert_eq!(code, 0);
    assert!(out.contains("11\tneedle\n12\tneedle\n"), "{out}");
    assert!(!out.contains("let needle"), "{out}");
    let (_, json, _) = acus(&["find", "-o", "need", "--json", "-g", "*.rs"]);
    let v: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(v["files"][0]["hits"][0]["matches"][0], "need");
    assert!(v["files"][0]["hits"][0].get("text").is_none());
}

#[test]
fn find_invert_and_max_count_work_per_file() {
    let (code, out, _) = acus(&["find", "-v", "needle", "README.md"]);
    assert_eq!(code, 0);
    assert!(
        out.contains("Demo") && !out.contains("Run the needle"),
        "{out}"
    );
    let (_, out, _) = acus(&["find", "-m", "1", "needle", "src/lib.rs"]);
    assert_eq!(out.matches("needle").count(), 1, "{out}");
    assert_eq!(acus(&["find", "needle", "-m", "0"]).0, 2);
}

#[test]
fn find_grep_glob_flags_map_to_globs() {
    let rs = acus(&["find", "needle", "-l", "-g", "*.rs"]).1;
    assert_eq!(acus(&["find", "needle", "-l", "--include=*.rs"]).1, rs);
    assert_eq!(acus(&["find", "needle", "-l", "--include", "*.rs"]).1, rs);
    let (_, out, _) = acus(&[
        "find",
        "needle",
        "-l",
        "--exclude",
        "*.rs",
        "--exclude-dir",
        "web",
    ]);
    assert_eq!(out, "README.md 1\n");
    assert_eq!(
        acus(&["find", "needle", "-l", "-g", "!web/", "-g", "!src/"]).1,
        "README.md 1\n"
    );
}

#[test]
fn find_accepts_extended_and_perl_flags() {
    let plain = acus(&["find", "need(le|s)"]).1;
    assert_eq!(acus(&["find", "-E", "need(le|s)"]).1, plain);
    assert_eq!(acus(&["find", "-P", "need(le|s)"]).1, plain);
    let (code, _, err) = acus(&["find", "-P", r"(?<=let )needle"]);
    assert_eq!(code, 2);
    assert!(
        err.contains("-P is accepted") && err.contains("no look-around"),
        "{err}"
    );
}

#[test]
fn find_reads_bre_alternation() {
    let (code, out, err) = acus(&["find", r"Usage\|Parser", "-l"]);
    assert_eq!(code, 0);
    assert_eq!(out, acus(&["find", "Usage|Parser", "-l"]).1);
    assert!(err.contains("note: read `\\|` as alternation"), "{err}");
    // ERE users keep their literal pipe; so do -E, -F and classes.
    assert!(!acus(&["find", r"a|b\|c", "-l"]).2.contains("note:"));
    assert!(
        !acus(&["find", "-E", r"Usage\|Parser", "-l"])
            .2
            .contains("note:")
    );
    assert_eq!(acus(&["find", "-F", r"Usage\|Parser"]).0, 1);
    assert_eq!(acus(&["find", r"[\|]x", "-l"]).0, 1);
}

#[test]
fn grep_flags_work_inside_run() {
    let (_, out, _) = acus_in(
        std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/proj")),
        &["run"],
        "find needle -c --include *.rs\nfind -o need\\w+ -g *.rs\nshow src/lib.rs:10,11\n",
    );
    assert!(out.contains("src/lib.rs 3\ntotal 3 in 1 files"), "{out}");
    assert!(out.contains("11\tneedle"), "{out}");
    assert!(out.contains("10\t") && !out.contains("error"), "{out}");
}

#[test]
fn patch_help_documents_replace_all() {
    let (code, out, _) = acus(&["patch", "--help"]);
    assert_eq!(code, 0);
    insta::assert_snapshot!(&out[out.find("PATCH FORMAT").unwrap()..]);
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
fn find_json_no_matches_is_null() {
    let (code, out, err) = acus(&["find", "zzz_not_here", "--json"]);
    assert_eq!(code, 1);
    assert!(err.is_empty());
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&out).unwrap(),
        serde_json::Value::Null
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
    acus_in_env(dir, args, stdin, &[])
}

fn acus_in_env(
    dir: &std::path::Path,
    args: &[&str],
    stdin: &str,
    env: &[(&str, &str)],
) -> (i32, String, String) {
    use std::io::Write;
    let mut child = bin()
        .args(args)
        .envs(env.iter().copied())
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
fn patch_reports_tolerant_matches_and_new_directives() {
    let d = tempfile::tempdir().unwrap();
    std::fs::write(d.path().join("a.py"), "def f():\n\treturn 1  \n").unwrap();
    let p = "*** Update File: a.py\n def f():\n-    return 1\n+    return 2\n*** Copy File: a.py\n*** To: b.py\n*** Insert After: b.py:1\n+# top\n";
    let (code, out, err) = acus_in(d.path(), &["patch", "-q"], p);
    assert_eq!((code, err.as_str()), (0, ""), "{out}");
    assert_eq!(
        out,
        "M a.py +1 -1\nA b.py +2\nM b.py +1 -0\nnote: tolerant match (whitespace/indentation) in a.py hunk 1\n"
    );
    assert_eq!(
        std::fs::read_to_string(d.path().join("b.py")).unwrap(),
        "def f():\n# top\n\treturn 2\n"
    );
    let (code, _, err) = acus_in(
        d.path(),
        &["patch"],
        "*** Update File: a.py\n-nothing like\n",
    );
    assert_eq!(code, 2);
    assert!(err.contains("hunk 1: context not found"), "{err}");
}

#[test]
fn tilde_paths_expand_to_home() {
    let home = tempfile::tempdir().unwrap();
    let h = home.path();
    std::fs::write(h.join("w.rs"), "fn worker() {}\nfn other() {}\n").unwrap();
    let env = [("HOME", h.to_str().unwrap())];
    // Command-line arguments: a symbol and a range, with the address suffix kept.
    let (code, out, err) = acus_env(&["show", "~/w.rs#worker", "~/w.rs:2-2"], &env);
    assert_eq!(code, 0, "{err}");
    assert!(
        out.contains("fn worker() {}") && out.contains("fn other() {}"),
        "{out}"
    );
    // Lines of `acus run` get no shell either.
    let (code, out, err) = acus_in_env(h, &["run"], "show ~/w.rs:1-1\n", &env);
    assert_eq!(code, 0, "{err}");
    assert!(out.contains("fn worker() {}"), "{out}");
    // Paths named inside patch text, and the patch file itself.
    std::fs::write(
        h.join("p.patch"),
        "*** Begin Patch\n*** Update File: ~/w.rs\n-fn other() {}\n+fn other() { 1 }\n*** End Patch\n",
    )
    .unwrap();
    let (code, _, err) = acus_in_env(h, &["patch", "-q", "--file", "~/p.patch"], "", &env);
    assert_eq!(code, 0, "{err}");
    assert_eq!(
        std::fs::read_to_string(h.join("w.rs")).unwrap(),
        "fn worker() {}\nfn other() { 1 }\n"
    );
}

#[cfg(unix)]
#[test]
fn closed_stdout_ends_quietly() {
    use std::io::Read;
    let d = tempfile::tempdir().unwrap();
    let input = d.path().join("in.txt");
    std::fs::write(&input, "show src/lib.rs\n".repeat(5_000)).unwrap();
    let mut child = bin()
        .arg("run")
        .stdin(std::fs::File::open(&input).unwrap())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .current_dir(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/proj"))
        .spawn()
        .unwrap();
    // Read a little, then close the pipe as `| head -1` does while output is still pending.
    let mut first = [0; 16];
    child.stdout.take().unwrap().read_exact(&mut first).unwrap();
    let status = child.wait().unwrap();
    let mut err = String::new();
    child
        .stderr
        .take()
        .unwrap()
        .read_to_string(&mut err)
        .unwrap();
    assert_ne!(status.code(), Some(101), "{err}");
    assert!(
        !err.contains("Broken pipe") && !err.contains("panicked"),
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
    let out = guard(serde_json::json!({
        "tool_name": "Bash",
        "tool_input": {"command": "cd app && ls; cat settings.json; grep -n foo src/a.rs | head"},
    }));
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    let o = &v["hookSpecificOutput"];
    assert_eq!(o["permissionDecision"], "allow", "{out}");
    assert_eq!(
        o["updatedInput"]["command"],
        "cd app && ls; cat settings.json; acus find 'foo' 'src/a.rs' | head",
        "{out}"
    );
    assert!(o.get("permissionDecisionReason").is_none(), "{out}");
    assert!(
        o["additionalContext"]
            .as_str()
            .unwrap()
            .contains("instead of `grep -n foo src/a.rs`"),
        "{out}"
    );
    // Other tool_input fields survive; a build pipe is rewritten in every mode, with no decision.
    let build = serde_json::json!({
        "tool_name": "Bash",
        "permission_mode": "default",
        "tool_input": {"command": "cargo test 2>&1 | tail -3", "timeout": 5000},
    });
    let v: serde_json::Value = serde_json::from_str(&guard(build)).unwrap();
    let o = &v["hookSpecificOutput"];
    assert!(o.get("permissionDecision").is_none(), "{v}");
    assert_eq!(
        o["updatedInput"]["command"],
        "acus ctx 'cargo test' --tail 3"
    );
    assert_eq!(o["updatedInput"]["timeout"], 5000);
    // In-place edits have no exact rewrite.
    let out = guard(serde_json::json!({
        "tool_name": "Bash",
        "tool_input": {"command": "cd app && cat a.rs; sed -i 's/a/b/' b.rs"},
    }));
    assert!(out.contains(r#""permissionDecision":"deny""#), "{out}");
    assert!(out.contains("Nothing ran"), "{out}");
}

#[test]
fn guard_never_fails_on_malformed_input() {
    for input in [
        "",
        "not json",
        "[]",
        "null",
        r#"{"tool_name": "Bash"}"#,
        r#"{"tool_name": "Bash", "tool_input": null}"#,
        r#"{"tool_name": "Bash", "tool_input": {"command": 5}}"#,
        r#"{"tool_name": "Bash", "tool_input": {"command": "grep -e '"}}"#,
        r#"{"tool_name": "PowerShell", "tool_input": {"command": "findstr /é é"}}"#,
        r#"{"tool_name": "Read", "tool_input": {"file_path": 7}}"#,
        r#"{"tool_name": "Grep", "tool_input": {"pattern": 7}}"#,
        r#"{"tool_name": 3}"#,
    ] {
        let (code, out, _) = acus_in(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")),
            &["guard"],
            input,
        );
        // Exit 0 with nothing to say, or with a verdict for what could be read.
        assert_eq!(code, 0, "{input}");
        assert!(
            out.is_empty() || out.starts_with(r#"{"hookSpecificOutput""#),
            "{input}: {out}"
        );
    }
}

#[cfg(feature = "cmd-decide")]
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
    let (code, out, err) = acus_in(dir, &["run", "--json"], r#"[["find","zzz_none"]]"#);
    assert_eq!((code, out.as_str(), err.as_str()), (1, "null\n", ""));
}

#[cfg(unix)]
#[test]
fn ctx_tail_and_grep_keep_failures_and_their_code() {
    let d = tempfile::tempdir().unwrap();
    std::fs::write(d.path().join("a.rs"), "fn f() {\n    boom()\n}\n").unwrap();
    let cmd =
        "echo l1; echo l2; echo 'error: boom at a.rs:2:5'; echo; echo l3; echo l4; echo l5; exit 3";
    let (code, out, _) = acus_in(d.path(), &["ctx", cmd, "--tail", "2"], "");
    assert_eq!(code, 3);
    assert!(
        out.starts_with("… 2 lines dropped (--tail 2)\nerror: boom at a.rs:2:5\n… 1 line dropped (--tail 2)\nl4\nl5\n"),
        "{out}"
    );
    assert!(out.contains("== a.rs#f (line 2)\n"), "{out}");
    assert!(out.contains("… full output (7 lines)"), "{out}");

    let (code, out, _) = acus_in(d.path(), &["ctx", cmd, "--grep", "^l[34]$"], "");
    assert_eq!(code, 3);
    assert!(
        out.starts_with("l3\nl4\n… 5 lines not matching --grep dropped\n"),
        "{out}"
    );
    assert!(!out.contains("a.rs#f"), "{out}");

    let (_, out, _) = acus_in(d.path(), &["ctx", cmd, "--grep", "^error"], "");
    assert!(out.contains("== a.rs#f (line 2)\n"), "{out}");

    let (code, _, err) = acus_in(d.path(), &["ctx", cmd, "--grep", "("], "");
    assert_eq!(code, 2);
    assert!(err.contains("invalid --grep regex"), "{err}");
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
    assert_eq!(
        acus_in(d.path(), &["diff"], ""),
        (0, String::new(), "(no changes)\n".into())
    );
    let (code, out, err) = acus_in(d.path(), &["diff", "--json"], "");
    assert_eq!(code, 0);
    assert!(err.is_empty());
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&out).unwrap(),
        serde_json::Value::Null
    );
    let (code, out, err) = acus_in(d.path(), &["run", "--json"], r#"[["diff"]]"#);
    assert_eq!((code, out.as_str(), err.as_str()), (0, "null\n", ""));
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

#[test]
fn guard_rewrite_quotes_every_copied_argument() {
    use std::io::Write;
    let run = |command: &str| {
        let mut child = bin()
            .arg("guard")
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        let input = serde_json::json!({"tool_name": "Bash", "tool_input": {"command": command}});
        write!(child.stdin.take().unwrap(), "{input}").unwrap();
        let out = child.wait_with_output().unwrap();
        assert_eq!(out.status.code(), Some(0));
        let out = String::from_utf8(out.stdout).unwrap();
        serde_json::from_str::<serde_json::Value>(&out)
            .ok()
            .and_then(|v| {
                v["hookSpecificOutput"]["updatedInput"]["command"]
                    .as_str()
                    .map(str::to_owned)
            })
    };
    for (input, want) in [
        (
            "cat 'a;touch pwn;.rs' b.rs",
            "acus show 'a;touch pwn;.rs' 'b.rs'",
        ),
        (
            "rg -t 'py;touch pwn' foo src",
            "acus find 'foo' 'src' -t 'py;touch pwn'",
        ),
        ("cat 'my file.rs' b.rs", "acus show 'my file.rs' 'b.rs'"),
        ("cat 'a&b.rs' b.rs", "acus show 'a&b.rs' 'b.rs'"),
    ] {
        assert_eq!(run(input).as_deref(), Some(want), "{input}");
    }
}
