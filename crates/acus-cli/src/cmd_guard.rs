//! PreToolUse hook for Claude Code: refuses shell searches, reads and in-place edits of source
//! files that acus does in one call, and tells the agent which acus command to use instead.

use crate::Outcome;
use anyhow::Result;
use regex::Regex;
use std::io::Read;

#[derive(clap::Args)]
pub struct Args {}

pub fn run(_: Args) -> Result<Outcome> {
    let mut input = String::new();
    std::io::stdin().read_to_string(&mut input)?;
    let v: serde_json::Value = serde_json::from_str(&input).unwrap_or_default();
    if let Some(reason) = check(v["tool_input"]["command"].as_str().unwrap_or_default()) {
        let out = serde_json::json!({ "hookSpecificOutput": {
            "hookEventName": "PreToolUse",
            "permissionDecision": "deny",
            "permissionDecisionReason": reason,
        }});
        println!("{out}");
    }
    Ok(Outcome::Found)
}

const ESCAPE: &str = "Only if acus cannot do this, rerun with a `command ` prefix.";

const CODE: &[&str] = &[
    "rs", "swift", "ts", "tsx", "mts", "cts", "js", "jsx", "mjs", "cjs", "svelte", "vue", "py",
    "pyi", "md", "toml", "json", "yaml", "yml", "go", "c", "h", "cc", "cpp", "hpp", "m", "java",
    "kt", "rb", "sh", "zsh", "css", "scss", "html", "lua", "cs", "sql",
];

/// The reason to refuse `cmd`, or `None` to let it run.
fn check(cmd: &str) -> Option<String> {
    segments(&strip_heredocs(cmd))
        .into_iter()
        .find_map(|(seg, piped)| rule(&seg, piped))
}

fn rule(seg: &str, mut piped: bool) -> Option<String> {
    let mut words = seg.split_whitespace().skip_while(|w| {
        is_assignment(w) || matches!(*w, "do" | "then" | "else" | "{" | "!" | "time" | "sudo")
    });
    let mut head = words.next()?.rsplit('/').next()?;
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
    match head {
        "grep" | "rg" | "egrep" | "fgrep" | "ag" | "ack" if !piped => Some(format!(
            "Use acus instead of {head}: `acus find 'PAT' [PATH…]` (`--block` adds the enclosing code, `-l` lists files, `-w`, `-t TYPE`, `-g GLOB`, `-u --hidden` include ignored and hidden files). {ESCAPE}"
        )),
        "sed" | "perl" if in_place => Some(format!(
            "Use acus instead of {head} -i: one `acus patch` with `*** Replace All: PATHS` or SEARCH/REPLACE blocks edits every file at once and prints the written lines. {ESCAPE}"
        )),
        "sed" | "cat" | "head" | "tail" | "nl" | "bat" | "less"
            if !piped
                && code_file
                && !seg.contains('>')
                && (head != "sed" || args.contains(&"-n"))
                && !args.iter().any(|a| matches!(*a, "-f" | "-F")) =>
        {
            Some(format!(
                "Use acus instead of {head}: `acus show path path:A-B path#Symbol` reads several files, ranges or symbols in one call; `acus outline PATH` gives an overview. {ESCAPE}"
            ))
        }
        _ => None,
    }
}

fn is_assignment(w: &str) -> bool {
    w.split_once('=').is_some_and(|(k, _)| {
        !k.is_empty() && k.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
    })
}

/// Drops heredoc bodies, which are data rather than commands.
fn strip_heredocs(cmd: &str) -> String {
    let start = Regex::new(r#"<<-?\s*['"]?([A-Za-z_][A-Za-z0-9_]*)['"]?"#).unwrap();
    let mut out = String::new();
    let mut end: Option<String> = None;
    for line in cmd.lines() {
        if let Some(d) = &end {
            if line.trim() == d {
                end = None;
            }
            continue;
        }
        end = start.captures(line).map(|c| c[1].to_owned());
        out.push_str(line);
        out.push('\n');
    }
    out
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
    use super::check;

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
        ] {
            assert!(check(cmd).is_some(), "{cmd}");
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
        ] {
            assert_eq!(check(cmd), None, "{cmd}");
        }
    }
}
