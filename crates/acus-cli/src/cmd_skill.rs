use crate::Outcome;
use anyhow::{Context, Result};
use serde_json::{Value, json};
use std::path::Path;

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
}

pub fn run(a: Args) -> Result<Outcome> {
    if !a.install {
        print!("{}", body(SKILL));
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
