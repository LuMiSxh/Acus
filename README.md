# Acus

Agent-first code search, outline and editing CLI — fewer round trips, fewer tokens.

*Acus* is Latin for "needle" (*rem acu tetigisti* — "you hit it with the needle").

## Install

```bash
cargo install --path crates/acus-cli
```

## Commands

| Command | Status | Purpose |
|---|---|---|
| `acus find PATTERN [PATH…]` | phase 1 | Regex search, hits grouped by enclosing symbol; `--block` prints symbol bodies |
| `acus outline PATH…` | phase 1 | Symbols with kinds and line ranges |
| `acus show ADDR…` | phase 1 | Print symbols, line ranges or files with line numbers |
| `acus patch` | planned | Apply patches (stdin), all-or-nothing, atomic writes |
| `acus usage` | planned | Claude Code / Codex transcript analytics |
| `acus decide` | planned | Typed judgements via a Jev-compatible API (opt-in feature) |
| `acus run` | planned | Several commands in one invocation |

Addresses: `path#Qual::Name`, `path:START-END`, `path:LINE`, `path`.
Output: `--format agent` (default), `--json`, `--human`.
Exit codes: 0 results, 1 nothing found, 2 error.

## Licence

MPL-2.0
