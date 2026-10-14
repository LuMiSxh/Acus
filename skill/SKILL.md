---
name: acus
description: Code search, reading and editing CLI that replaces grep/rg/find/cat/head/tail/sed and Read→Edit chains with one call. Use whenever about to search a codebase, read a file, symbol or line range, get an overview of a file or directory, edit one or more files (instead of sed -i, perl -pi or a Python replace script), run a build or tests and inspect the failures, or review uncommitted changes and recent commits.
---

# acus

One call answers what usually takes search → read → read. Output is line-numbered, grouped by enclosing symbol, and every truncation names the exact next command.

## Instead of … run

| Instead of | Run |
|---|---|
| `grep`, `rg`, `find` to locate code or usages | `acus find 'PAT' [PATH…] [-g '*.rs']`; `--block` adds the code around each hit; `-l`, `-w`, `-i`, `-c`, `-o`, `-v`, `-t py` work as in grep; `-u --hidden` also searches ignored and hidden files |
| `ls -R`, `tree`, `find -type f` | `acus map [DIR]` |
| `cat`, `head`, `tail`, `sed -n`, or Read to look something up | `acus show src/a.rs#Parser::parse src/b.rs:40-80 README.md` (several in one call); `acus outline PATH… [--depth 1]` for what a file or directory contains |
| `sed -i`, `perl -pi`, Python replace scripts, Edit | `acus patch` (below) |
| `cargo test \| tail`, `npm test 2>&1 \| grep …` | `acus ctx 'CMD'` with `--tail N` or `--grep RE`; `acus ctx` alone runs the project's tests |
| `git status`, `git diff` | `acus diff [REV] [-p]` |
| `git log` | `acus log [REV] [PATH…] [-n N]`; one commit's changes: `acus diff REV^..REV` |

A hook catches the commands on the left: it rewrites those with an exact acus equivalent and refuses the rest, and a refused command costs a turn. Calling acus directly is also shorter, because line numbers and enclosing symbols come with the first result.

## Batch lookups

Every tool call is one turn. Before calling, list the lookups you still need, then send all that do not depend on each other at once: several tool calls in one response, or one `acus run` with one command per line:

```bash
acus run <<'EOF'
find 'fn parse' --block
show src/a.rs#f src/b.rs:10-30
outline src/lib.rs
EOF
```

## Patch

Codex `apply_patch` format; `@@` takes the start of an enclosing line. No prior Read needed. Put every change of one step into one patch.

```bash
acus patch --fmt <<'PATCH'
*** Begin Patch
*** Update File: src/lib.rs
@@ fn parse
-        let x = 1;
+        let x = 2;
@@ fn finish
-        self.done = false;
+        self.done = true;
*** Replace Symbol: src/lib.rs#Parser::reset
+    fn reset(&mut self) {
+        self.pos = 0;
+    }
*** Add File: notes.md
+hello
*** Delete File: old.rs
*** End Patch
PATCH
```

Hunk rules:
- A hunk is one contiguous block of the file. Edits at separate places get separate hunks, each with its own `@@` anchor, as in `fn parse` and `fn finish` above.
- Context and `-` lines are whole lines copied exactly, without the line-number prefix `acus show` prints. To change part of a line, use `*** Replace All:` (below).
- acus still applies some near misses (whitespace drift, non-adjacent lines in one hunk, a unique part of a line) and then prints a `note:`; check the written lines it shows.

Output lists the written lines with their new numbers, so no re-read is needed. All or nothing: on error nothing was written, and the error shows the nearest matching lines; fix that hunk from them and resend the whole patch. `--check` validates without writing. `--fmt` runs the file's formatter (rustfmt, ruff/black, prettier, swift-format, gofmt) and prints the formatted lines, so the next patch's context still matches. Pick a heredoc delimiter that no line of the patch equals (`PATCH`, or `ACUS_END` when editing docs or scripts). Symbols may be written `Class.method` too.

More directives, mixable with the above in one patch (`acus patch --help` has the full syntax):

```
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
*** Delete Symbol: src/a.rs#unused
*** Move Symbol: src/a.rs#helper
*** After: src/b.rs#main
*** Insert After: src/a.rs#main
+fn helper() {}
*** Copy File: src/a.rs
*** To: src/a_copy.rs
```

- `*** Replace All: PATH [PATH...]` takes space-separated files, directories and globs (`*.py`, matched below the current directory) and changes every occurrence. SEARCH blocks are literal and whitespace-exact and may span lines; REGEX blocks are Rust regex run per file with `(?m)`, with `${1}` in the replacement. A block that matches nothing fails the patch.
- `*** Update File:` also accepts SEARCH blocks, matched as whole lines.
- `Delete Symbol` and `Move Symbol` take doc comments and attributes along; `Move Symbol` needs `*** Before:` or `*** After: path#Sym`, or `*** To: path`.

## Reading the output

- Exit 0 = results, 1 = nothing found (not a failure), 2 = error plus a `hint:` line with the fix; `ctx` returns the command's own code.
- `… N more …` names the follow-up command; run it instead of searching again.
- Addresses from earlier output (`path#Sym`, `path:A-B`) can be pasted verbatim; a unique suffix such as `#parse` is enough, and an ambiguous one lists the candidates. Config keys nest: `cfg.yaml#server.port`.

## Common mistakes

- Unquoted globs are expanded by the shell: write `-g '*.ts'`, and `-g '!dist'` to exclude.
- Patterns are Rust regex, so `foo(` fails; use `-F` for literal text.
- Symbols exist for Rust, Swift, TS/JS, Svelte, Python, Markdown and TOML/JSON/YAML keys. Other files: `find` and `show path:A-B`.
- Paths may be relative, absolute or start with `~`.

Flags: `acus help <command>`.
