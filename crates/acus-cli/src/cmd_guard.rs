//! PreToolUse hook for Claude Code: refuses shell searches, reads and in-place edits of source
//! files that acus does in one call, and tells the agent which acus command to use instead.

use crate::Outcome;
use anyhow::Result;
use regex::Regex;
use std::io::Read;
use std::sync::LazyLock;

#[derive(clap::Args)]
pub struct Args {}

pub fn run(_: Args) -> Result<Outcome> {
    let mut input = String::new();
    std::io::stdin().read_to_string(&mut input)?;
    let v: serde_json::Value = serde_json::from_str(&input).unwrap_or_default();
    let out = match check(v["tool_input"]["command"].as_str().unwrap_or_default()) {
        Some(Verdict::Deny(reason)) => serde_json::json!({ "hookSpecificOutput": {
            "hookEventName": "PreToolUse",
            "permissionDecision": "deny",
            "permissionDecisionReason": reason,
        }}),
        // The command runs as usual; only the agent's context gains the hint.
        Some(Verdict::Hint(hint)) => serde_json::json!({ "hookSpecificOutput": {
            "hookEventName": "PreToolUse",
            "additionalContext": hint,
        }}),
        None => return Ok(Outcome::Found),
    };
    println!("{out}");
    Ok(Outcome::Found)
}

#[derive(Debug, PartialEq)]
enum Verdict {
    /// Refuse the command; the reason names the acus command to use.
    Deny(String),
    /// Run the command, but tell the agent about the acus alternative.
    Hint(String),
}

const ESCAPE: &str = "Only if acus cannot do this, rerun with a `command ` prefix.";

const CODE: &[&str] = &[
    "rs", "swift", "ts", "tsx", "mts", "cts", "js", "jsx", "mjs", "cjs", "svelte", "vue", "py",
    "pyi", "md", "toml", "json", "yaml", "yml", "go", "c", "h", "cc", "cpp", "hpp", "m", "java",
    "kt", "rb", "sh", "zsh", "css", "scss", "html", "lua", "cs", "sql",
];

/// What to do with `cmd`: refusing wins over hinting, `None` lets it run.
fn check(cmd: &str) -> Option<Verdict> {
    let (rest, docs) = strip_heredocs(cmd);
    let scripts = docs.iter().filter_map(|(opener, body)| {
        let interp = segments(opener).iter().find_map(|(seg, _)| {
            head(seg)
                .map(|(h, _)| h.to_owned())
                .filter(|h| is_interpreter(h))
        })?;
        is_edit_script(body).then(|| script_hint(&interp))
    });
    let verdicts: Vec<Verdict> = segments(&rest)
        .into_iter()
        .filter_map(|(seg, piped)| rule(&seg, piped))
        .chain(scripts)
        .collect();
    let deny = verdicts.iter().position(|v| matches!(v, Verdict::Deny(_)));
    verdicts.into_iter().nth(deny.unwrap_or(0))
}

fn is_interpreter(head: &str) -> bool {
    head.starts_with("python") || matches!(head, "node" | "ruby" | "bun" | "deno")
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

/// Edit scripts are only hinted at: telling them apart from data processing is a heuristic.
fn script_hint(interp: &str) -> Verdict {
    Verdict::Hint(format!(
        "acus hint: this {interp} script edits source files. Next time use one `acus patch` with SEARCH/REPLACE blocks or `*** Replace All: PATHS` (literal or REGEX): it edits every file at once, all or nothing, and prints the written lines."
    ))
}

/// The command word of a simple command, past assignments and keywords, and its arguments.
fn head(seg: &str) -> Option<(&str, impl Iterator<Item = &str>)> {
    let mut words = seg.split_whitespace().skip_while(|w| {
        is_assignment(w) || matches!(*w, "do" | "then" | "else" | "{" | "!" | "time" | "sudo")
    });
    let mut head = words.next()?.rsplit('/').next()?;
    if matches!(head, "uv" | "env" | "exec") {
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
    let code_file = args.iter().any(|a| {
        let a = a.trim_matches(['\'', '"']);
        !a.starts_with('-') && a.rsplit_once('.').is_some_and(|(_, e)| CODE.contains(&e))
    });
    let reason = match head {
        "grep" | "rg" | "egrep" | "fgrep" | "ag" | "ack" if !piped => format!(
            "Use acus instead of {head}: `acus find 'PAT' [PATH…]` (`--block` adds the enclosing code, `-l` lists files, `-w`, `-t TYPE`, `-g GLOB`, `-u --hidden` include ignored and hidden files). {ESCAPE}"
        ),
        "sed" | "perl" if in_place => format!(
            "Use acus instead of {head} -i: one `acus patch` with `*** Replace All: PATHS` or SEARCH/REPLACE blocks edits every file at once and prints the written lines. {ESCAPE}"
        ),
        "sed" | "cat" | "head" | "tail" | "nl" | "bat" | "less"
            if !piped
                && code_file
                && !seg.contains('>')
                && (head != "sed" || args.contains(&"-n"))
                && !args.iter().any(|a| matches!(*a, "-f" | "-F")) =>
        {
            format!(
                "Use acus instead of {head}: `acus show path path:A-B path#Symbol` reads several files, ranges or symbols in one call; `acus outline PATH` gives an overview. {ESCAPE}"
            )
        }
        _ => return None,
    };
    Some(Verdict::Deny(reason))
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
    use super::{Verdict, check};

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
        ] {
            assert!(matches!(check(cmd), Some(Verdict::Deny(_))), "{cmd}");
        }
    }

    #[test]
    fn hints_at_edit_scripts() {
        for cmd in [
            "python3 - <<'EOF'\np='src/a.rs'\ns=open(p).read()\nopen(p,'w').write(s.replace('a','b'))\nEOF",
            "cd app && uv run python - <<'E'\nimport re, pathlib\nf=pathlib.Path(\"Sources/A.swift\")\nf.write_text(re.sub(r'x','y',f.read_text()))\nE",
            "python3 -c \"p='lib.py'; s=open(p).read(); open(p,'w').write(s.replace('a','b'))\"",
            "node -e \"const fs=require('fs');fs.writeFileSync('a.ts',fs.readFileSync('a.ts','utf8').replace('a','b'))\"",
        ] {
            assert!(matches!(check(cmd), Some(Verdict::Hint(_))), "{cmd}");
        }
    }

    #[test]
    fn allows_filters_data_and_escapes() {
        for cmd in [
            "git log --oneline | grep fix",
            "cargo test 2>&1 | grep -E 'FAILED|panicked'",
            "git commit -m \"fix; grep foo\"",
            "command grep -rn foo src",
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
            "cat > gen.py <<'EOF'\nopen('a.py','w').write(s.replace('a','b'))\nEOF",
        ] {
            assert_eq!(check(cmd), None, "{cmd}");
        }
    }
}
