# Changelog

## [0.8.0]

### Added

- `acus guard` names the refused part of a compound command: the reason quotes the segment, says that no part of the command ran, and shows the acus command to run in its place (`acus find` with the grep flags carried over, `acus show path:A-B` for `head`/`sed -n`/`Get-Content -TotalCount`, `acus patch` with `*** Replace All: FILES` for `sed -i`), so the retry is one step.
- `acus guard` on Windows and PowerShell: `findstr` (cmd.exe and PowerShell, `/s /i /n /c:`), `type` and `more` reads, `grep.exe`-style and backslash command paths, `Get-ChildItem … | Select-String` pipelines (`gci`, `ls`, `dir`), `ls -r` and any `-Recurse` abbreviation as a recursive listing, `-creplace`/`-ireplace` and `.Replace()` edits written with `sc`, `Out-File`, `>` or `WriteAllText`, `ReadAllText` reads of source files, comma-separated `Get-Content a.rs,b.rs`, and `-Wait` followed like `tail -f`.
- `acus find` accepts `-r`, `-R` and `-H` as no-ops, like `-n`. Other grep and rg flags it lacks (`-c`, `-v`, `-o`, `-x`, `-m`, `--include`, `--exclude`) fail as before, now with a hint at the nearest `acus find` equivalent and `acus find --help`, also inside `acus run`.
- `acus patch --help` documents the patch format, including `*** Replace All: PATH [PATH...]` with its literal and regex blocks. `skill/SKILL.md` and the README describe it too.

### Changed

- `acus guard` no longer refuses `cat`, `head`, `tail`, `type` or `Get-Content` of `.json`, `.toml`, `.yaml` and `.yml` files, which agents read whole on purpose (the Read tool already leaves them alone). `sed -i` on them is still refused.
- `acus guard` no longer refuses `grep`, `rg`, `findstr` or `Select-String` on plain files that are not source (`.log`, `.txt`, `.csv`, data formats, `*.log` globs) or on stdin (`grep -c x <<< "$v"`, `grep -l x <(cmd)`). A directory, a glob, a variable, no operand with `rg`, `-r`, or a source file still counts as a code search.
- `acus guard` treats `cat a.rs >> all.txt` and `cat a.rs b.rs > all.txt` as copies, as before, but `2>&1` and `2>/dev/null` no longer make `cat a.rs 2>&1` look like one.
- `acus guard` also refuses `find … -exec grep` and `grep` inside `env`, `uv run` and `xargs` wrappers by their real command name.
- `acus patch` errors for a `-` or SEARCH line that holds only part of a line point at `*** Replace All: PATH` with a SEARCH or REGEX block in every case (before, only some). An unknown `***` directive lists every accepted form with its arguments and suggests the right one for a near miss (`*** Replace all:`, `*** Replace All:src`); a `<<<<<<< REGEX` block under `Update File` says it belongs under `Replace All`.

### Fixed

- `acus guard` parsed an escaped quote inside double quotes as the end of the string, which hid the commands after it; in PowerShell, backslash is a path character and the backtick the escape, so `ls src\; Select-String x a.rs` is two commands.

## [0.7.1]

### Changed

- `acus guard` also hints at edit scripts written to a file with `cat > x.py <<EOF` or `tee x.rb <<EOF` (`.py`, `.js`, `.mjs`, `.cjs`, `.rb`), not only at `python - <<EOF`. Writing a script file that does not rewrite a source file stays unhinted.

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
