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

macOS and Linux:

```sh
curl -fsSL https://raw.githubusercontent.com/LuMiSxh/Acus/main/install.sh | sh
```

Windows (PowerShell):

```powershell
irm https://raw.githubusercontent.com/LuMiSxh/Acus/main/install.ps1 | iex
```

Both scripts download the latest release to `~/.local/bin` (`ACUS_INSTALL_DIR` changes that), add it to `PATH` and run `acus skill --install`, which writes the agent skill to `~/.claude/skills/acus/` and `~/.agents/skills/acus/`. Set `ACUS_NO_SKILL=1` to skip the skill. Release binaries include `acus decide`.

From source, with Rust 1.88 or newer:

```sh
cargo install --path crates/acus-cli --features cmd-decide
acus skill --install
```

`acus update` replaces the binary where it is installed with the latest release when that is newer (checked against the release's SHA-256), then reinstalls the skill so it matches the binary; `--check` only reports. After a source build, run `acus skill --install` yourself. `acus skill --install --hooks` also sets up Claude Code (see below); on Windows this is the whole setup.

## Agent setup

An installed skill is only listed by its description, so agents often fall back to grep and cat. Load it into context, and let a hook steer the remaining habits.

**Claude Code.** One command registers both hooks in `~/.claude/settings.json` (keeping a `.bak` of the old file and everything else in it; a rerun replaces the acus entries instead of duplicating them):

```sh
acus skill --install --hooks
```

That adds:

```json
{
  "hooks": {
    "SessionStart": [
      {
        "matcher": "startup|clear|compact",
        "hooks": [{ "type": "command", "command": "acus skill 2>/dev/null || true" }]
      }
    ],
    "PreToolUse": [
      {
        "matcher": "Bash|Read|Grep|Glob|PowerShell",
        "hooks": [{ "type": "command", "command": "acus guard 2>/dev/null || true" }]
      }
    ]
  }
}
```

The SessionStart hook prints the skill at every start, `/clear` and compaction. There is deliberately no project map in it: [overviews in context files did not help agents find the relevant files and raised cost](https://arxiv.org/abs/2602.11988). Agents call `acus map` when they need the layout instead.

`acus guard` refuses what acus does better and names the acus command to use:

- Bash: `grep`/`rg` searches, `cat`/`head`/`tail`/`sed -n` reads of source files, `sed -i`/`perl -i` edits, and builds or tests piped into `tail`/`head`/`grep` (`cargo test | tail` hides failures and leads to reruns). Pipe filters on other output (`git log | grep fix`), `grep` of stdin, logs, text and other plain files, reads and searches of JSON, TOML and YAML data files, `cat a.rs >> all.txt` copies and heredoc bodies pass. Python, Node and Ruby scripts that rewrite a source file with `.replace()` or `re.sub()` still run, with a hint to use `acus patch`, because telling them apart from data processing is a heuristic.
- A refusal stops the whole command, so nothing in it runs. For a compound command (`cd x && ls; cat a.rs`) the reason names the refused segment, says that no part ran and shows the acus command to run in its place, so the retry is one step.
- PowerShell and Windows: the same rules for `Select-String` (`sls`), `findstr`, `Get-Content` (`gc`, `type`, `cat`, `-Raw`, `-TotalCount`, `-Tail`), `Get-ChildItem … | Select-String`, `Select-Object -Last` after a build, `.exe` and backslash command paths, `-replace`/`.Replace()` edits written with `Set-Content`, `sc`, `Out-File`, `>` or `WriteAllText`, and `ReadAllText` reads. Separators `;`, `&&` and `||` split commands in both PowerShell 5.1 and 7, and backslashes in PowerShell are path characters.
- Read: without `offset`/`limit`, source and Markdown files over 300 lines; the refusal carries the file's outline so the agent can pick symbols with `acus show`.
- Grep: refused with the equivalent `acus find` command; counts and multiline searches pass. Glob runs, with a hint at `acus map` and `acus outline`.
- Recursive listings (`tree`, `ls -R`, `find` without filters, `Get-ChildItem -Recurse`) run, with a hint at `acus map`.
- Plain `git status`, `git diff`, `git log` and `git show` run, with a hint at `acus diff` or `acus log`. Output already compacted by a flag (`--short`, `--stat`, `--name-only`, `--oneline`, `--format`) and `git show REV:path` pass without a hint.

A refused shell command can run anyway with a `command ` prefix (Bash) or a trailing `# acus-skip` comment (both shells). Every decision, escapes included, is logged next to the config file; `acus usage --guard` counts them per rule and lists the escaped calls, which point at refusals acus could not replace. `|| true` lets every call through when acus is missing or `guard` is disabled in the configuration. Hooks apply to subagents too, and on Windows Claude Code runs them with Git Bash.

Subagents do not see SessionStart output. Preload the skill in each custom agent's frontmatter, and give read-only agents `Bash` so they can run it:

```yaml
tools: Read, Glob, Grep, Bash
skills:
  - acus
```

The built-in Explore agent cannot preload skills; a custom agent with the same role can.

**Codex.** Codex has no session hooks. Add a line to `~/.codex/AGENTS.md`:

```markdown
Search and read code with `acus` instead of grep/rg/find/cat/sed -n (`acus find 'PAT' --block`, `acus outline PATH`, `acus show path#Symbol path:A-B`), edit with one `acus patch`, and run builds and tests via `acus ctx` (alone for the project's tests, or `acus ctx 'COMMAND'`).
```

## Commands

| Command | What it does |
| --- | --- |
| `acus find PATTERN [PATH...]` | Regex search, hits grouped by enclosing symbol; `--block` adds the bodies; `-w`, `-l`, `-t`, `-u` work as in rg |
| `acus outline PATH...` | Symbols of a file or directory with kinds and line ranges |
| `acus show ADDR...` | Symbols, line ranges or whole files, numbered |
| `acus patch` | Applies a patch from stdin, all or nothing, and prints the written lines |
| `acus ctx ['COMMAND']` | Runs a build or test, drops progress noise, colour codes, passing tests and library stack frames, and shows the code behind every `path:line` in its output, each function once. Without a command it runs the project's tests (detected from Cargo.toml, Package.swift, go.mod, package.json or pytest files). When output is cut, the full log is saved to a temp file and its path printed |
| `acus diff [REV] [PATH...]` | Uncommitted changes per function or type, with untracked files; `-p` adds the lines, `--staged` and `A..B` work as in git |
| `acus log [REV] [PATH...]` | One line per commit with date, author and changed lines; `-n` sets the count (default 15); `REV` can be `A..B` |
| `acus run` | Several of the above in one process, one command line per stdin line (or a JSON list of argument lists) |
| `acus usage` | Token, cost and tool-call statistics from Claude Code and Codex transcripts; `--tools` shows how agents search, read, edit and build (acus vs grep, sed, python, built-ins); `--guard` counts `acus guard` decisions per rule |
| `acus decide QUESTION` | Yes/no, choice or score answer from a Jev-compatible API; without an API key it warns and exits 2 |
| `acus map [DIR]` | Directory tree with file and line counts within a token budget (`--budget 400`): the largest directories open first, chains like `a/b/c/` and single-file directories are rolled up, project roots are tagged, and small directories list their files' top-level symbols; tests and docs open last |
| `acus skill` | Prints the bundled agent skill for a SessionStart hook; `--install` writes it for Claude Code and Codex, `--install --hooks` also registers the hooks |
| `acus update` | Replaces the binary with the latest GitHub release when newer and reinstalls the skill; `--check` exits 0 when an update exists, 1 when current |
| `acus guard` | PreToolUse hook for Bash, PowerShell, Read, Grep and Glob that refuses shell searches, reads and in-place edits of source files, filtered builds, whole reads of large files and Grep calls, and names the acus command instead |

Addresses look like `path#Type::method`, `path:10-40`, `path:10` or just `path`. A unique suffix such as `#method` is enough, `Type.method` works too, and config keys nest the same way (`config.yaml#server::port`). A symbol is shown with its doc comments, attributes and decorators.

Exit codes are 0 for results or success, 1 for an empty result when that command treats emptiness as no match, and 2 for errors. Empty `find`, `log` and `diff` results serialize as `null`; `run --json` emits one `null` per empty entry. `find` and `log` with no matches exit 1, while `diff` with no changes exits 0. Errors carry a `hint:` line. `ctx` passes on the exit code of the command it ran.

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

SEARCH/REPLACE blocks are the format Aider made popular and can stand in for `-`/`+` lines inside any `Update File`. `*** Replace All: PATH [PATH...]` changes every occurrence, also part of a line, in the space-separated files, directories and globs after it (a glob such as `*.py` matches below the current directory), and fails if a block matches nowhere. A block is `<<<<<<< SEARCH` (literal, whitespace-exact, may span lines) or `<<<<<<< REGEX` (a Rust regex run per file with `(?m)`; `${1}` in the replacement). `-` and SEARCH lines inside `Update File` match whole lines, and a failed hunk that holds only part of a line says to use `Replace All`; `acus patch --help` shows the syntax. `Delete Symbol` and `Move Symbol` carry doc comments and attributes along; a move goes `*** Before:` or `*** After:` another symbol, or `*** To:` the end of a file, which is created if needed.

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
command = "just test"                               # what `acus ctx` alone runs; default: detected

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

CI runs the same checks on Ubuntu and Windows. Releases are built by the `Release` workflow: bump the version in `Cargo.toml`, add a `CHANGELOG.md` section, merge, then run the workflow with the tag (`v0.2.0`). It builds macOS (arm64, x86_64), Windows (x86_64) and Linux (x86_64, arm64) archives with checksums.

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
