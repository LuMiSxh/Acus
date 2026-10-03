# Acus

Agent-first code search, outline and editing CLI — fewer round trips, fewer tokens.

*Acus* is Latin for "needle" (*rem acu tetigisti* — "you hit it with the needle").

## Install

```bash
cargo install --path crates/acus-cli
```

## Commands

| Command | Purpose |
|---|---|
| `acus find PATTERN [PATH…]` | Regex search, hits grouped by enclosing symbol; `--block` prints symbol bodies |
| `acus outline PATH…` | Symbols with kinds and line ranges |
| `acus show ADDR…` | Print symbols, line ranges or files with line numbers |
| `acus patch [--check] [-f FILE]` | Apply a patch from stdin: all-or-nothing, atomic writes, no temp files left behind |
| `acus usage [--days N] [--project P]` | Token, cost and tool-call analytics over Claude Code / Codex transcripts |
| `acus decide QUESTION` | Yes/no, choice or score judgement via a Jev-compatible API (opt-in feature) |
| `acus run` | Several commands in one invocation (JSON array of argument lists on stdin) |

Addresses: `path#Qual::Name`, `path:START-END`, `path:LINE`, `path`.
Output: `--format agent` (default), `--json`, `--human`.
Exit codes: 0 results, 1 nothing found, 2 error.

## Examples

```bash
acus find "fn parse" -g '*.rs' --block   # hits plus enclosing function bodies
acus outline src --depth 1               # top-level symbols of every file
acus show src/lib.rs#Parser::parse README.md#Usage src/main.rs:10-40
echo '[["outline","src"],["find","TODO"]]' | acus run
acus usage --days 7
```

### Patch format

Codex `apply_patch` plus symbol replacement. Context lines match exactly, then ignoring
trailing and finally surrounding whitespace; ambiguous matches are rejected with their line numbers.

```
*** Begin Patch
*** Update File: src/lib.rs
@@ fn parse
-        let x = 1;
+        let x = 2;
*** Replace Symbol: src/lib.rs#Parser::reset
+    fn reset(&mut self) {
+        self.pos = 0;
+    }
*** Add File: notes.md
+hello
*** Delete File: old.rs
*** End Patch
```

## Configuration

`config.toml` at `$ACUS_CONFIG`, otherwise `~/Library/Application Support/acus/` (macOS),
`%APPDATA%\acus\` (Windows) or `$XDG_CONFIG_HOME/acus/` (`~/.config/acus/`).

```toml
[commands]
disabled = ["usage"]

[decide]
enabled = true
url = "https://api.typesafe.ai/v1/systemone"
model = "jev-latest"
api_key_env = "TYPESAFE_API_KEY"
```

Environment overrides: `ACUS_DISABLE=usage,decide`, `ACUS_DECIDE_ENABLED`, `ACUS_DECIDE_URL`,
`ACUS_DECIDE_MODEL`, `ACUS_DECIDE_API_KEY_ENV`.

Compile-time features (`crates/acus-cli`): `lang-rust`, `lang-swift`, `lang-ts`, `lang-python`,
`lang-markdown`, `lang-svelte`, `lang-data` (default on) and `cmd-decide` (off):

```bash
cargo install --path crates/acus-cli --features cmd-decide
```

## Agent skill

```bash
mkdir -p ~/.agents/skills/acus ~/.claude/skills/acus
cp skill/SKILL.md ~/.agents/skills/acus/ && cp skill/SKILL.md ~/.claude/skills/acus/
```

## Licence

MPL-2.0
