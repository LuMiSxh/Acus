# Changelog

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
