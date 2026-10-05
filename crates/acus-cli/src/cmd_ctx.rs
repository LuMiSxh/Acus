use crate::Outcome;
use crate::config::Config;
use crate::fmt::{Format, Out};
use acus_syntax::{Lang, outline};
use acus_walk::display_path;
use anyhow::{Context, Result};
use regex::{Regex, RegexSet};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::LazyLock;

#[derive(clap::Args)]
pub struct Args {
    /// Shell command, e.g. 'cargo test -q'; stdout and stderr are merged. Default: `[ctx] command`,
    /// else the project's test command (Cargo.toml, Package.swift, go.mod, package.json, pytest).
    command: Option<String>,
    /// Output lines kept (head and tail halves).
    #[arg(long, default_value_t = 200)]
    max_output: usize,
    /// Code locations shown.
    #[arg(long, default_value_t = 8)]
    max_refs: usize,
    /// Lines per location.
    #[arg(long, default_value_t = 40)]
    max_lines: usize,
    /// Lines around a location outside any known symbol.
    #[arg(long, default_value_t = 3)]
    context: usize,
    /// Keep build noise (progress lines, passing tests, colour codes, repeated lines).
    #[arg(long)]
    raw: bool,
    /// Also drop output lines matching this regex (repeatable), e.g. --drop '^warning: unused'.
    #[arg(long, value_name = "REGEX")]
    drop: Vec<String>,
}

/// Runs a command, prints its output, then the code its `path:line` references point at.
/// Exit: the command's exit code (2 if it could not start).
pub fn run(a: Args, out: &Out, cfg: &Config) -> Result<Outcome> {
    let extra = RegexSet::new(cfg.ctx.drop.iter().chain(&a.drop)).with_context(|| {
        format!(
            "invalid --drop or [ctx] drop regex ({})",
            cfg.path.display()
        )
    })?;
    let command = match &a.command {
        Some(c) => c.clone(),
        None => {
            let cwd = std::env::current_dir()?;
            let c = cfg
                .ctx
                .command
                .clone()
                .or_else(|| detect_command(&cwd))
                .ok_or_else(|| {
                    anyhow::anyhow!(
                        "no command given and no project detected\nhint: pass a command, e.g. acus ctx 'cargo test -q', or set [ctx] command in {}",
                        cfg.path.display()
                    )
                })?;
            if out.format != Format::Json {
                println!("{}", out.dim(&format!("$ {c}")));
            }
            c
        }
    };
    // The redirect must cover the whole command line, not just its last `&&` part.
    let cmd = if cfg!(windows) {
        format!("({command}) 2>&1")
    } else {
        format!("exec 2>&1\n{command}")
    };
    let res = if cfg!(windows) {
        Command::new("cmd").args(["/C", &cmd]).output()
    } else {
        Command::new("sh").args(["-c", &cmd]).output()
    }
    .with_context(|| format!("cannot run `{command}`"))?;
    let raw = String::from_utf8_lossy(&res.stdout);
    let text = if a.raw { raw.to_string() } else { clean(&raw) };
    let code = res.status.code().unwrap_or(1);
    let cwd = std::env::current_dir()?;
    let refs = refs(&text, &cwd);
    if out.format == Format::Json {
        let locs: Vec<_> = refs
            .iter()
            .take(a.max_refs)
            .filter_map(|(p, l)| snippet(p, *l, &a))
            .map(|s| serde_json::json!({"address": s.0, "start": s.1, "text": s.2.join("\n")}))
            .collect();
        println!(
            "{}",
            serde_json::json!({"exit": code, "output": text, "locations": locs})
        );
        return Ok(Outcome::Exit(code.clamp(0, 255) as u8));
    }
    let all: Vec<&str> = text.lines().collect();
    let (lines, passed, dropped) = if a.raw {
        (all.iter().map(|l| l.to_string()).collect(), 0, 0)
    } else {
        quiet(&all, &extra)
    };
    let half = a.max_output.max(2) / 2;
    let omitted = lines.len().saturating_sub(half * 2);
    // Whatever is not printed stays readable without running the command again.
    // Dropped noise matters only when looking into a failure.
    let log = (omitted > 0 || (dropped > 0 && code != 0))
        .then(|| save_log(&cwd, &command, &text))
        .flatten();
    if omitted > 0 {
        lines[..half].iter().for_each(|l| println!("{l}"));
        println!("… {omitted} output lines omitted");
        lines[lines.len() - half..]
            .iter()
            .for_each(|l| println!("{l}"));
    } else {
        lines.iter().for_each(|l| println!("{l}"));
    }
    if dropped > 0 {
        let tests = if passed > 0 {
            format!("; {passed} tests passed")
        } else {
            String::new()
        };
        println!(
            "{}",
            out.dim(&format!(
                "… {dropped} noise lines dropped{tests} (--raw keeps them)"
            ))
        );
    }
    if let Some(log) = &log {
        let p = log.display();
        let n = text.lines().count();
        println!(
            "{}",
            out.dim(&format!(
                "… full output ({n} lines): acus find 'PAT' {p}, or acus show {p}:A-B"
            ))
        );
    }
    println!("{}", out.header(&format!("== exit {code}")));
    // References into the same snippet print it once, with every referenced line marked.
    let mut shown: Vec<(String, usize, Vec<String>, Vec<usize>)> = Vec::new();
    let mut more = 0;
    for (p, l) in &refs {
        let Some((label, start, body)) = snippet(p, *l, &a) else {
            continue;
        };
        if let Some(s) = shown.iter_mut().find(|s| s.0 == label && s.1 == start) {
            s.3.push(*l);
        } else if shown.len() < a.max_refs {
            shown.push((label, start, body, vec![*l]));
        } else {
            more += 1;
        }
    }
    for (label, start, body, marks) in &shown {
        let ls: Vec<String> = marks.iter().map(ToString::to_string).collect();
        let word = if marks.len() > 1 { "lines" } else { "line" };
        println!(
            "{}",
            out.header(&format!("== {label} ({word} {})", ls.join(", ")))
        );
        for (i, t) in body.iter().enumerate() {
            println!(
                "{}",
                out.numbered(start + i, t, marks.contains(&(start + i)))
            );
        }
    }
    if more > 0 {
        println!("… {more} more locations (--max-refs {})", a.max_refs + more);
    }
    Ok(Outcome::Exit(code.clamp(0, 255) as u8))
}

/// The test command of the nearest project root at or above `start`; one directory is checked
/// in a fixed order, so a Cargo.toml beside a package.json wins.
fn detect_command(start: &Path) -> Option<String> {
    start.ancestors().find_map(|d| {
        let has = |f: &str| d.join(f).exists();
        Some(if has("Cargo.toml") {
            "cargo test -q".into()
        } else if has("Package.swift") {
            "swift test".into()
        } else if has("go.mod") {
            "go test ./...".into()
        } else if has("package.json") {
            let pm = if has("pnpm-lock.yaml") {
                "pnpm"
            } else if has("yarn.lock") {
                "yarn"
            } else if has("bun.lock") || has("bun.lockb") {
                "bun"
            } else {
                "npm"
            };
            format!("{pm} test")
        } else if has("pyproject.toml") || has("pytest.ini") || has("setup.py") {
            "pytest -q".into()
        } else {
            return None;
        })
    })
}

/// Drops progress and success lines that only cost tokens: cargo `Compiling`/`Checking`/…,
/// SwiftPM `[n/m]` steps, passing tests, green `test result` lines (returned as a pass count)
/// and repeated blank lines.
/// Progress, status and passing-test lines of common toolchains. Anchored and specific, so
/// program output and failures stay; `[ctx] drop` in the config adds more.
const NOISE: &[&str] = &[
    // cargo indents its status words; requiring that keeps program output like "Running x".
    r"^\s{2,}(Compiling|Checking|Downloaded|Downloading|Fresh|Blocking|Updating|Locking|Adding|Removing|Finished|Running|Doc-tests|Documenting|Packaging|Verifying|Installing|Installed|Replacing|Replaced) ",
    r"^test .* \.\.\. ok$",
    r"^running \d+ tests?$",
    // Dot progress (cargo -q, pytest -q) without failures.
    r"^[.s]+\s*(\[\s*\d+%\])?$",
    r"^\S+\.py [.s]+\s*(\[\s*\d+%\])?$",
    // SwiftPM, ninja, cmake.
    r"^\[\d+/\d+\] (Compiling|Emitting|Linking|Write|Build|Planning|Applying|Building|Generating|Copying)",
    r"^\[\s*\d+%\] (Building|Linking|Built target|Generating)",
    r"^make(\[\d+\])?: (Entering|Leaving|Nothing to be done)",
    // xcodebuild steps and the command lines under them.
    r"^(CompileC|CompileSwift|CompileSwiftSources|SwiftCompile|SwiftDriver|SwiftDriverJobDiscovery|SwiftEmitModule|SwiftMergeGeneratedHeaders|EmitSwiftModule|Ld|Libtool|ProcessInfoPlistFile|CodeSign|CpResource|CopySwiftLibs|ProcessProductPackaging|ProcessProductPackagingDER|GenerateDSYMFile|Touch|RegisterExecutionPolicyException|RegisterWithLaunchServices|Validate|ValidateEmbeddedBinary|WriteAuxiliaryFile|MkDir|CreateBuildDirectory|PhaseScriptExecution|ExtractAppIntentsMetadata|AppIntentsSSUTraining|ClangStatCache|LinkAssetCatalog|CompileAssetCatalog|CompileAssetCatalogVariant|GenerateAssetSymbols|CopyPlistFile|SymLink|ComputeTargetDependencyGraph|ComputePackagePrebuildTargetDependencyGraph|CreateUniversalBinary|Copy|Ditto|ScanDependencies|PrecompileModule|SwiftExplicitDependencyCompileModuleFromInterface|SwiftExplicitDependencyGeneratePcm|ProcessXCFramework|CompileStoryboard|CompileXIB|LinkStoryboards|GenerateTAPI|Strip|SetMode|SetOwnerAndGroup) ",
    r"^    (cd |export |builtin-|/Applications/Xcode|/usr/bin/|/Library/Developer/)",
    r"^(Prepare packages|Resolve Package Graph|Resolved source packages:|Command line invocation:|Build settings from command line:|User defaults from command line:|Writing result bundle at path:|Computing target dependency graph and provisioning inputs|Create build description|Build description path:|note: Building targets in dependency order|note: Run script build phase|note: Target dependency graph)",
    r"^Test (Case|Suite) '.*' (passed|started)",
    // swift-testing: "✔ Test foo() passed after 0.001 seconds."
    r"^\S{1,2} (Test|Suite) .*(started\.|passed after [\d.]+ seconds\.)$",
    // vitest / jest / mocha / node:test.
    r"^\s*[✓✔√] ",
    r"^ ?PASS ",
    r"^ok \d+ - ",
    // pytest headers and -v passes.
    r"^=+ test session starts =+$",
    r"^(platform \S+ -- Python|rootdir: |plugins: |cachedir: |configfile: |testpaths: |collecting \.\.\.|collected \d+ items?)",
    r"^\S+::\S+ PASSED",
    // go test -v.
    r"^ok  \t",
    r"^=== (RUN|PAUSE|CONT|NAME) ",
    r"^\s*--- PASS: ",
    r"^PASS$",
    // npm, pnpm, yarn, bun.
    r"^> \S+@\S+ \S+",
    r"^(Progress: resolved|Packages: [+-]|Done in [\d.]+m?s|Already up to date|Lockfile is up to date|Scope: all \d+ workspace|Resolving: total|Downloading: total|\$ \S+ \S+ \S+|added \d+ packages?|up to date, audited|found 0 vulnerabilities|\d+ packages? (are|is) looking for funding|  run `npm fund`)",
    r"^[+-]{2,}$",
    // pip, uv, poetry.
    r"^(Requirement already satisfied: |Collecting |  Downloading \S+|  Using cached |Using cached |Installing collected packages: |(Resolved|Prepared|Installed|Audited|Uninstalled|Built) \d+ packages? in )",
    // gradle.
    r"^> Task :\S+( UP-TO-DATE| NO-SOURCE| FROM-CACHE| SKIPPED)?$",
    // svelte-check, mix.
    r"^(Loading svelte-check|Getting Svelte diagnostics|====================================)",
    r"^(==> \S+$|Compiling \d+ files? \(\.\w+\)$|Generated \S+ app$)",
    // Stack frames inside the toolchain or dependencies; frames in your own code stay.
    r"^\s+\d+: +(std|core|alloc|test|rust_begin_unwind|__rust|<unknown>|_?_?pthread|thread_start|_start\b|start_thread|clone3?\b)",
    r"^\s+at (/rustc/|.*(node:internal|node_modules[/\\]|/\.cargo/registry/))",
    r"^note: (run with `RUST_BACKTRACE=1`|Some details are omitted, run with `RUST_BACKTRACE=full`)",
];

/// A Python traceback frame outside the project; the code lines indented below it go too.
static PY_LIB_FRAME: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"^(\s*)File "[^"]*(site-packages|dist-packages|/lib/python\d[\d.]*/|<frozen )"#)
        .unwrap()
});

static NOISE_SET: LazyLock<RegexSet> = LazyLock::new(|| RegexSet::new(NOISE).unwrap());

/// Writes the full output to a temp file named after the directory and command, so a rerun
/// replaces its previous log.
fn save_log(cwd: &Path, command: &str, text: &str) -> Option<PathBuf> {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    (cwd, command).hash(&mut h);
    let dir = std::env::temp_dir().join("acus-ctx");
    std::fs::create_dir_all(&dir).ok()?;
    let path = dir.join(format!("{:016x}.log", h.finish()));
    std::fs::write(&path, text).ok()?;
    Some(path)
}

/// The output as a terminal would show it: no colour codes, and only the final state of lines
/// a progress bar redrew with `\r`.
fn clean(text: &str) -> String {
    static ANSI: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"\x1b(\[[0-9;?]*[ -/]*[@-~]|\][^\x07\x1b]*(\x07|\x1b\\)|[()][0-9A-B])").unwrap()
    });
    ANSI.replace_all(text, "")
        .lines()
        .map(|l| l.rsplit('\r').find(|s| !s.is_empty()).unwrap_or(""))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Drops noise lines, blank runs and repeats; returns kept lines, passed tests and the dropped count.
fn quiet(lines: &[&str], extra: &RegexSet) -> (Vec<String>, usize, usize) {
    let mut passed = 0;
    let mut v: Vec<String> = Vec::new();
    let mut repeat = 0;
    let flush = |v: &mut Vec<String>, repeat: &mut usize| {
        if *repeat > 0 {
            let last = v.last_mut().unwrap();
            *last = format!("{last} (×{})", *repeat + 1);
            *repeat = 0;
        }
    };
    let mut frame: Option<usize> = None;
    let mut noise = |l: &str| {
        let ind = l.len() - l.trim_start().len();
        if let Some(f) = frame {
            if ind > f && !l.trim().is_empty() {
                return true;
            }
            frame = None;
        }
        if let Some(c) = PY_LIB_FRAME.captures(l) {
            frame = Some(c[1].len());
            return true;
        }
        NOISE_SET.is_match(l) || extra.is_match(l)
    };
    for &l in lines.iter().filter(|l| !noise(l)) {
        if let Some(r) = l.trim_start().strip_prefix("test result: ok. ") {
            passed += r
                .split(' ')
                .next()
                .and_then(|n| n.parse::<usize>().ok())
                .unwrap_or(0);
            continue;
        }
        if l.trim().is_empty() && v.last().is_none_or(|p| p.trim().is_empty()) {
            continue;
        }
        if !l.trim().is_empty() && v.last().is_some_and(|p| p == l) {
            repeat += 1;
            continue;
        }
        flush(&mut v, &mut repeat);
        v.push(l.to_string());
    }
    flush(&mut v, &mut repeat);
    let dropped = lines.len() - v.len();
    (v, passed, dropped)
}

/// `path:line` references to files under `cwd`, first occurrence order, one per enclosing
/// region is decided later. Handles `a.rs:3:5`, `a.ts(3,5)` and Python's `File "a.py", line 3`.
fn refs(text: &str, cwd: &Path) -> Vec<(PathBuf, usize)> {
    let mut seen = HashSet::new();
    let mut v = Vec::new();
    for raw in text.lines() {
        for (p, l) in candidates(raw) {
            let path = Path::new(p.trim_start_matches("./"));
            let full = if path.is_absolute() {
                path.to_path_buf()
            } else {
                cwd.join(path)
            };
            // ponytail: only files inside the working directory; library and toolchain paths are noise.
            let Ok(rel) = full.strip_prefix(cwd) else {
                continue;
            };
            if full.is_file() && seen.insert((rel.to_path_buf(), l)) {
                v.push((rel.to_path_buf(), l));
            }
        }
    }
    v
}

fn candidates(line: &str) -> Vec<(&str, usize)> {
    let mut out = Vec::new();
    if let Some(i) = line.find("File \"") {
        let rest = &line[i + 6..];
        if let Some((p, tail)) = rest.split_once("\", line ") {
            let n: String = tail.chars().take_while(char::is_ascii_digit).collect();
            if let Ok(n) = n.parse() {
                out.push((p, n));
            }
        }
    }
    let b = line.as_bytes();
    let path_char = |c: u8| c.is_ascii_alphanumeric() || b"._-/\\".contains(&c);
    let mut i = 0;
    while i < b.len() {
        if b[i] == b':' || b[i] == b'(' {
            let mut s = i;
            while s > 0 && path_char(b[s - 1]) {
                s -= 1;
            }
            // Windows drive letter: `C:\x\a.rs:3`.
            if s >= 2 && b[s - 1] == b':' && b[s - 2].is_ascii_alphabetic() && b[s] == b'\\' {
                s -= 2;
            }
            let p = &line[s..i];
            let digits: String = line[i + 1..]
                .chars()
                .take_while(char::is_ascii_digit)
                .collect();
            let ext = Path::new(p).extension().is_some();
            if ext && !digits.is_empty() && p.contains('.') {
                out.push((p, digits.parse().unwrap_or(0)));
            }
        }
        i += 1;
    }
    out.retain(|(_, n)| *n > 0);
    out
}

/// Enclosing symbol (capped), else a few lines around `line`.
fn snippet(path: &Path, line: usize, a: &Args) -> Option<(String, usize, Vec<String>)> {
    let src = std::fs::read_to_string(path).ok()?;
    let lines: Vec<&str> = src.lines().collect();
    if line > lines.len() {
        return None;
    }
    let shown = display_path(path);
    let sym = Lang::from_path(path)
        .and_then(|l| outline(l, &src))
        .and_then(|o| o.enclosing(line).cloned());
    let (label, start, end) = match sym {
        Some(s) if s.end_line - s.start_line < a.max_lines => {
            (format!("{shown}#{}", s.qual), s.start_line, s.end_line)
        }
        // Too long to print whole: the lines around the reference, still labelled.
        Some(s) => {
            let half = a.max_lines / 2;
            let st = line.saturating_sub(half).max(s.start_line);
            (
                format!("{shown}#{}", s.qual),
                st,
                (st + a.max_lines - 1).min(s.end_line),
            )
        }
        None => (
            shown,
            line.saturating_sub(a.context).max(1),
            (line + a.context).min(lines.len()),
        ),
    };
    let body = lines[start - 1..end]
        .iter()
        .map(|s| (*s).to_owned())
        .collect();
    Some((label, start, body))
}

#[cfg(test)]
mod tests {
    use super::{candidates, clean, detect_command, quiet};
    use regex::RegexSet;

    #[test]
    fn detects_project_command() {
        let t = tempfile::tempdir().unwrap();
        let root = t.path();
        let sub = root.join("a/b");
        std::fs::create_dir_all(&sub).unwrap();
        let touch = |f: &str| std::fs::write(root.join(f), "").unwrap();
        touch("package.json");
        assert_eq!(detect_command(&sub).as_deref(), Some("npm test"));
        touch("yarn.lock");
        assert_eq!(detect_command(&sub).as_deref(), Some("yarn test"));
        touch("pnpm-lock.yaml");
        assert_eq!(detect_command(&sub).as_deref(), Some("pnpm test"));
        touch("pyproject.toml");
        assert_eq!(detect_command(&sub).as_deref(), Some("pnpm test"));
        touch("go.mod");
        assert_eq!(detect_command(&sub).as_deref(), Some("go test ./..."));
        touch("Cargo.toml");
        assert_eq!(detect_command(root).as_deref(), Some("cargo test -q"));
        // The nearest root wins over a farther one.
        std::fs::write(sub.join("pytest.ini"), "").unwrap();
        assert_eq!(detect_command(&sub).as_deref(), Some("pytest -q"));
    }

    #[test]
    fn drops_build_noise_but_keeps_failures() {
        let out = "   Compiling foo v0.1.0 (/x)\n    Finished `test` profile\n     Running unittests src/lib.rs\n\nrunning 2 tests\ntest a ... ok\ntest b ... FAILED\n\n\nfailures:\ntest result: FAILED. 1 passed; 1 failed\n\nrunning 0 tests\ntest result: ok. 0 passed; 0 failed\n[3/9] Compiling Foo Bar.swift\nerror: x\nRunning is a word";
        let l: Vec<&str> = out.lines().collect();
        let (v, passed, dropped) = quiet(&l, &RegexSet::empty());
        assert_eq!((passed, dropped), (0, 10));
        assert_eq!(
            v,
            [
                "test b ... FAILED",
                "",
                "failures:",
                "test result: FAILED. 1 passed; 1 failed",
                "",
                "error: x",
                "Running is a word"
            ]
        );
    }

    #[test]
    fn drops_noise_of_other_toolchains() {
        let out = "\x1b[32m ✓ src/a.test.ts (3 tests)\x1b[0m\n \x1b[31m× src/b.test.ts > fails\x1b[0m\n=== RUN   TestX\n--- PASS: TestX (0.00s)\n--- FAIL: TestY (0.00s)\nok  \tpkg/a\t0.1s\nCompileSwift normal arm64 /x/A.swift\n    cd /x\n/x/A.swift:3:1: error: nope\nTest Case '-[T a]' passed (0.001 seconds).\nTest Case '-[T b]' failed (0.001 seconds).\n============================= test session starts ==============================\nplatform darwin -- Python 3.13.0\ncollected 4 items\n\ntests/test_a.py ..F.  [100%]\ntests/test_b.py ....  [100%]\nwarn: x\nwarn: x\nwarn: x\ncustom noise 1\n> app@1.0.0 build\nProgress: resolved 1, reused 1\rProgress: resolved 9, reused 9\ndone";
        let text = clean(out);
        let l: Vec<&str> = text.lines().collect();
        let (v, _, _) = quiet(&l, &RegexSet::new(["^custom noise"]).unwrap());
        assert_eq!(
            v,
            [
                " × src/b.test.ts > fails",
                "--- FAIL: TestY (0.00s)",
                "/x/A.swift:3:1: error: nope",
                "Test Case '-[T b]' failed (0.001 seconds).",
                "",
                "tests/test_a.py ..F.  [100%]",
                "warn: x (×3)",
                "done"
            ]
        );
    }

    #[test]
    fn drops_library_stack_frames() {
        let out = "thread 'main' panicked at src/main.rs:3:5:\nboom\nstack backtrace:\n   0: rust_begin_unwind\n             at /rustc/abc/library/std/src/panicking.rs:665:5\n   1: app::run\n             at ./src/main.rs:3:5\nnote: Some details are omitted, run with `RUST_BACKTRACE=full` for a verbose backtrace.\nTraceback (most recent call last):\n  File \"/x/app.py\", line 3, in <module>\n    go()\n  File \"/usr/lib/python3.12/site-packages/lib.py\", line 9, in go\n    raise X\n    ^^^^^^^\nX: bad\n    at run (src/a.ts:4:9)\n    at Module._compile (node:internal/modules/cjs/loader:1105:14)";
        let l: Vec<&str> = out.lines().collect();
        let (v, _, _) = quiet(&l, &RegexSet::empty());
        assert_eq!(
            v,
            [
                "thread 'main' panicked at src/main.rs:3:5:",
                "boom",
                "stack backtrace:",
                "   1: app::run",
                "             at ./src/main.rs:3:5",
                "Traceback (most recent call last):",
                "  File \"/x/app.py\", line 3, in <module>",
                "    go()",
                "X: bad",
                "    at run (src/a.ts:4:9)"
            ]
        );
    }

    #[test]
    fn finds_compiler_and_runtime_references() {
        assert_eq!(candidates("  --> src/lib.rs:12:5"), [("src/lib.rs", 12)]);
        assert_eq!(
            candidates("thread 'x' panicked at crates/a/src/b.rs:7:9:"),
            [("crates/a/src/b.rs", 7)]
        );
        assert_eq!(
            candidates("src/app.ts(3,14): error TS2304"),
            [("src/app.ts", 3)]
        );
        assert_eq!(
            candidates("  File \"pkg/mod.py\", line 41, in f"),
            [("pkg/mod.py", 41)]
        );
        assert_eq!(candidates("time: 12:30 ok"), []);
    }
}
