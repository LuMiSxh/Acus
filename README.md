<div align="center">

# Acus

**Code search, outline and editing for coding agents.** One call where an agent would otherwise chain `rg`, `cat` and `sed -n`, with output small enough to keep the context window clean.

![Rust](https://img.shields.io/badge/rust-2024-orange.svg)
![macOS](https://img.shields.io/badge/macOS-primary-black.svg)
![Windows](https://img.shields.io/badge/Windows-CI-lightgrey.svg)
[![License](https://img.shields.io/badge/license-MPL--2.0-blue.svg)](LICENSE)

*rem acu tetigisti*: you hit it with the needle.

</div>

## Why

Coding agents spend a surprising share of their turns finding code: a search, then a read of the file, then a second read because the first window was too small. Every step costs a round trip and puts lines into the context that nobody needed.

Acus answers that chain in one call. Search hits come grouped by the function or type they sit in, `--block` prints those bodies right away, and symbols can be addressed by name instead of by guessed line numbers:

```console
$ acus find 'fn spawn_blocking' --block
$ acus show src/runtime/handle.rs#Handle::spawn_blocking src/lib.rs:40-80
```

Whenever output is cut short, the last line names the exact command that shows the rest.

## Install

Rust 1.88 or newer:

```sh
cargo install --path crates/acus-cli
```

`acus decide` is behind a feature flag, so add `--features cmd-decide` if you want it.

To teach Claude Code and Codex when to reach for it, copy the skill:

```sh
mkdir -p ~/.claude/skills/acus ~/.agents/skills/acus
cp skill/SKILL.md ~/.claude/skills/acus/
cp skill/SKILL.md ~/.agents/skills/acus/
```

## Commands

| Command | What it does |
| --- | --- |
| `acus find PATTERN [PATH...]` | Regex search, hits grouped by enclosing symbol; `--block` adds the bodies |
| `acus outline PATH...` | Symbols of a file or directory with kinds and line ranges |
| `acus show ADDR...` | Symbols, line ranges or whole files, numbered |
| `acus patch` | Applies a patch from stdin, all or nothing, and prints the written lines |
| `acus ctx 'COMMAND'` | Runs a build or test, drops progress noise, colour codes, passing tests and library stack frames, and shows the code behind every `path:line` in its output, each function once |
| `acus diff [REV] [PATH...]` | Uncommitted changes per function or type, with untracked files; `-p` adds the lines, `--staged` and `A..B` work as in git |
| `acus run` | Several of the above in one process, one command line per stdin line (or a JSON list of argument lists) |
| `acus usage` | Token, cost and tool-call statistics from Claude Code and Codex transcripts; `--tools` shows how agents search, read, edit and build (acus vs grep, sed, python, built-ins) |
| `acus decide QUESTION` | Yes/no, choice or score answer from a Jev-compatible API |

Addresses look like `path#Type::method`, `path:10-40`, `path:10` or just `path`. A unique suffix such as `#method` is enough.

Exit codes are 0 for results, 1 for nothing found and 2 for errors. Errors carry a `hint:` line. `ctx` passes on the exit code of the command it ran.

Output defaults to a compact agent format. `--json` and `--human` are there for scripts and people.

Symbols are understood for Rust, Swift, TypeScript and JavaScript, Svelte, Python, Markdown headings and TOML, JSON and YAML keys. Every other file still works with `find` and line ranges.

## Patches

`acus patch` reads the Codex `apply_patch` format and adds `*** Replace Symbol:`, which swaps out a whole function or type by name:

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

Context lines are matched exactly first, then without trailing whitespace, then without any surrounding whitespace. An `@@` anchor can be any part of a line, and the context may start on the anchor line itself. If a hunk matches in more than one place, the patch is refused and the error lists the candidate lines.

Agents that would otherwise write a `sed -i` line or a small Python script get a few more directives:

```
*** Update File: src/lib.rs
<<<<<<< SEARCH
    let x = 1;
=======
    let x = 2;
>>>>>>> REPLACE
*** Replace All: src *.py
<<<<<<< SEARCH
old_name(
=======
new_name(
>>>>>>> REPLACE
<<<<<<< REGEX
fn (\w+)_old\(
=======
fn ${1}_new(
>>>>>>> REPLACE
*** Delete Symbol: src/lib.rs#unused
*** Move Symbol: src/lib.rs#helper
*** After: src/util.rs#main
```

SEARCH/REPLACE blocks are the format Aider made popular and can stand in for `-`/`+` lines inside any `Update File`. `Replace All` changes every occurrence in the given files, directories and globs, and fails if a block matches nowhere. `Delete Symbol` and `Move Symbol` carry doc comments and attributes along; a move goes `*** Before:` or `*** After:` another symbol, or `*** To:` the end of a file, which is created if needed.

Nothing is written unless every hunk applies. Files are replaced atomically. `--check` only validates. `--fmt` (or `[patch] fmt = true`) runs each written file through its formatter if one is installed (rustfmt with the crate's edition, ruff or black, the project's prettier, swift-format, gofmt) and prints the formatted lines, so line numbers and context stay true for the next patch.

## Numbers

Measured with [hyperfine](https://github.com/sharkdp/hyperfine) on an Apple M4. The small corpus is tokio (564 files), the large one a full Cargo registry (80,197 files, 1.8 GB).

| Task | acus | Compared with |
| --- | --- | --- |
| Literal search without hits, tokio | 9.6 ms | rg 10.1 ms, grep 20.0 ms |
| Literal search without hits, registry | 2.08 s | rg 2.21 s, grep 6.66 s |
| Regex search with hits, registry | 1.60 s | rg 1.64 s |
| Find a definition plus its body, tokio | 9.8 ms | ast-grep 54.5 ms |
| Outline of every file, tokio | 81 ms | ctags 67 ms |
| Outline of every Rust file, registry | 15.7 s | ctags 20.0 s |

What the agent has to read matters more than milliseconds. For one function in tokio:

| Approach | Bytes |
| --- | --- |
| `acus show handle.rs#Handle::spawn_blocking` | 384 |
| `acus outline handle.rs` | 974 |
| `acus find --block` (all hits with bodies) | 5,132 |
| `cat handle.rs` | 26,550 |
| `rg -C20` (hits with guessed context) | 58,501 |

Criterion benchmarks for the core paths live in `crates/acus-cli/benches`.

## Configuration

`config.toml` is read from `$ACUS_CONFIG`, otherwise from `~/Library/Application Support/acus/` on macOS, `%APPDATA%\acus\` on Windows and `$XDG_CONFIG_HOME/acus/` elsewhere.

```toml
[commands]
disabled = ["usage"]

[ctx]
drop = ["^warning: unused"]                         # extra output lines acus ctx hides

[patch]
fmt = true                                          # as if every patch had --fmt

[decide]
enabled = true
url = "https://openrouter.ai/api/alpha/decisions"   # or https://api.typesafe.ai/v1/systemone
model = "~typesafe/jev-latest"                      # "jev-latest" when talking to TypeSafe directly
api_key_env = "OPENROUTER_API_KEY"                  # TYPESAFE_API_KEY when talking to TypeSafe directly
```

`acus ctx --drop REGEX` does the same for a single run. The other settings can also come from the environment: `ACUS_DISABLE=usage,decide`, `ACUS_DECIDE_ENABLED`, `ACUS_DECIDE_URL`, `ACUS_DECIDE_MODEL` and `ACUS_DECIDE_API_KEY_ENV`.

Language support is compiled in through the features `lang-rust`, `lang-swift`, `lang-ts`, `lang-python`, `lang-markdown`, `lang-svelte` and `lang-data`, all on by default. Leave some out for a smaller binary.

## Development

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
cargo bench -p acus-cli
```

CI runs the same checks on Ubuntu and Windows.

| Crate | Role |
| --- | --- |
| `acus-cli` | The `acus` binary, output formats and the `run` batch mode |
| `acus-walk` | Parallel file walking that respects `.gitignore`, plus globs |
| `acus-search` | Search with enclosing-symbol context, built on the ripgrep crates |
| `acus-syntax` | tree-sitter outlines and symbol lookup |
| `acus-edit` | Patch parsing and atomic application |
| `acus-usage` | Transcript statistics |
| `acus-decide` | Client for Jev-compatible decision APIs |

## License

[MPL-2.0](LICENSE).

Copyright © 2026 LuMiSxh. Claude Code, Codex and Jev belong to their respective owners.
