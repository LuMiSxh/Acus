use crate::Outcome;
use acus_syntax::Lang;
use acus_walk::{WalkOpts, walk};
use anyhow::{Context, Result};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::sync::Mutex;

/// The skill shipped with this binary, so hook output and installed copies match its version.
const SKILL: &str = include_str!("../../../skill/SKILL.md");

#[derive(clap::Args)]
pub struct Args {
    /// Write SKILL.md to ~/.claude/skills/acus/ and ~/.agents/skills/acus/ instead of printing it.
    #[arg(long)]
    install: bool,
    /// With --install: also register the SessionStart and PreToolUse hooks in ~/.claude/settings.json.
    #[arg(long, requires = "install")]
    hooks: bool,
    /// After the skill, print a compact project map (only inside a git checkout).
    #[arg(long, conflicts_with = "install")]
    map: bool,
}

pub fn run(a: Args) -> Result<Outcome> {
    if !a.install {
        print!("{}", body(SKILL));
        if a.map {
            let cwd = std::env::current_dir()?;
            // Without a repository the cwd may be a directory full of projects: nothing useful to map.
            if cwd.ancestors().any(|d| d.join(".git").exists()) {
                print_map()?;
            }
        }
        return Ok(Outcome::Found);
    }
    let home = std::env::home_dir().context("no home directory")?;
    for dir in [".claude/skills/acus", ".agents/skills/acus"] {
        let dir = home.join(dir);
        std::fs::create_dir_all(&dir)
            .with_context(|| format!("cannot create {}", dir.display()))?;
        let file = dir.join("SKILL.md");
        std::fs::write(&file, SKILL).with_context(|| format!("cannot write {}", file.display()))?;
        println!("{}", file.display());
    }
    if a.hooks {
        install_hooks(&home.join(".claude/settings.json"))?;
    }
    Ok(Outcome::Found)
}

const MAP_LINES: usize = 40;

/// Source files under the cwd with their line counts, as a map for the session context.
fn print_map() -> Result<()> {
    let found = Mutex::new(Vec::new());
    let opts = WalkOpts {
        roots: vec![".".into()],
        include: vec![],
        exclude: vec![],
        hidden: false,
        no_ignore: false,
    };
    walk(&opts, |f| {
        if Lang::from_path(f).is_some()
            && let Ok(b) = std::fs::read(f)
        {
            let n = b.iter().filter(|&&c| c == b'\n').count() + usize::from(!b.ends_with(b"\n"));
            let path = acus_walk::display_path(f);
            found.lock().unwrap().push((path, n));
        }
    })?;
    let files = found.into_inner().unwrap();
    if !files.is_empty() {
        println!("\n## Project map\n");
        println!("`acus outline DIR --depth 1` lists a directory's symbols.\n");
        for l in map_lines(&files, MAP_LINES) {
            println!("{l}");
        }
    }
    Ok(())
}

/// One line per directory (`dir/  N files, L lines (rs, md)`, top level as `./`), sorted by
/// path. Past `cap` lines only the directories with the most code stay.
fn map_lines(files: &[(String, usize)], cap: usize) -> Vec<String> {
    let mut dirs: BTreeMap<&str, (usize, usize, BTreeSet<&str>)> = BTreeMap::new();
    for (path, lines) in files {
        let (dir, name) = path.rsplit_once('/').unwrap_or((".", path));
        let e = dirs.entry(dir).or_default();
        e.0 += 1;
        e.1 += lines;
        if let Some((_, ext)) = name.rsplit_once('.') {
            e.2.insert(ext);
        }
    }
    let mut rows: Vec<_> = dirs.into_iter().collect();
    let more = rows.len().saturating_sub(cap);
    if more > 0 {
        let mut by_size: Vec<_> = rows.iter().map(|r| (r.1.1, r.0)).collect();
        by_size.sort_by_key(|&(lines, _)| std::cmp::Reverse(lines));
        let keep: BTreeSet<_> = by_size.into_iter().take(cap).map(|(_, d)| d).collect();
        rows.retain(|r| keep.contains(r.0));
    }
    let mut v: Vec<String> = rows
        .iter()
        .map(|(dir, (n, lines, exts))| {
            let exts: Vec<_> = exts.iter().copied().collect();
            let files = if *n == 1 { "file" } else { "files" };
            format!("{dir}/  {n} {files}, {lines} lines ({})", exts.join(", "))
        })
        .collect();
    if more > 0 {
        v.push(format!(
            "… {more} more directories (acus outline DIR --depth 1)"
        ));
    }
    v
}

/// Registers the acus hooks in a Claude Code settings file, keeping a `.bak` of the old one.
fn install_hooks(path: &Path) -> Result<()> {
    let old = match std::fs::read_to_string(path) {
        Ok(t) => Some(t),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return Err(e).with_context(|| format!("cannot read {}", path.display())),
    };
    let settings = match &old {
        Some(t) => serde_json::from_str(t)
            .with_context(|| format!("{} is not valid JSON; fix it first", path.display()))?,
        None => json!({}),
    };
    if let Some(t) = &old {
        let bak = path.with_extension("json.bak");
        std::fs::write(&bak, t).with_context(|| format!("cannot write {}", bak.display()))?;
    } else if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut text = serde_json::to_string_pretty(&merge_hooks(settings))?;
    text.push('\n');
    std::fs::write(path, text).with_context(|| format!("cannot write {}", path.display()))?;
    println!("{}", path.display());
    Ok(())
}

/// Adds the acus SessionStart and PreToolUse hooks; earlier acus entries are replaced, so a
/// rerun never duplicates them and everything else in the settings stays as it was.
fn merge_hooks(mut settings: Value) -> Value {
    const OURS: [(&str, &str, &str); 2] = [
        (
            "SessionStart",
            "startup|clear|compact",
            "acus skill 2>/dev/null || true",
        ),
        (
            "PreToolUse",
            "Bash|Read|Grep|Glob|PowerShell",
            "acus guard 2>/dev/null || true",
        ),
    ];
    if !settings.is_object() {
        settings = json!({});
    }
    let hooks = settings
        .as_object_mut()
        .unwrap()
        .entry("hooks")
        .or_insert_with(|| json!({}));
    if !hooks.is_object() {
        *hooks = json!({});
    }
    let hooks = hooks.as_object_mut().unwrap();
    let ours = |c: &Value| {
        c.as_str()
            .is_some_and(|c| c.contains("acus skill") || c.contains("acus guard"))
    };
    for groups in hooks.values_mut().filter_map(Value::as_array_mut) {
        groups.retain_mut(|g| {
            let Some(items) = g.get_mut("hooks").and_then(Value::as_array_mut) else {
                return true;
            };
            let before = items.len();
            items.retain(|i| !ours(&i["command"]));
            !items.is_empty() || before == 0
        });
    }
    for (event, matcher, command) in OURS {
        let group = json!({"matcher": matcher, "hooks": [{"type": "command", "command": command}]});
        match hooks.entry(event).or_insert_with(|| json!([])) {
            Value::Array(a) => a.push(group),
            other => *other = json!([group]),
        }
    }
    settings
}

/// The skill without its YAML frontmatter, for injecting as session context.
fn body(s: &str) -> &str {
    let mut lines = s.split_inclusive('\n');
    let Some(first) = lines.next().filter(|l| l.trim_end() == "---") else {
        return s;
    };
    let mut end = first.len();
    for l in lines {
        end += l.len();
        if l.trim_end() == "---" {
            return s[end..].trim_start_matches(['\r', '\n']);
        }
    }
    s
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    #[test]
    fn map_groups_by_directory() {
        let f = |p: &str, n| (p.to_string(), n);
        let files = [
            f("src/a.rs", 10),
            f("src/b.rs", 5),
            f("README.md", 3),
            f("src/c.md", 2),
        ];
        assert_eq!(
            super::map_lines(&files, 40),
            [
                "./  1 file, 3 lines (md)",
                "src/  3 files, 17 lines (md, rs)"
            ]
        );
    }

    #[test]
    fn map_keeps_largest_directories() {
        let files: Vec<_> = (0..5).map(|i| (format!("d{i}/a.rs"), i + 1)).collect();
        assert_eq!(
            super::map_lines(&files, 2),
            [
                "d3/  1 file, 4 lines (rs)",
                "d4/  1 file, 5 lines (rs)",
                "… 3 more directories (acus outline DIR --depth 1)"
            ]
        );
    }

    fn count(v: &serde_json::Value, event: &str) -> usize {
        v["hooks"][event].as_array().map_or(0, Vec::len)
    }

    #[test]
    fn hooks_into_empty_settings() {
        let v = super::merge_hooks(json!({}));
        assert_eq!(
            v["hooks"]["SessionStart"],
            json!([{"matcher": "startup|clear|compact", "hooks": [{"type": "command", "command": "acus skill 2>/dev/null || true"}]}])
        );
        assert_eq!(
            v["hooks"]["PreToolUse"][0]["matcher"],
            "Bash|Read|Grep|Glob|PowerShell"
        );
    }

    #[test]
    fn hooks_keep_unrelated_and_are_idempotent() {
        let start = json!({"model": "x", "hooks": {"PreToolUse": [
            {"matcher": "Edit", "hooks": [{"type": "command", "command": "fmt.sh"}]}
        ], "Stop": [{"hooks": []}]}, "theme": "dark"});
        let once = super::merge_hooks(start);
        assert_eq!(super::merge_hooks(once.clone()), once);
        assert_eq!(count(&once, "PreToolUse"), 2);
        assert_eq!(
            once["hooks"]["PreToolUse"][0]["hooks"][0]["command"],
            "fmt.sh"
        );
        assert_eq!(once["hooks"]["Stop"], json!([{"hooks": []}]));
        let keys: Vec<_> = once.as_object().unwrap().keys().collect();
        assert_eq!(keys, ["model", "hooks", "theme"]);
    }

    #[test]
    fn hooks_replace_old_guard_entry() {
        let start = json!({"hooks": {"PreToolUse": [
            {"matcher": "Bash", "hooks": [{"type": "command", "command": "acus guard"}]},
            {"matcher": "Bash", "hooks": [
                {"type": "command", "command": "acus guard"},
                {"type": "command", "command": "keep.sh"}
            ]}
        ]}});
        let v = super::merge_hooks(start);
        let g = v["hooks"]["PreToolUse"].as_array().unwrap();
        assert_eq!(g.len(), 2);
        assert_eq!(
            g[0]["hooks"],
            json!([{"type": "command", "command": "keep.sh"}])
        );
        assert_eq!(g[1]["matcher"], "Bash|Read|Grep|Glob|PowerShell");
    }
    #[test]
    fn body_drops_frontmatter() {
        assert_eq!(super::body("---\nname: x\n---\n\n# acus\n"), "# acus\n");
        assert_eq!(super::body("# no frontmatter\n"), "# no frontmatter\n");
        assert!(super::body(super::SKILL).starts_with("# acus"));
    }
}
