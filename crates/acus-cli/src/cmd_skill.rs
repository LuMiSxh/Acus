use crate::Outcome;
use anyhow::{Context, Result};

/// The skill shipped with this binary, so hook output and installed copies match its version.
const SKILL: &str = include_str!("../../../skill/SKILL.md");

#[derive(clap::Args)]
pub struct Args {
    /// Write SKILL.md to ~/.claude/skills/acus/ and ~/.agents/skills/acus/ instead of printing it.
    #[arg(long)]
    install: bool,
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
    Ok(Outcome::Found)
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
    #[test]
    fn body_drops_frontmatter() {
        assert_eq!(super::body("---\nname: x\n---\n\n# acus\n"), "# acus\n");
        assert_eq!(super::body("# no frontmatter\n"), "# no frontmatter\n");
        assert!(super::body(super::SKILL).starts_with("# acus"));
    }
}
