# Changelog

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
