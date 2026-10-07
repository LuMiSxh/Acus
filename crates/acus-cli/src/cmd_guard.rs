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

/// PowerShell: the same rules (cmdlets ignore case, so the rules see a lowercased command),
/// plus `-replace … | Set-Content` edits and .NET file reads.
fn check_ps(cmd: &str) -> Option<Verdict> {
    let c = cmd.to_lowercase();
    let words = |is_path: fn(&str) -> bool| {
        c.split(|ch: char| ch.is_whitespace() || "()'\",;".contains(ch))
            .filter(move |w| is_path(w))
    };
    let writes = [
        "set-content",
        "out-file",
        "writealltext",
        "writealllines",
        "| sc ",
        " > ",
    ]
    .iter()
    .any(|w| c.contains(w));
    let replaces = ["-replace", "-creplace", "-ireplace", ".replace("]
        .iter()
        .any(|w| c.contains(w));
    let v = if replaces && writes && words(is_code_path).next().is_some() {
        let files: Vec<&str> = words(is_code_path).collect();
        Verdict::Deny(
            "ps-replace",
            format!(
                "Nothing ran: the command was refused as a whole because it rewrites {} with -replace. Use one `acus patch` with `*** Replace All: {}` and a SEARCH/REPLACE block (or SEARCH/REPLACE blocks under `*** Update File:`): it edits every file at once and prints the written lines. {ESCAPE_PS}",
                files.join(", "),
                files.join(" ")
            ),
        )
    } else if ["readalltext", "readalllines", "readlines("]
        .iter()
        .any(|w| c.contains(w))
        && words(is_source_path).next().is_some()
        && !c.contains("acus ")
    {
        Verdict::Deny(
            "read-shell",
            format!(
                "Nothing ran: the command was refused as a whole because it reads source files through .NET. Use acus instead: `acus show path path:A-B path#Symbol` reads several files, ranges or symbols in one call; `acus outline PATH` gives an overview. {ESCAPE_PS}"
            ),
        )
    } else {
        match check_in(cmd, true)? {
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
    check_in(cmd, false)
}

/// `check` for Bash, or for PowerShell, where cmdlets ignore case: the rules see a lowercased
/// command and a refusal quotes the original.
fn check_in(cmd: &str, ps: bool) -> Option<Verdict> {
    let low = if ps {
        cmd.to_lowercase()
    } else {
        cmd.to_owned()
    };
    let (rest, docs) = strip_heredocs(&low);
    let scripts = docs.iter().filter_map(|(opener, body)| {
        let interp = segments(opener, ps).iter().find_map(|(seg, _)| {
            head(seg)
                .map(|(h, _)| h.to_owned())
                .filter(|h| is_interpreter(h))
                .or_else(|| script_writer(seg).map(str::to_owned))
        })?;
        is_edit_script(body).then(|| script_hint(&interp))
    });
    let segs = segments(&rest, ps);
    let shown = segments(&strip_heredocs(cmd).0, ps);
    let mark = |esc: bool, v: Verdict| if esc { v.escaped() } else { v };
    // Each verdict with the first and last segment it is about.
    // PowerShell reasons quote the original case; its segments line up with the lowercased ones.
    let orig = if shown.len() == segs.len() {
        &shown
    } else {
        &segs
    };
    let filtered = segs.windows(2).enumerate().filter_map(|(i, w)| {
        let (esc, seg) = unescape(&w[0].0);
        let build = filtered_build(seg, &w[1]).map(|v| (mark(esc, v), i, i + 1));
        build.or_else(|| {
            let (esc, seg) = unescape(&orig[i].0);
            let v = listed_search(seg, &orig[i + 1], ps)?;
            Some((mark(esc, v), i, i + 1))
        })
    });
    let (v, from, to) = segs
        .iter()
        .enumerate()
        .filter_map(|(i, (seg, piped))| {
            let (esc, seg) = unescape(seg);
            rule(seg, *piped, ps).map(|v| (mark(esc, v), i, i))
        })
        .chain(filtered)
        .chain(scripts.map(|v| (v, 0, 0)))
        .min_by_key(|(v, ..)| match v {
            Verdict::Deny(..) => 0,
            Verdict::Hint(..) => 1,
            Verdict::Escape(_) => 2,
        })?;
    let compound = segs.iter().filter(|(s, _)| !s.trim().is_empty()).count() > 1;
    let v = match v {
        // A refusal stops the whole command, so a compound one says which part and what to run.
        Verdict::Deny(rule, reason) if compound => {
            let at = |i: usize| orig[i].0.as_str();
            let part = (from..=to).map(at).collect::<Vec<_>>().join(" | ");
            let part = one_line(&part);
            let instead = suggest(rule, at(from))
                .filter(|s| s.len() <= 240)
                .map_or("Replace that segment".to_owned(), |s| {
                    format!("Replace it with {s}")
                });
            Verdict::Deny(
                rule,
                format!(
                    "Nothing ran: the whole command was refused because of `{part}`; the other segments are fine. {instead} and send the command again. {reason}"
                ),
            )
        }
        v => v,
    };
    Some(mark(cmd.contains("acus-skip"), v))
}

/// A segment on one line, cut to a length that reads in a refusal.
fn one_line(s: &str) -> String {
    let s = s.split_whitespace().collect::<Vec<_>>().join(" ");
    match s.char_indices().nth(160) {
        Some((i, _)) => format!("{}…", &s[..i]),
        None => s,
    }
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

/// PowerShell's way to grep: `Get-ChildItem … | Select-String PAT` searches the contents of the
/// files listed. Narrowed to plain files (`-Include *.log`) it is a search of data and passes.
fn listed_search(seg: &str, (next, piped): &(String, bool), ps: bool) -> Option<Verdict> {
    let (filter, fargs) = head(next).filter(|_| ps && *piped)?;
    let (lister, largs) = head(seg)?;
    let (filter, lister) = (filter.to_lowercase(), lister.to_lowercase());
    if !matches!(filter.as_str(), "select-string" | "sls")
        || !matches!(lister.as_str(), "get-childitem" | "gci" | "ls" | "dir")
    {
        return None;
    }
    let (mut paths, mut globs) = (Vec::new(), Vec::new());
    let mut words = largs;
    while let Some(a) = words.next() {
        match a.to_lowercase().as_str() {
            // Only names are listed, nothing is searched.
            "-name" => return None,
            "-filter" | "-include" => {
                globs.extend(
                    words
                        .next()
                        .into_iter()
                        .flat_map(|g| g.split(','))
                        .map(unquote),
                );
            }
            "-path" | "-literalpath" => paths.extend(words.next().map(unquote)),
            "-exclude" | "-depth" | "-attributes" => {
                words.next();
            }
            l if l.starts_with(['-', '/']) => {}
            _ => paths.push(unquote(a)),
        }
    }
    let named = if globs.is_empty() { &paths } else { &globs };
    if !named.is_empty() && named.iter().all(|n| is_plain_file(n)) {
        return None;
    }
    let fargs: Vec<&str> = fargs.collect();
    let mut search = parse_search(&filter, &fargs);
    search.operands = paths
        .into_iter()
        .filter(|p| !matches!(*p, "." | ".\\"))
        .collect();
    search
        .flags
        .extend(globs.iter().map(|g| format!("-g {}", quote(g))));
    Some(Verdict::Deny(
        "grep",
        format!(
            "Use acus instead of piping {lister} into {filter}: `{}` searches the same files and groups the hits by file and enclosing symbol. {ESCAPE}",
            find_cmd(&search)
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

/// Whitespace-separated words, with quoted text kept together (quotes included).
fn split_words(s: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let (mut start, mut quote) = (None, None);
    for (i, c) in s.char_indices() {
        match (quote, c) {
            (Some(q), _) if c == q => quote = None,
            (Some(_), _) => {}
            (None, '\'' | '"') => {
                quote = Some(c);
                start = start.or(Some(i));
            }
            (None, c) if c.is_whitespace() => out.extend(start.take().map(|st| &s[st..i])),
            _ => start = start.or(Some(i)),
        }
    }
    out.extend(start.map(|st| &s[st..]));
    out
}

/// A command name: the file name without its directory (either separator) and `.exe`.
fn command_name(w: &str) -> &str {
    let w = w.trim_matches(['\'', '"']);
    let w = w.rsplit(['/', '\\']).next().unwrap_or(w);
    w.strip_suffix(".exe").unwrap_or(w)
}

/// The command word of a simple command, past assignments and keywords, and its arguments.
fn head(seg: &str) -> Option<(&str, impl Iterator<Item = &str>)> {
    let mut words = split_words(seg).into_iter().skip_while(|w| {
        is_assignment(w) || matches!(*w, "do" | "then" | "else" | "{" | "!" | "time" | "sudo")
    });
    let mut head = command_name(words.next()?);
    if matches!(head, "uv" | "env" | "exec" | "npx" | "bunx") {
        // `uv run python …` runs the interpreter.
        head = command_name(
            words
                .by_ref()
                .find(|w| !w.starts_with('-') && *w != "run" && !is_assignment(w))?,
        );
    }
    Some((head, words))
}

/// Commands that search file contents; PowerShell and cmd.exe names included.
const GREPS: &[&str] = &[
    "grep",
    "rg",
    "egrep",
    "fgrep",
    "ag",
    "ack",
    "select-string",
    "sls",
    "findstr",
];

fn grep_reason(head: &str) -> String {
    format!(
        "Use acus instead of {head}: `acus find 'PAT' [PATH…]` (`--block` adds the enclosing code, `-l` lists files, `-w`, `-t TYPE`, `-g GLOB`, `-u --hidden` include ignored and hidden files). {ESCAPE}"
    )
}

/// A search command's pattern, targets and the `acus find` flags that mean the same.
#[derive(Default)]
struct Search<'a> {
    pattern: Option<&'a str>,
    operands: Vec<&'a str>,
    /// Searches the working directory when it names no file.
    recursive: bool,
    flags: Vec<String>,
}

/// The value of a short flag: the rest of its cluster (`-A3`) or the next word.
fn flag_value<'a>(
    cluster: &'a str,
    at: usize,
    words: &mut impl Iterator<Item = &'a str>,
) -> Option<&'a str> {
    match cluster.get(at + 1..) {
        Some(v) if !v.is_empty() => Some(v),
        _ => words.next(),
    }
}

fn unquote(w: &str) -> &str {
    w.trim_matches(['\'', '"'])
}

/// Reads grep, rg, `Select-String` and `findstr` arguments; unknown flags are skipped.
fn parse_search<'a>(head: &str, args: &[&'a str]) -> Search<'a> {
    // Select-String and findstr ignore case in their flags.
    let ci = matches!(head, "select-string" | "sls" | "findstr");
    let mut s = Search {
        recursive: matches!(head, "rg" | "ag" | "ack"),
        ..Search::default()
    };
    let mut have_pattern = false;
    let mut words = args.iter().copied();
    while let Some(raw) = words.next() {
        let lc = if ci {
            raw.to_lowercase()
        } else {
            raw.to_owned()
        };
        // Redirections are not operands; `< file` reads the file, `<<<` a string.
        let r = raw.trim_start_matches(|c: char| c.is_ascii_digit() || c == '&');
        if r.starts_with('>') {
            if matches!(r, ">" | ">>") {
                words.next();
            }
            continue;
        }
        if let Some(f) = r.strip_prefix('<') {
            match f {
                "<<" => {
                    words.next();
                }
                "" | "<" => {}
                f => s.operands.push(f),
            }
            continue;
        }
        if matches!(head, "select-string" | "sls") && raw.starts_with('-') {
            match lc.as_str() {
                "-pattern" => {
                    have_pattern = true;
                    s.pattern = words.next().map(unquote);
                }
                "-path" | "-literalpath" => s.operands.extend(
                    words
                        .next()
                        .into_iter()
                        .flat_map(|p| p.split(','))
                        .map(unquote),
                ),
                "-include" | "-exclude" | "-context" | "-encoding" | "-inputobject" => {
                    words.next();
                }
                "-simplematch" => s.flags.push("-F".into()),
                "-list" => s.flags.push("-l".into()),
                _ => {}
            }
            continue;
        }
        if head == "findstr"
            && raw.starts_with('/')
            && !raw.contains('\\')
            && (raw.len() == 2 || raw.get(2..).is_some_and(|r| r.starts_with(':')))
        {
            match lc.as_bytes()[1] {
                b's' => s.recursive = true,
                b'i' => s.flags.push("-i".into()),
                b'm' => s.flags.push("-l".into()),
                b'c' | b'g' => {
                    have_pattern = true;
                    if lc.as_bytes()[1] == b'c' {
                        s.pattern = raw.get(3..).map(unquote).filter(|p| !p.is_empty());
                    }
                    s.flags.push("-F".into());
                }
                _ => {}
            }
            continue;
        }
        if raw == "-" {
            continue;
        }
        if let Some(long) = raw.strip_prefix("--") {
            let (name, attached) = long
                .split_once('=')
                .map_or((long, None), |(n, v)| (n, Some(v)));
            match name {
                "recursive" | "dereference-recursive" => s.recursive = true,
                "ignore-case" => s.flags.push("-i".into()),
                "word-regexp" => s.flags.push("-w".into()),
                "files-with-matches" => s.flags.push("-l".into()),
                "fixed-strings" => s.flags.push("-F".into()),
                "regexp" | "file" => {
                    have_pattern = true;
                    let v = attached.or_else(|| words.next());
                    if name == "regexp" {
                        s.pattern = v.map(unquote);
                    }
                }
                "include" | "glob" => {
                    if let Some(v) = attached.or_else(|| words.next()) {
                        s.flags.push(format!("-g {}", quote(unquote(v))));
                    }
                }
                "context" | "after-context" | "before-context" => {
                    attached.or_else(|| words.next());
                    s.flags.push("--block".into());
                }
                "max-count" | "exclude" | "exclude-dir" | "max-depth" | "type" => {
                    attached.or_else(|| words.next());
                }
                _ => {}
            }
            continue;
        }
        if let Some(cluster) = raw.strip_prefix('-').filter(|c| !c.is_empty()) {
            for (i, c) in cluster.char_indices() {
                match c {
                    'r' | 'R' if !matches!(head, "rg" | "ag" | "ack") => s.recursive = true,
                    'i' => s.flags.push("-i".into()),
                    'w' => s.flags.push("-w".into()),
                    'l' => s.flags.push("-l".into()),
                    'F' => s.flags.push("-F".into()),
                    'e' | 'f' => {
                        let v = flag_value(cluster, i, &mut words);
                        have_pattern = true;
                        if c == 'e' {
                            s.pattern = v.map(unquote);
                        }
                        break;
                    }
                    'g' | 't' => {
                        if let Some(v) = flag_value(cluster, i, &mut words) {
                            s.flags.push(if c == 'g' {
                                format!("-g {}", quote(unquote(v)))
                            } else {
                                format!("-t {}", unquote(v))
                            });
                        }
                        break;
                    }
                    'm' | 'd' | 'D' | 'T' | 'j' | 'M' => {
                        flag_value(cluster, i, &mut words);
                        break;
                    }
                    'A' | 'B' | 'C' => {
                        flag_value(cluster, i, &mut words);
                        s.flags.push("--block".into());
                        break;
                    }
                    _ => {}
                }
            }
            continue;
        }
        if have_pattern {
            s.operands.push(unquote(raw));
        } else {
            have_pattern = true;
            s.pattern = Some(unquote(raw));
        }
    }
    // Select-String ignores case unless told otherwise.
    if matches!(head, "select-string" | "sls")
        && !args
            .iter()
            .any(|a| a.eq_ignore_ascii_case("-casesensitive"))
    {
        s.flags.push("-i".into());
    }
    s.flags.dedup();
    s
}

/// The `acus find` call that does what `s` did.
fn find_cmd(s: &Search) -> String {
    let mut cmd = format!("acus find {}", quote(s.pattern.unwrap_or("PAT")));
    for o in &s.operands {
        cmd += &format!(" {}", quote(o));
    }
    for f in &s.flags {
        cmd += &format!(" {f}");
    }
    cmd
}

/// An operand that is clearly not source: a file with an extension that is neither code nor
/// has wildcards (logs, text, data formats).
fn is_plain_file(w: &str) -> bool {
    let w = unquote(w);
    let name = w.rsplit(['/', '\\']).next().unwrap_or(w);
    name.rsplit_once('.').is_some_and(|(_, e)| {
        !e.is_empty() && e.chars().all(|c| c.is_ascii_alphanumeric()) && !is_source_path(name)
    })
}

/// Whether a search is aimed at source: it names a directory, a glob or a source file, or
/// walks the working directory. A search of stdin or of plain files is left alone.
fn aims_at_source(s: &Search, via_xargs: bool) -> bool {
    via_xargs
        || if s.operands.is_empty() {
            s.recursive
        } else {
            !s.operands.iter().all(|o| is_plain_file(o))
        }
}

/// Output sent to a file (`> x`, `>> x`, `1> x`), which makes a command a copy or an append
/// rather than a read. Stderr redirects do not count.
fn redirects_stdout(args: &[&str]) -> bool {
    args.iter().any(|a| {
        let a = a.trim_start_matches('1');
        a.starts_with('>') && !a.starts_with(">&")
    })
}

fn rule(seg: &str, mut piped: bool, ps: bool) -> Option<Verdict> {
    let (mut head, mut words) = head(seg)?;
    if is_interpreter(head) {
        // Inline `-c` / `-e` scripts sit in the segment itself.
        return is_edit_script(seg).then(|| script_hint(head));
    }
    let via_xargs = head == "xargs";
    if via_xargs {
        // `… | xargs grep` searches the files it is given.
        head = command_name(words.by_ref().find(|w| !w.starts_with('-'))?);
        piped = false;
    }
    let args: Vec<&str> = words.collect();
    let in_place = args.iter().any(|a| {
        *a == "--in-place" || (a.starts_with('-') && !a.starts_with("--") && a.contains('i'))
    });
    // JSON, TOML and YAML are data, read whole on purpose; they do not count as source here.
    let source_file = args
        .iter()
        .any(|a| unquote(a).split(',').any(is_source_path));
    // `find … -exec grep …` searches the files it finds.
    if head == "find"
        && let Some(i) = args.iter().position(|a| matches!(*a, "-exec" | "-execdir"))
        && let Some(&exec) = args.get(i + 1).filter(|c| GREPS.contains(&command_name(c)))
    {
        return Some(Verdict::Deny("grep", grep_reason(command_name(exec))));
    }
    if is_listing(head, &args, ps) {
        return Some(Verdict::Hint("tree", TREE_HINT.into()));
    }
    if head == "git" {
        return git_hint(&args);
    }
    // PowerShell cmdlets and aliases (lowercased by `check_ps`) share the rules.
    let (name, reason) = match head {
        h if GREPS.contains(&h) && !piped && aims_at_source(&parse_search(h, &args), via_xargs) => {
            ("grep", grep_reason(h))
        }
        "sed" | "perl" if in_place => (
            "sed-i",
            format!(
                "Use acus instead of {head} -i: one `acus patch` with `*** Replace All: PATHS` or SEARCH/REPLACE blocks edits every file at once and prints the written lines. {ESCAPE}"
            ),
        ),
        "sed" | "cat" | "head" | "tail" | "nl" | "bat" | "less" | "more" | "get-content" | "gc"
        | "type"
            if !piped
                && source_file
                && !redirects_stdout(&args)
                && (head != "sed" || args.contains(&"-n"))
                && !args
                    .iter()
                    .any(|a| matches!(*a, "-f" | "-F" | "-wait" | "--follow")) =>
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

/// What to run instead of the segment a rule refused, when it can be worked out.
fn suggest(rule: &str, seg: &str) -> Option<String> {
    let (head, words) = head(seg)?;
    let head = head.to_lowercase();
    let args: Vec<&str> = words.collect();
    match rule {
        "grep" if GREPS.contains(&head.as_str()) => {
            Some(format!("`{}`", find_cmd(&parse_search(&head, &args))))
        }
        "read-shell" => show_cmd(&head, &args),
        "sed-i" => {
            let files: Vec<&str> = args
                .iter()
                .map(|a| unquote(a))
                .filter(|a| is_code_path(a))
                .collect();
            let files = if files.is_empty() {
                "PATHS".to_owned()
            } else {
                files.join(" ")
            };
            Some(format!(
                "`acus patch` with `*** Replace All: {files}` and a SEARCH/REPLACE block"
            ))
        }
        _ => None,
    }
}

/// The `acus show` call for a `cat`, `head`, `sed -n 'A,Bp'` or `Get-Content` read.
fn show_cmd(head: &str, args: &[&str]) -> Option<String> {
    let (mut files, mut first, mut range): (Vec<&str>, Option<usize>, Option<(usize, usize)>) =
        (vec![], None, None);
    let mut words = args.iter().copied();
    while let Some(a) = words.next() {
        let lc = a.to_lowercase();
        let count = |v: Option<&str>| v.and_then(|v| v.parse::<usize>().ok());
        match lc.as_str() {
            "-n" if head != "sed" => {
                let n = count(words.next());
                if head != "tail" {
                    first = first.or(n);
                }
            }
            "-totalcount" | "-head" | "-first" => first = first.or(count(words.next())),
            "-tail" | "-last" | "-encoding" | "-delimiter" => {
                words.next();
            }
            "-path" | "-literalpath" => files.extend(words.next().map(unquote)),
            l if l.starts_with('-') => {
                // `head -20` and `head -n20`.
                if head == "head" {
                    first = first.or(l[1..].trim_start_matches('n').parse().ok());
                }
            }
            _ => {
                let a = unquote(a);
                match a
                    .strip_suffix('p')
                    .map(|r| r.split_once(',').unwrap_or((r, r)))
                {
                    Some((x, y)) if head == "sed" && x.parse::<usize>().is_ok() => {
                        range = x.parse().ok().zip(y.parse().ok());
                    }
                    _ => files.extend(a.split(',').filter(|f| is_code_path(f))),
                }
            }
        }
    }
    files.dedup();
    let [file] = files.as_slice() else {
        return (!files.is_empty()).then(|| format!("`acus show {}`", files.join(" ")));
    };
    let range = range.or(first.map(|n| (1, n)));
    Some(match range {
        Some((a, b)) => format!("`acus show {}`", quote(&format!("{file}:{a}-{b}"))),
        None => format!("`acus show {}`", quote(file)),
    })
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
fn is_listing(head: &str, args: &[&str], ps: bool) -> bool {
    const FIND_FILTERS: &[&str] = &[
        "-name", "-iname", "-path", "-ipath", "-regex", "-exec", "-execdir", "-delete", "-newer",
        "-mtime", "-mmin", "-size", "-empty", "-perm", "-user",
    ];
    let short = |a: &&&str| a.starts_with('-') && !a.starts_with("--");
    match head {
        "tree" => true,
        "ls" if !ps => {
            args.iter().filter(short).any(|a| a.contains('R')) || args.contains(&"--recursive")
        }
        // `-r` is `-Recurse` in PowerShell, any prefix of it being enough.
        "get-childitem" | "gci" | "dir" | "ls" if ps || head != "ls" => {
            args.iter().any(|a| {
                let name = a.split(':').next().unwrap_or(a);
                *a == "/s" || (name.len() > 1 && "-recurse".starts_with(name))
            }) && !args
                .iter()
                .any(|a| a.contains('*') || matches!(*a, "-filter" | "-include"))
        }
        "find" => !args.iter().any(|a| FIND_FILTERS.contains(a)),
        _ => false,
    }
}

fn is_code_path(w: &str) -> bool {
    !w.starts_with('-')
        && w.rsplit_once('.')
            .is_some_and(|(_, e)| CODE.contains(&e.to_ascii_lowercase().as_str()))
}

/// Source rather than data: `.json`, `.toml` and `.yaml` files are config and data, which agents
/// read whole on purpose (the Read tool leaves them alone, too).
fn is_source_path(w: &str) -> bool {
    is_code_path(w)
        && !w.rsplit_once('.').is_some_and(|(_, e)| {
            ["json", "toml", "yaml", "yml"].contains(&e.to_ascii_lowercase().as_str())
        })
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
fn segments(cmd: &str, ps: bool) -> Vec<(String, bool)> {
    let mut out = Vec::new();
    let (mut cur, mut piped) = (String::new(), false);
    let mut quote = None;
    let mut chars = cmd.chars().peekable();
    let mut cut = |cur: &mut String, piped: bool| out.push((std::mem::take(cur), piped));
    while let Some(c) = chars.next() {
        match (quote, c) {
            // Inside double quotes an escaped quote does not end the string.
            (Some('"'), '\\') if !ps => {
                cur.push(c);
                cur.extend(chars.next());
            }
            (Some('"'), '`') if ps => {
                cur.push(c);
                cur.extend(chars.next());
            }
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
            // Backslash is a path separator in PowerShell, whose escape is the backtick.
            (None, '\\') if !ps => {
                cur.push(c);
                cur.extend(chars.next());
            }
            (None, '`') if ps => {
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
    /// The reason of a refused command.
    fn deny(v: Option<Verdict>) -> (&'static str, String) {
        match v {
            Some(Verdict::Deny(r, reason)) => (r, reason),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn names_the_refused_segment_and_its_replacement_in_compound_commands() {
        let (rule, r) = deny(check("cd X && ls a b; cat src/a.rs; acus find foo"));
        assert_eq!(rule, "read-shell");
        assert!(r.starts_with("Nothing ran: the whole command was refused because of `cat src/a.rs`; the other segments are fine."), "{r}");
        assert!(r.contains("Replace it with `acus show 'src/a.rs'`"), "{r}");

        let (rule, r) = deny(check(
            "cmd1 && grep -n -i 'foo bar' src file.log | head && cmd3",
        ));
        assert_eq!(rule, "grep");
        assert!(
            r.contains("because of `grep -n -i 'foo bar' src file.log`"),
            "{r}"
        );
        assert!(
            r.contains("`acus find 'foo bar' 'src' 'file.log' -i`"),
            "{r}"
        );

        let (_, r) = deny(check("cd app && cargo test -q | tail -5 && echo done"));
        assert!(r.contains("because of `cargo test -q | tail -5`"), "{r}");
        assert!(r.contains("acus ctx 'cargo test -q'"), "{r}");

        let (_, r) = deny(check("cd a && head -n 20 src/a.rs"));
        assert!(r.contains("`acus show 'src/a.rs:1-20'`"), "{r}");
        let (_, r) = deny(check("cd a && sed -n '10,20p' src/a.rs"));
        assert!(r.contains("`acus show 'src/a.rs:10-20'`"), "{r}");
        let (_, r) = deny(check("true && cat a.rs b.rs"));
        assert!(r.contains("`acus show a.rs b.rs`"), "{r}");
        let (_, r) = deny(check("cd x && sed -i 's/a/b/' a.rs b.rs"));
        assert!(r.contains("`*** Replace All: a.rs b.rs`"), "{r}");
        let (_, r) = deny(check("cd x && rg -n --glob '*.rs' -w foo"));
        assert!(r.contains("`acus find 'foo' -g '*.rs' -w`"), "{r}");

        // The escape hint stays, and a lone command is not called compound.
        assert!(r.ends_with("`command ` prefix."), "{r}");
        let (_, r) = deny(check("cat src/a.rs"));
        assert!(!r.contains("Nothing ran"), "{r}");

        // Long segments are cut.
        let (_, r) = deny(check(&format!("cd x && cat {}.rs", "a".repeat(400))));
        assert!(r.contains('…') && r.len() < 700, "{r}");
    }

    #[test]
    fn leaves_data_files_plain_files_and_other_commands_output_alone() {
        // Decisions: JSON, TOML and YAML are data, read whole on purpose (the Read tool agrees);
        // searching a log or text file is not a code search; a pipe or stdin is another
        // command's output; `>`/`>>` make a read a copy or an append.
        for cmd in [
            "cat .claude/settings.local.json",
            "cat Cargo.toml",
            "head -20 config.yaml",
            "tail -n 5 data.yml",
            "grep version Cargo.toml",
            "grep -c error build.log",
            "grep -n TODO notes.txt out.csv",
            "grep -l foo *.log",
            "cmd | grep -c foo",
            "cmd | grep -l foo",
            "grep -c foo <(cmd)",
            "grep -q foo <<< \"$out\"",
            "grep -c foo < results.log",
            "grep -c foo",
            "cat a.rs >> all.txt",
            "cat a.rs b.rs > all.txt",
            "cat a.rs 1> all.txt",
            "cat > conf.json <<'EOF'\n{}\nEOF",
            "cat > notes.md <<'EOF'\ngrep foo src\nEOF",
            "git log --oneline 2>&1 | grep -c fix",
        ] {
            assert_eq!(check(cmd), None, "{cmd}");
        }
    }

    #[test]
    fn still_refuses_source_reads_and_searches_that_look_harmless() {
        for cmd in [
            "cat src/a.rs 2>&1",
            "cat src/a.rs 2>/dev/null",
            "cat settings.json src/a.rs",
            "grep foo src/a.rs",
            "grep -c foo README.md",
            "grep -rn foo .",
            "grep -l foo src",
            "grep -c foo *",
            "grep -c foo $file",
            "grep -c foo build.log src",
            "grep -rn foo",
            "rg foo",
            "rg foo notes.txt src",
            "sed -i '' 's/a/b/' config.json",
            "find . -name '*.rs' -exec grep -n foo {} +",
            "ls | xargs grep -c foo",
            "env FOO=1 grep -rn foo src",
        ] {
            assert!(matches!(check(cmd), Some(Verdict::Deny(..))), "{cmd}");
        }
    }

    #[test]
    fn recognises_windows_commands_in_either_shell() {
        for (cmd, rule) in [
            ("grep.exe -rn foo src", "grep"),
            (
                "'C:\\Program Files\\Git\\usr\\bin\\grep.exe' -n x a.rs",
                "grep",
            ),
            (r"C:\tools\rg.exe foo", "grep"),
            ("findstr /s /i foo *.rs", "grep"),
            (r#"findstr /n /c:"two words" src\a.rs"#, "grep"),
            (r"type src\a.rs", "read-shell"),
            (r"more src\a.rs", "read-shell"),
            (r"cd src && type a.rs b.rs", "read-shell"),
        ] {
            let Some(Verdict::Deny(r, _)) = check(cmd) else {
                panic!("{cmd}");
            };
            assert_eq!(r, rule, "{cmd}");
        }
        for cmd in [
            "findstr /i error build.log",
            r#"findstr /c:"a b" out.txt"#,
            r"type notes.txt",
            r"type package.json",
            "type a.rs > b.rs",
        ] {
            assert_eq!(check(cmd), None, "{cmd}");
        }
        assert!(matches!(check("dir /s"), Some(Verdict::Hint("tree", _))));
        let (_, r) = deny(check(r#"cd x && findstr /n /c:"two words" src\a.rs"#));
        assert!(r.contains(r"`acus find 'two words' 'src\a.rs' -F`"), "{r}");
    }

    #[test]
    fn covers_powershell_habits() {
        for (cmd, rule) in [
            (r"gc -Raw src\a.rs", "read-shell"),
            (r"Get-Content -TotalCount 5 .\src\a.rs", "read-shell"),
            (r"Get-Content -Path 'src\a.rs' -Tail 5", "read-shell"),
            ("Get-Content a.rs,b.txt", "read-shell"),
            (r"cat -Raw a.rs", "read-shell"),
            (r"type C:\repo\SRC\MAIN.RS", "read-shell"),
            (r"sls foo src\*.rs", "grep"),
            (r"Select-String -Pattern 'a b' -Path a.rs,b.rs", "grep"),
            (r"& 'C:\tools\rg.exe' foo src", "grep"),
            (r"findstr /s foo *.swift", "grep"),
            (r"gci -Recurse -Filter *.rs | Select-String foo", "grep"),
            (r"Get-ChildItem -Recurse | sls foo", "grep"),
            (r"ls -r src | Select-String -Pattern foo", "grep"),
            (r"dir *.rs | sls foo", "grep"),
            (
                r"(Get-Content a.rs) -creplace 'x','y' | sc a.rs",
                "ps-replace",
            ),
            (
                r"(gc a.rs -Raw).Replace('x','y') | Out-File a.rs",
                "ps-replace",
            ),
            (r"(Get-Content a.rs) -replace 'x','y' > a.rs", "ps-replace"),
            (
                r"[IO.File]::WriteAllText('a.rs', [IO.File]::ReadAllText('a.rs').Replace('x','y'))",
                "ps-replace",
            ),
            (r#"[System.IO.File]::ReadAllText("src\a.rs")"#, "read-shell"),
            ("cargo build 2>&1 | select -First 20", "build-pipe"),
        ] {
            let Some(Verdict::Deny(r, reason)) = check_ps(cmd) else {
                panic!("{cmd}");
            };
            assert_eq!(r, rule, "{cmd}");
            assert!(reason.contains("# acus-skip"), "{cmd}: {reason}");
        }
        for cmd in [
            "Get-Content app.log -Tail 20 -Wait",
            "Get-Content settings.json -Raw",
            "Get-Content out.txt | Select-String error",
            "Select-String error build.log",
            "Get-ChildItem -Recurse -Include *.log | Select-String error",
            "gci *.txt | sls foo",
            "gci -Name | sls foo",
            "ls | Select-Object -First 3",
            "Write-Host 'cat a.rs; sls x a.rs'",
            "Set-Content notes.txt 'hello'",
            r"(Get-Content notes.txt) -replace 'x','y' | Set-Content notes.txt",
            r"[IO.File]::ReadAllText('data.json')",
        ] {
            assert_eq!(check_ps(cmd), None, "{cmd}");
        }
        for cmd in [
            "ls -r src",
            "gci -Rec",
            "Get-ChildItem -Recurse -Depth 2",
            "dir /s",
        ] {
            assert!(
                matches!(check_ps(cmd), Some(Verdict::Hint("tree", _))),
                "{cmd}"
            );
        }
        // `ls -r` reverses in Bash.
        assert_eq!(check("ls -r src"), None);
    }

    #[test]
    fn powershell_separators_and_quoting_split_commands_the_same() {
        // `;` everywhere, `&&` and `||` (PowerShell 7; Windows PowerShell 5.1 rejects them but
        // the guard still sees the parts), backslashes are paths, quotes hide separators.
        for cmd in [
            r"cd src; Select-String foo a.rs",
            r"cd src && Select-String foo a.rs",
            r"cd src || Select-String foo a.rs",
            r"ls src\; Select-String foo src\a.rs",
            "Write-Host \"a`\";\" ; Get-Content a.rs",
            "Write-Host 'it''s'; Get-Content a.rs",
        ] {
            assert!(matches!(check_ps(cmd), Some(Verdict::Deny(..))), "{cmd}");
        }
        let (_, r) = deny(check_ps(r"cd C:\Repo; Get-Content Src\Main.rs; Get-Date"));
        assert!(r.contains(r"because of `Get-Content Src\Main.rs`"), "{r}");
        assert!(r.contains(r"`acus show 'Src\Main.rs'`"), "{r}");
        // The escape works on a refused segment of a compound command.
        assert_eq!(
            check_ps(r"cd src; Get-Content a.rs # acus-skip"),
            Some(Verdict::Escape("read-shell"))
        );
    }

    #[test]
    fn never_panics_on_odd_input() {
        for cmd in [
            "",
            " ",
            "\"",
            "'",
            "`",
            "\\",
            "&",
            "|",
            "||",
            "&&",
            ";",
            "(",
            ")",
            "<",
            ">",
            "<<<",
            "<<",
            "<<EOF",
            "grep",
            "grep -e",
            "grep --regexp",
            "grep --regexp=",
            "grep -",
            "grep --",
            "grep -A",
            "grep -efoo",
            "rg -g",
            "findstr",
            "findstr /",
            "findstr /c",
            "findstr /c:",
            "findstr /é",
            "findstr /éa:x",
            "sls -pattern",
            "sls -path",
            "sls -",
            "gci -filter",
            "gci | sls",
            "head -n",
            "head -é",
            "sed -n",
            "sed -n p",
            "cat >",
            "cat 2>",
            "xargs",
            "find -exec",
            "env",
            "uv run",
            "cd",
            "é",
            "日本語 grep 日本語.rs",
            "gc ,",
            "gc ,a.rs",
            "ls -",
            "ls -r | sls -",
            "'a b",
            "\"a b",
            "cat 'a.rs",
            "cat \"a.rs",
        ] {
            let _ = check(cmd);
            let _ = check_ps(cmd);
            let _ = check_ps(&format!("cd x; {cmd}; {cmd} | {cmd}"));
        }
    }

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
