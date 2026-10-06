# Changelog

## [0.7.0]

### Added

- `acus log [REV] [PATH...] [-n N]`: one line per commit with date, author and changed lines, in agent, JSON and `run` output.
- `acus guard` hints at `acus diff` for plain `git status` and `git diff`, at `acus log` for `git log`, and at `acus diff REV^..REV` for `git show`, in Bash and PowerShell. Calls already compacted by a flag and `git show REV:path` pass without a hint.

## [0.6.1]

### Fixed

- Empty diffs now exit successfully; empty `find` and `diff` JSON results, including entries in `run --json`, emit `null`.

## [0.6.0]

### Added

- `acus update` replaces the binary in place with the latest GitHub release when it is newer, verified against the release's SHA-256, and reinstalls the skill; `--check` only reports, `--force` reinstalls. On by default as the `cmd-update` feature; it only goes online when called.

## [0.5.0]

### Added

- `acus guard` handles the Grep tool (refused with the equivalent `acus find`; counts and multiline searches pass), the Glob tool (a hint at `acus outline`) and the PowerShell tool on Windows (`Select-String`, `Get-Content`, builds piped into `Select-Object`, `-replace … | Set-Content`).
- `acus guard` logs every decision next to the config file, including calls run through the escape; `acus usage --guard` counts them per rule and lists the escaped calls.
- A trailing `# acus-skip` comment runs a refused shell command anyway, in Bash and PowerShell.
- `acus ctx` without a command runs `[ctx] command` or the project's tests, detected from Cargo.toml, Package.swift, go.mod, package.json (npm, pnpm, yarn or bun) or pytest files.
- `acus map [DIR]`: directory tree within a token budget. The largest directories open first, tests and docs last; single-child chains and single-file directories are rolled up; manifests tag project roots; small directories show their files' top-level symbols. `acus guard` hints at it for `tree`, `ls -R`, unfiltered `find` and `Get-ChildItem -Recurse`.
- `acus skill --install --hooks` registers the SessionStart and PreToolUse hooks in `~/.claude/settings.json`, keeping other settings and a backup.

### Changed

- `acus ctx` only points to the full log when output was cut or the command failed.
- `--json` output keeps keys in their natural order instead of sorting them.
- CI also runs on macOS.

### Fixed

- `acus ctx` on Windows passes the command to `cmd` verbatim, so quoted paths work.

## [0.4.0]

### Added

- `acus guard` refuses builds and tests (cargo, swift, go, npm/pnpm/yarn/bun, tsc, pytest, xcodebuild and others) piped into `tail`, `head` or `grep`, and suggests `acus ctx` with the same command.
- `acus guard` handles the Read tool: a Read without `offset`/`limit` of a source or Markdown file over 300 lines is refused with the file's outline. Add `Read` to the hook matcher (`"Bash|Read"`).
- `acus ctx` saves the full output to a temp file when it cuts or drops lines, and prints the path, so more of the output can be read without running the command again.

## [0.3.1]

### Added

- `acus guard` recognises Python, Node and Ruby scripts (heredoc, `-c`, `-e`) that read a source file, replace text and write it back. They still run, but the agent gets a hint to use `acus patch` next time.

## [0.3.0]

### Added

- `acus guard`, a Claude Code PreToolUse hook for Bash. It refuses `grep`/`rg` searches, `cat`/`head`/`tail`/`sed -n` reads of source files and `sed -i`/`perl -i` edits, and names the acus command to use instead. Pipe filters such as `git log | grep fix`, heredoc bodies and commands prefixed with `command ` pass.

## [0.2.0]

First release with prebuilt binaries for macOS, Windows and Linux.

### Added

- `acus skill` prints the bundled agent skill for a SessionStart hook; `acus skill --install` writes it to `~/.claude/skills/acus/` and `~/.agents/skills/acus/`.
- `install.sh` and `install.ps1` install the latest release and the skill.
- `find`: `-w`, `-l`, `-t TYPE`, `-u` and `-n`; long lines are clipped around the match.
- `patch`: `--fmt`, `Class.method` addresses, `Replace All` over directories and globs, better near-miss hints.
- `outline`/`show`: nested config keys, Markdown setext headings, Swift initialisers, decorators and doc comments belong to their symbol.
- `ctx` groups references per symbol and drops library frames; `usage --tools`.

### Changed

- `decide` warns and exits 2 without calling the API when no API key is set.
- `diff` compares unstaged changes against the index.
- `run` keeps going after a line that fails to parse.

### Fixed

- `find` no longer adds the regex hint to errors that are not regex errors, such as a missing path.
- `acus … | head` ends quietly instead of panicking on a closed pipe.

## [0.1.0]

Initial version: `find`, `outline`, `show`, `patch`, `ctx`, `diff`, `run`, `usage` and `decide`.
