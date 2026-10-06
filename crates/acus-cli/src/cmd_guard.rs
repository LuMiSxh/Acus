//! PreToolUse hook for Claude Code: refuses shell searches, reads and in-place edits of source
//! files that acus does in one call, builds piped into a filter, and whole reads of large source
//! files, and tells the agent which acus command to use instead.

use crate::Outcome;
use crate::config::Config;
use acus_syntax::{Lang, outline};
use anyhow::Result;
use regex::Regex;
use serde_json::Value;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

#[derive(clap::Args)]
pub struct Args {}

pub fn run(_: Args, cfg: &Config) -> Result<Outcome> {
    let mut input = String::new();
    std::io::stdin().read_to_string(&mut input)?;
    let v: Value = serde_json::from_str(&input).unwrap_or_default();
    let input = &v["tool_input"];
    let cwd = v["cwd"].as_str();
    let command = input["command"].as_str().unwrap_or_default();
    let verdict = match v["tool_name"].as_str() {
        Some("Read") => read_check(input, cwd),
        Some("Grep") => grep_tool(input, cwd),
        Some("Glob") => Some(Verdict::Hint("glob-tool", GLOB_HINT.to_owned())),
        Some("PowerShell") => check_ps(command),
        _ => check(command),
    };
    let Some(verdict) = verdict else {
        return Ok(Outcome::Found);
    };
    log(cfg, &v, &verdict);
    let out = match verdict {
        Verdict::Deny(_, reason) => serde_json::json!({ "hookSpecificOutput": {
            "hookEventName": "PreToolUse",
            "permissionDecision": "deny",
            "permissionDecisionReason": reason,
        }}),
        // The command runs as usual; only the agent's context gains the hint.
        Verdict::Hint(_, hint) => serde_json::json!({ "hookSpecificOutput": {
            "hookEventName": "PreToolUse",
            "additionalContext": hint,
        }}),
        Verdict::Escape(_) => return Ok(Outcome::Found),
    };
    println!("{out}");
    Ok(Outcome::Found)
}

/// Every decision is logged here, for `acus usage --guard`.
pub fn log_path(cfg: &Config) -> PathBuf {
    cfg.path.with_file_name("guard.jsonl")
}

/// Appends one decision as a JSON line; a log that cannot be written is skipped.
fn log(cfg: &Config, hook: &Value, v: &Verdict) {
    let path = log_path(cfg);
    // Start over past 4 MB, keeping one old generation.
    if std::fs::metadata(&path).is_ok_and(|m| m.len() > 4 << 20) {
        let _ = std::fs::rename(&path, path.with_extension("jsonl.old"));
    }
    let input = &hook["tool_input"];
    let call: String = ["command", "file_path", "pattern"]
        .iter()
        .find_map(|k| input[k].as_str())
        .unwrap_or_default()
        .chars()
        .take(300)
        .collect();
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let line = serde_json::json!({
        "ts": ts,
        "session": hook["session_id"],
        "cwd": hook["cwd"],
        "tool": hook["tool_name"],
        "rule": v.rule(),
        "kind": v.kind(),
        "call": call,
    });
    let _ = path.parent().map(std::fs::create_dir_all);
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
    {
        let _ = writeln!(f, "{line}");
    }
}

/// Each verdict names the rule that produced it, for the log.
#[derive(Debug, PartialEq)]
enum Verdict {
    /// Refuse the call; the reason names the acus command to use.
    Deny(&'static str, String),
    /// Run the call, but tell the agent about the acus alternative.
    Hint(&'static str, String),
    /// A call the rule would refuse, run anyway through the escape; only logged.
    Escape(&'static str),
}

impl Verdict {
    fn rule(&self) -> &'static str {
        match self {
            Verdict::Deny(r, _) | Verdict::Hint(r, _) | Verdict::Escape(r) => r,
        }
    }

    fn kind(&self) -> &'static str {
        match self {
            Verdict::Deny(..) => "deny",
            Verdict::Hint(..) => "hint",
            Verdict::Escape(_) => "escape",
        }
    }

    fn escaped(self) -> Verdict {
        Verdict::Escape(self.rule())
    }
}

const ESCAPE: &str = "Only if acus cannot do this, rerun with a `command ` prefix.";
const ESCAPE_PS: &str = "Only if acus cannot do this, rerun with `# acus-skip` at the end.";
const GLOB_HINT: &str = "acus hint: `acus map [DIR]` shows the layout with file and line counts, `acus outline DIR --depth 1` lists a directory's source files with their top-level symbols, and `acus find -l 'PAT'` lists the files containing a text.";
const TREE_HINT: &str = "acus hint: `acus map [DIR]` shows the tree with file and line counts within a token budget, rolls up deep paths and lists the top-level symbols of small directories.";

const CODE: &[&str] = &[
    "rs", "swift", "ts", "tsx", "mts", "cts", "js", "jsx", "mjs", "cjs", "svelte", "vue", "py",
    "pyi", "md", "toml", "json", "yaml", "yml", "go", "c", "h", "cc", "cpp", "hpp", "m", "java",
    "kt", "rb", "sh", "zsh", "css", "scss", "html", "lua", "cs", "sql",
];

/// Source files longer than this are not read whole without a reason.
const READ_MAX: usize = 300;
/// Symbols listed when such a read is refused.
const OUTLINE_MAX: usize = 60;

/// A whole read of a large source file: refused with the file's outline, so the agent can pick
/// symbols or a range instead.
fn read_check(input: &serde_json::Value, cwd: Option<&str>) -> Option<Verdict> {
    if ["offset", "limit", "pages"]
        .iter()
        .any(|k| !input[k].is_null())
    {
        return None;
    }
    let file = input["file_path"].as_str()?;
    let path = Path::new(file);
    // Data files are mostly read whole on purpose; their outline says little.
    let lang =
        Lang::from_path(path).filter(|l| !matches!(l, Lang::Toml | Lang::Json | Lang::Yaml))?;
    let src = std::fs::read_to_string(path).ok()?;
    read_verdict(&relative(file, cwd), lang, &src)
}

/// `path` relative to the session's directory when inside it, else as given.
fn relative(path: &str, cwd: Option<&str>) -> String {
    match cwd.and_then(|c| Path::new(path).strip_prefix(c).ok()) {
        Some(p) if p.as_os_str().is_empty() => ".".to_owned(),
        Some(p) => p.display().to_string(),
        None => path.to_owned(),
    }
}

/// A shell word: single-quoted unless the text holds a single quote.
fn quote(s: &str) -> String {
    if s.contains('\'') {
        let escaped: String = s
            .chars()
            .flat_map(|c| {
                let esc = matches!(c, '"' | '\\' | '$' | '`').then_some('\\');
                esc.into_iter().chain([c])
            })
            .collect();
        format!("\"{escaped}\"")
    } else {
        format!("'{s}'")
    }
}

/// The built-in Grep tool: refused with the equivalent `acus find`. Counts and multiline
/// patterns, which acus does not do, pass.
fn grep_tool(input: &Value, cwd: Option<&str>) -> Option<Verdict> {
    let mode = input["output_mode"]
        .as_str()
        .unwrap_or("files_with_matches");
    if mode == "count" || input["multiline"].as_bool() == Some(true) {
        return None;
    }
    let mut cmd = format!("acus find {}", quote(input["pattern"].as_str()?));
    if let Some(p) = input["path"].as_str().map(|p| relative(p, cwd))
        && p != "."
    {
        cmd += &format!(" {}", quote(&p));
    }
    if let Some(g) = input["glob"].as_str() {
        cmd += &format!(" -g {}", quote(g));
    }
    if let Some(t) = input["type"].as_str() {
        cmd += &format!(" -t {t}");
    }
    if input["-i"].as_bool() == Some(true) {
        cmd += " -i";
    }
    if mode == "files_with_matches" {
        cmd += " -l";
    } else if ["-A", "-B", "-C", "context"]
        .iter()
        .any(|k| !input[k].is_null())
    {
        cmd += " --block";
    }
    Some(Verdict::Deny(
        "grep-tool",
        format!(
            "Use acus instead of the Grep tool: `{cmd}` groups the hits by file and enclosing symbol, and names the follow-up command for anything it cuts. Only if acus cannot do this, run `command rg` via Bash."
        ),
    ))
}

/// PowerShell: the same rules on the lowercased command, as cmdlets ignore case, plus
/// `-replace … | Set-Content` edits.
fn check_ps(cmd: &str) -> Option<Verdict> {
    let c = cmd.to_lowercase();
    let edit = (c.contains("-replace") || c.contains(".replace("))
        && ["set-content", "out-file", "writealltext"]
            .iter()
            .any(|w| c.contains(w))
        && c.split(|ch: char| ch.is_whitespace() || "()'\",;".contains(ch))
            .any(is_code_path);
    let v = if edit {
        Verdict::Deny(
            "ps-replace",
            format!(
                "Use acus instead of -replace with Set-Content: one `acus patch` with `*** Replace All: PATHS` or SEARCH/REPLACE blocks edits every file at once and prints the written lines. {ESCAPE_PS}"
            ),
        )
    } else {
        match check(&c)? {
            Verdict::Deny(r, reason) => Verdict::Deny(r, reason.replace(ESCAPE, ESCAPE_PS)),
            v => v,
        }
    };
    Some(if c.contains("acus-skip") {
        v.escaped()
    } else {
        v
    })
}

fn read_verdict(path: &str, lang: Lang, src: &str) -> Option<Verdict> {
    let lines = src.lines().count();
    if lines <= READ_MAX {
        return None;
    }
    let o = outline(lang, src)?;
    let mut syms: Vec<String> = o
        .symbols
        .iter()
        .filter(|s| s.depth < 2)
        .map(|s| {
            let name = s.detail.as_deref().unwrap_or(&s.name);
            let indent = "  ".repeat(s.depth);
            format!("{indent}{} {name} {}-{}", s.kind, s.start_line, s.end_line)
        })
        .collect();
    if syms.len() > OUTLINE_MAX {
        let more = syms.len() - OUTLINE_MAX;
        syms.truncate(OUTLINE_MAX);
        syms.push(format!("… {more} more (acus outline {path})"));
    }
    Some(Verdict::Deny(
        "read-tool",
        format!(
            "{path} has {lines} lines. Read only what you need: `acus show {path}#Symbol {path}:A-B` reads several symbols and ranges in one call, or Read with offset and limit. Only if you need the whole file, Read it with limit: {lines}.\nOutline of {path}:\n{}",
            syms.join("\n")
        ),
    ))
}

/// A segment run through the `command ` escape, and the segment without it.
fn unescape(seg: &str) -> (bool, &str) {
    match seg.trim_start().strip_prefix("command ") {
        Some(rest) => (true, rest),
        None => (false, seg),
    }
}

/// What to do with `cmd`: refusing wins over hinting, `None` lets it run.
fn check(cmd: &str) -> Option<Verdict> {
    let (rest, docs) = strip_heredocs(cmd);
    let scripts = docs.iter().filter_map(|(opener, body)| {
        let interp = segments(opener).iter().find_map(|(seg, _)| {
            head(seg)
                .map(|(h, _)| h.to_owned())
                .filter(|h| is_interpreter(h))
                .or_else(|| script_writer(seg).map(str::to_owned))
        })?;
        is_edit_script(body).then(|| script_hint(&interp))
    });
    let segs = segments(&rest);
    let mark = |esc: bool, v: Verdict| if esc { v.escaped() } else { v };
    let filtered = segs.windows(2).filter_map(|w| {
        let (esc, seg) = unescape(&w[0].0);
        filtered_build(seg, &w[1]).map(|v| mark(esc, v))
    });
    let v = segs
        .iter()
        .filter_map(|(seg, piped)| {
            let (esc, seg) = unescape(seg);
            rule(seg, *piped).map(|v| mark(esc, v))
        })
        .chain(filtered)
        .chain(scripts)
        .min_by_key(|v| match v {
            Verdict::Deny(..) => 0,
            Verdict::Hint(..) => 1,
            Verdict::Escape(_) => 2,
        })?;
    Some(mark(cmd.contains("acus-skip"), v))
}

fn is_interpreter(head: &str) -> bool {
    head.starts_with("python") || matches!(head, "node" | "ruby" | "bun" | "deno")
}

/// `cat > x.py <<EOF` or `tee x.py <<EOF`: the heredoc body is a script that runs in a later call.
fn script_writer(seg: &str) -> Option<&'static str> {
    let (head, words) = head(seg)?;
    if !matches!(head, "cat" | "tee") {
        return None;
    }
    words
        .map(|w| w.trim_matches(['\'', '"', '>']))
        .find_map(|w| match w.rsplit_once('.')?.1 {
            "py" => Some("python"),
            "js" | "mjs" | "cjs" => Some("node"),
            "rb" => Some("ruby"),
            _ => None,
        })
}

/// A script that reads a source file, replaces text in it and writes it back: a hand-made patch.
fn is_edit_script(s: &str) -> bool {
    static CODE_PATH: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(&format!(r#"['"][^'"\s]+\.({})['"]"#, CODE.join("|"))).unwrap()
    });
    ["write(", "write_text(", "writeFileSync(", "writeFile("]
        .iter()
        .any(|w| s.contains(w))
        && [
            ".replace(",
            "re.sub(",
            ".index(",
            ".replaceAll(",
            ".gsub(",
            ".sub(",
        ]
        .iter()
        .any(|r| s.contains(r))
        && CODE_PATH.is_match(s)
}

/// A build or test piped into `tail`, `head` or `grep`: the cut output often hides the failure,
/// and the build runs again to see more.
fn filtered_build(seg: &str, (next, piped): &(String, bool)) -> Option<Verdict> {
    let (filter, _) = head(next).filter(|_| *piped)?;
    if !matches!(
        filter,
        "tail"
            | "head"
            | "grep"
            | "egrep"
            | "rg"
            | "select-object"
            | "select"
            | "select-string"
            | "sls"
    ) {
        return None;
    }
    let (build, args) = head(seg)?;
    let sub = args
        .filter(|a| !a.starts_with(['-', '+']))
        .find(|a| *a != "run");
    let is_build = match build {
        "cargo" => sub.is_some_and(|s| {
            matches!(
                s,
                "test" | "build" | "check" | "clippy" | "nextest" | "t" | "b" | "c"
            )
        }),
        "swift" => sub.is_some_and(|s| matches!(s, "build" | "test")),
        "go" => sub.is_some_and(|s| matches!(s, "build" | "test" | "vet")),
        "npm" | "pnpm" | "yarn" | "bun" => sub.is_some_and(|s| {
            s.starts_with("test") || s.starts_with("build") || s.contains("check") || s == "lint"
        }),
        "xcodebuild" | "pytest" | "tsc" | "vitest" | "jest" | "mypy" | "make" | "gradle"
        | "gradlew" | "mvn" | "dotnet" => true,
        _ => false,
    };
    if !is_build {
        return None;
    }
    let cmd = seg.trim().trim_end_matches("2>&1").trim_end();
    let example = if cmd.contains('\'') {
        "acus ctx 'COMMAND'".to_owned()
    } else {
        format!("acus ctx '{cmd}'")
    };
    Some(Verdict::Deny(
        "build-pipe",
        format!(
            "Use acus instead of piping {build} into {filter}: `{example}` drops build noise, shows the failures with the code they point at, and saves the full log for follow-up reads, so the build need not run again (`acus ctx` alone runs the project's tests). {ESCAPE}"
        ),
    ))
}

/// Edit scripts are only hinted at: telling them apart from data processing is a heuristic.
fn script_hint(interp: &str) -> Verdict {
    Verdict::Hint(
        "edit-script",
        format!(
            "acus hint: this {interp} script edits source files. Next time use one `acus patch` with SEARCH/REPLACE blocks or `*** Replace All: PATHS` (literal or REGEX): it edits every file at once, all or nothing, and prints the written lines."
        ),
    )
}

/// The command word of a simple command, past assignments and keywords, and its arguments.
fn head(seg: &str) -> Option<(&str, impl Iterator<Item = &str>)> {
    let mut words = seg.split_whitespace().skip_while(|w| {
        is_assignment(w) || matches!(*w, "do" | "then" | "else" | "{" | "!" | "time" | "sudo")
    });
    let mut head = words.next()?.rsplit('/').next()?;
    if matches!(head, "uv" | "env" | "exec" | "npx" | "bunx") {
        // `uv run python …` runs the interpreter.
        head = words
            .by_ref()
            .find(|w| !w.starts_with('-') && *w != "run")?;
    }
    Some((head, words))
}

fn rule(seg: &str, mut piped: bool) -> Option<Verdict> {
    let (mut head, mut words) = head(seg)?;
    if is_interpreter(head) {
        // Inline `-c` / `-e` scripts sit in the segment itself.
        return is_edit_script(seg).then(|| script_hint(head));
    }
    if head == "xargs" {
        // `… | xargs grep` searches the files it is given.
        head = words.by_ref().find(|w| !w.starts_with('-'))?;
        piped = false;
    }
    let args: Vec<&str> = words.collect();
    let in_place = args.iter().any(|a| {
        *a == "--in-place" || (a.starts_with('-') && !a.starts_with("--") && a.contains('i'))
    });
    let code_file = args
        .iter()
        .any(|a| is_code_path(a.trim_matches(['\'', '"'])));
    if is_listing(head, &args) {
        return Some(Verdict::Hint("tree", TREE_HINT.into()));
    }
    if head == "git" {
        return git_hint(&args);
    }
    // PowerShell cmdlets and aliases (lowercased by `check_ps`) share the rules.
    let (name, reason) = match head {
        "grep" | "rg" | "egrep" | "fgrep" | "ag" | "ack" | "select-string" | "sls" if !piped => (
            "grep",
            format!(
                "Use acus instead of {head}: `acus find 'PAT' [PATH…]` (`--block` adds the enclosing code, `-l` lists files, `-w`, `-t TYPE`, `-g GLOB`, `-u --hidden` include ignored and hidden files). {ESCAPE}"
            ),
        ),
        "sed" | "perl" if in_place => (
            "sed-i",
            format!(
                "Use acus instead of {head} -i: one `acus patch` with `*** Replace All: PATHS` or SEARCH/REPLACE blocks edits every file at once and prints the written lines. {ESCAPE}"
            ),
        ),
        "sed" | "cat" | "head" | "tail" | "nl" | "bat" | "less" | "get-content" | "gc" | "type"
            if !piped
                && code_file
                && !seg.contains('>')
                && (head != "sed" || args.contains(&"-n"))
                && !args.iter().any(|a| matches!(*a, "-f" | "-F")) =>
        {
            (
                "read-shell",
                format!(
                    "Use acus instead of {head}: `acus show path path:A-B path#Symbol` reads several files, ranges or symbols in one call; `acus outline PATH` gives an overview. {ESCAPE}"
                ),
            )
        }
        _ => return None,
    };
    Some(Verdict::Deny(name, reason))
}

/// Plain `git status|diff|log|show` print more than an agent needs; `acus diff` and `acus log` group
/// and cap them. Output already compacted by a flag, and `git show REV:path` file reads, stay unhinted.
fn git_hint(args: &[&str]) -> Option<Verdict> {
    let mut it = args.iter().copied();
    let sub = loop {
        match it.next()? {
            "-C" | "-c" | "--git-dir" | "--work-tree" => {
                it.next();
            }
            a if a.starts_with('-') => {}
            a => break a,
        }
    };
    let rest: Vec<&str> = it.collect();
    let has = |flags: &[&str]| {
        rest.iter().any(|a| {
            flags
                .iter()
                .any(|f| a == f || a.starts_with(&format!("{f}=")))
        })
    };
    let (rule, hint) = match sub {
        "status" if !has(&["-s", "-sb", "--short", "--porcelain"]) => (
            "git-status",
            "acus hint: `acus diff` lists uncommitted changes per file and function with line counts, untracked files included; `-p` adds the changed lines and `--staged` limits it to the index.",
        ),
        "diff"
            if !has(&[
                "--stat",
                "--shortstat",
                "--numstat",
                "--name-only",
                "--name-status",
                "--quiet",
                "--check",
                "--exit-code",
                "--summary",
                "--no-index",
            ]) =>
        {
            (
                "git-diff",
                "acus hint: `acus diff [REV|A..B] [PATH…]` groups changes by enclosing function; `-p` adds the changed lines, `--staged` limits it to the index.",
            )
        }
        "log"
            if !has(&[
                "--oneline",
                "--format",
                "--pretty",
                "--name-only",
                "--stat",
                "--shortstat",
            ]) =>
        {
            (
                "git-log",
                "acus hint: `acus log [REV] [PATH…] [-n N]` prints one line per commit with date, author and changed lines; `acus diff REV^..REV` shows what one commit changed.",
            )
        }
        "show"
            if !has(&[
                "--stat",
                "--name-only",
                "--name-status",
                "-s",
                "--no-patch",
                "--format",
                "--pretty",
            ]) && !rest.iter().any(|a| a.contains(':')) =>
        {
            (
                "git-show",
                "acus hint: `acus diff REV^..REV` shows one commit's changes per function (`-p` adds the lines); `acus log` lists commits.",
            )
        }
        _ => return None,
    };
    Some(Verdict::Hint(rule, hint.into()))
}

/// Recursive directory listings: `tree`, `ls -R`, `find` without filters, `Get-ChildItem -Recurse`.
/// A name filter or wildcard makes it a file search, which stays unhinted.
fn is_listing(head: &str, args: &[&str]) -> bool {
    const FIND_FILTERS: &[&str] = &[
        "-name", "-iname", "-path", "-ipath", "-regex", "-exec", "-execdir", "-delete", "-newer",
        "-mtime", "-mmin", "-size", "-empty", "-perm", "-user",
    ];
    let short = |a: &&&str| a.starts_with('-') && !a.starts_with("--");
    match head {
        "tree" => true,
        "ls" => args.iter().filter(short).any(|a| a.contains('R')) || args.contains(&"--recursive"),
        "get-childitem" | "gci" | "dir" => {
            args.iter().any(|a| matches!(*a, "-recurse" | "-r" | "/s"))
                && !args
                    .iter()
                    .any(|a| a.contains('*') || matches!(*a, "-filter" | "-include"))
        }
        "find" => !args.iter().any(|a| FIND_FILTERS.contains(a)),
        _ => false,
    }
}

fn is_code_path(w: &str) -> bool {
    !w.starts_with('-') && w.rsplit_once('.').is_some_and(|(_, e)| CODE.contains(&e))
}

fn is_assignment(w: &str) -> bool {
    w.split_once('=').is_some_and(|(k, _)| {
        !k.is_empty() && k.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
    })
}

/// Splits heredoc bodies, which are data rather than commands, from the rest of `cmd`;
/// each body comes with the line that opened it.
fn strip_heredocs(cmd: &str) -> (String, Vec<(String, String)>) {
    static START: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r#"<<-?\s*['"]?([A-Za-z_][A-Za-z0-9_]*)['"]?"#).unwrap());
    let mut out = String::new();
    let mut docs: Vec<(String, String)> = Vec::new();
    let mut end: Option<String> = None;
    for line in cmd.lines() {
        if let Some(d) = &end {
            if line.trim() == d {
                end = None;
            } else if let Some((_, body)) = docs.last_mut() {
                body.push_str(line);
                body.push('\n');
            }
            continue;
        }
        end = START.captures(line).map(|c| c[1].to_owned());
        if end.is_some() {
            docs.push((line.to_owned(), String::new()));
        }
        out.push_str(line);
        out.push('\n');
    }
    (out, docs)
}

/// Simple commands outside quotes, each with whether it reads a pipe.
fn segments(cmd: &str) -> Vec<(String, bool)> {
    let mut out = Vec::new();
    let (mut cur, mut piped) = (String::new(), false);
    let mut quote = None;
    let mut chars = cmd.chars().peekable();
    let mut cut = |cur: &mut String, piped: bool| out.push((std::mem::take(cur), piped));
    while let Some(c) = chars.next() {
        match (quote, c) {
            (Some(q), _) => {
                if c == q {
                    quote = None;
                }
                cur.push(c);
            }
            (None, '\'' | '"') => {
                quote = Some(c);
                cur.push(c);
            }
            (None, '\\') => {
                cur.push(c);
                cur.extend(chars.next());
            }
            (None, '&') if cur.ends_with('>') || chars.peek() == Some(&'>') => cur.push(c),
            (None, '|') if chars.peek() != Some(&'|') => {
                cut(&mut cur, piped);
                piped = true;
            }
            (None, ';' | '\n' | '(' | ')' | '`' | '&' | '|') => {
                if matches!(c, '&' | '|') {
                    chars.next_if_eq(&c);
                }
                cut(&mut cur, piped);
                piped = false;
            }
            _ => cur.push(c),
        }
    }
    cut(&mut cur, piped);
    out
}

#[cfg(test)]
mod tests {
    use super::{READ_MAX, Verdict, check, check_ps, grep_tool, read_verdict};
    use acus_syntax::Lang;
    use serde_json::json;

    #[test]
    fn refuses_file_searches_reads_and_in_place_edits() {
        for cmd in [
            "grep -rn foo src",
            "cd app && rg -n 'fn main'",
            "git ls-files | xargs grep -n TODO",
            "for f in $(git ls-files); do grep -c x $f; done",
            "sed -n 1,40p src/main.rs",
            "sed -i '' 's/a/b/' Sources/App.swift",
            "perl -pi -e 's/a/b/' lib.py",
            "cat src/a.rs src/b.rs",
            "X=1 head -50 README.md",
            "python3 - <<'EOF'\nopen('a.rs','w').write(s.replace('a','b'))\nEOF\ngrep -n x a.rs",
            "cargo test 2>&1 | grep -E 'FAILED|panicked'",
            "cd crates && cargo test -q -p acus-cli | tail -30",
            "swift build 2>&1 | head -50",
            "npx tsc --noEmit | head",
            "pnpm run build | tail -20",
        ] {
            assert!(matches!(check(cmd), Some(Verdict::Deny(..))), "{cmd}");
        }
    }

    #[test]
    fn hints_at_edit_scripts() {
        for cmd in [
            "python3 - <<'EOF'\np='src/a.rs'\ns=open(p).read()\nopen(p,'w').write(s.replace('a','b'))\nEOF",
            "cd app && uv run python - <<'E'\nimport re, pathlib\nf=pathlib.Path(\"Sources/A.swift\")\nf.write_text(re.sub(r'x','y',f.read_text()))\nE",
            "python3 -c \"p='lib.py'; s=open(p).read(); open(p,'w').write(s.replace('a','b'))\"",
            "node -e \"const fs=require('fs');fs.writeFileSync('a.ts',fs.readFileSync('a.ts','utf8').replace('a','b'))\"",
            "cd /w && cat > .tmp_e.py <<'EOF'\nfrom pathlib import Path\np = Path(\"a/b.py\")\np.write_text(p.read_text().replace('x', 'y'))\nEOF\npython .tmp_e.py && rm .tmp_e.py",
            "tee fix.rb <<'EOF'\nf = 'a.rb'\nFile.write(f, File.read(f).gsub('a', 'b'))\nEOF",
        ] {
            assert!(matches!(check(cmd), Some(Verdict::Hint(..))), "{cmd}");
        }
        // A script file that does not edit sources stays unhinted.
        assert!(check("cat > run.py <<'EOF'\nprint('hi')\nEOF").is_none());
    }

    #[test]
    fn hints_at_acus_map_for_recursive_listings() {
        for cmd in [
            "tree -L 2",
            "ls -laR src",
            "find . -type f",
            "find Sources -maxdepth 2",
        ] {
            assert!(
                matches!(check(cmd), Some(Verdict::Hint("tree", _))),
                "{cmd}"
            );
        }
        assert!(matches!(
            check_ps("Get-ChildItem -Recurse src"),
            Some(Verdict::Hint("tree", _))
        ));
        for cmd in [
            "ls -la",
            "find . -name '*.rs'",
            "find build -delete",
            "ls --color",
        ] {
            assert!(check(cmd).is_none(), "{cmd}");
        }
    }

    #[test]
    fn hints_at_acus_for_plain_git_history_and_status() {
        for (cmd, rule) in [
            ("git status", "git-status"),
            ("git diff", "git-diff"),
            ("git diff --cached src/main.rs", "git-diff"),
            ("git -C ../x log -5", "git-log"),
            ("git --no-pager log", "git-log"),
            ("git show HEAD", "git-show"),
        ] {
            assert!(
                matches!(check(cmd), Some(Verdict::Hint(r, _)) if r == rule),
                "{cmd}"
            );
        }
        assert!(matches!(
            check_ps("git status"),
            Some(Verdict::Hint("git-status", _))
        ));
        for cmd in [
            "git status --short",
            "git status -sb",
            "git diff --stat",
            "git diff --name-only HEAD~1",
            "git log --oneline -5",
            "git log --format=%h",
            "git show HEAD:src/main.rs",
            "git show --stat HEAD",
            "git commit -m x",
            "git add -A",
        ] {
            assert!(check(cmd).is_none(), "{cmd}");
        }
    }

    #[test]
    fn allows_filters_data_and_escapes() {
        for cmd in [
            "git log --oneline | grep fix",
            "cargo tree | grep serde",
            "cargo test 2>&1 | tee test.log",
            "acus ctx 'cargo test | tail'",
            "npm install | tail -3",
            "git commit -m \"fix; grep foo\"",
            "cat > notes.md <<'EOF'\ngrep foo bar\nEOF",
            "acus patch <<'P'\nsed -i x a.rs\nP",
            "cat Cargo.lock",
            "tail -f server.log",
            "cat a.rs > b.rs",
            "sed 's/a/b/' input.txt",
            "ls src && find . -name '*.rs'",
            "python3 - <<'EOF'\nimport json\nprint(json.load(open('data.json'))['x'].replace('a','b'))\nEOF",
            "python3 - <<'EOF'\nrows=open('in.csv').read().replace(';',',')\nopen('out.csv','w').write(rows)\nEOF",
            "acus patch <<'P'\n*** Update File: tool.py\n+open('a.py','w').write(s.replace('a','b'))\nP",
        ] {
            assert_eq!(check(cmd), None, "{cmd}");
        }
    }

    #[test]
    fn refuses_whole_reads_of_large_sources_with_their_outline() {
        let small: String = (0..READ_MAX).map(|i| format!("fn f{i}() {{}}\n")).collect();
        assert_eq!(read_verdict("src/a.rs", Lang::Rust, &small), None);
        let large = format!("{small}fn last() {{}}\n");
        let Some(Verdict::Deny("read-tool", reason)) = read_verdict("src/a.rs", Lang::Rust, &large)
        else {
            panic!("large file read whole");
        };
        assert!(reason.contains("src/a.rs has 301 lines"), "{reason}");
        assert!(
            reason.contains("… 241 more (acus outline src/a.rs)"),
            "{reason}"
        );
        assert!(reason.contains("fn f0 1-1"), "{reason}");
    }

    #[cfg(windows)]
    #[test]
    fn shows_windows_paths_relative_to_the_session() {
        assert_eq!(
            super::relative(r"C:\repo\src\a.rs", Some(r"C:\repo")),
            r"src\a.rs"
        );
        assert_eq!(super::relative(r"C:\repo", Some(r"C:\repo")), ".");
        assert_eq!(
            super::relative(r"D:\other\a.rs", Some(r"C:\repo")),
            r"D:\other\a.rs"
        );
    }

    #[test]
    fn logs_escapes_as_such() {
        for (cmd, rule) in [
            ("command grep -rn foo src", "grep"),
            ("cd a && command cat src/a.rs", "read-shell"),
            ("command cargo test | tail", "build-pipe"),
            ("grep -rn foo src # acus-skip", "grep"),
        ] {
            assert_eq!(check(cmd), Some(Verdict::Escape(rule)), "{cmd}");
        }
        // An escape elsewhere does not excuse another refused segment.
        assert!(matches!(
            check("command grep x a.rs; grep y src"),
            Some(Verdict::Deny("grep", _))
        ));
    }

    #[test]
    fn turns_grep_tool_calls_into_acus_find() {
        let cwd = Some("/repo");
        let deny = |v| match grep_tool(&v, cwd) {
            Some(Verdict::Deny("grep-tool", r)) => r,
            other => panic!("{other:?}"),
        };
        let r = deny(
            json!({"pattern": "fn main", "path": "/repo/src", "output_mode": "content", "-C": 3, "type": "rust"}),
        );
        assert!(
            r.contains("`acus find 'fn main' 'src' -t rust --block`"),
            "{r}"
        );
        let r = deny(json!({"pattern": "it's", "path": "/repo", "glob": "*.md"}));
        assert!(r.contains("`acus find \"it's\" -g '*.md' -l`"), "{r}");
        assert_eq!(
            grep_tool(&json!({"pattern": "x", "output_mode": "count"}), cwd),
            None
        );
        assert_eq!(
            grep_tool(&json!({"pattern": "a\nb", "multiline": true}), cwd),
            None
        );
    }

    #[test]
    fn applies_the_rules_to_powershell() {
        for (cmd, rule) in [
            ("Select-String -Path src\\*.rs -Pattern foo", "grep"),
            ("Get-Content src\\main.rs", "read-shell"),
            ("cargo test 2>&1 | Select-Object -Last 30", "build-pipe"),
            (
                "(Get-Content a.rs) -replace 'x','y' | Set-Content a.rs",
                "ps-replace",
            ),
        ] {
            match check_ps(cmd) {
                Some(Verdict::Deny(r, reason)) => {
                    assert_eq!(r, rule, "{cmd}");
                    assert!(reason.contains("# acus-skip"), "{reason}");
                }
                other => panic!("{cmd}: {other:?}"),
            }
        }
        assert_eq!(
            check_ps("Get-Content src\\main.rs # acus-skip"),
            Some(Verdict::Escape("read-shell"))
        );
        for cmd in [
            "git log --oneline | Select-String fix",
            "Get-Content settings.ini",
            "Get-ChildItem -Recurse *.rs",
        ] {
            assert_eq!(check_ps(cmd), None, "{cmd}");
        }
    }
}
