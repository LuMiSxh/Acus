---
name: acus
description: Code search, reading and editing CLI that replaces grep/rg/find/cat/sed/head and Read→Edit chains with one call. Use when searching a codebase, locating a definition or its usages, reading a file, function, class or line range, getting an overview of an unfamiliar file or directory, editing several places or files at once (instead of sed -i, perl -pi or a Python str.replace/re.sub script), renaming across files, moving or deleting a function, running a build or tests and inspecting the failing code, or reviewing uncommitted changes — i.e. whenever about to run grep, rg, find, cat, head, tail, sed -n or git diff, or to Read a file just to look something up.
---

# acus

One call answers what usually takes search → read → read. Output is line-numbered, grouped by enclosing symbol, and every truncation names the exact next command.

## Pick the command

| You want | Run |
|---|---|
| Where X is used or defined | `acus find 'PAT' [PATH…] [-g '*.rs']` |
| …plus the code around each hit | `acus find 'PAT' --block` |
| Just the files / whole words / one type | `-l`, `-w`, `-t py`; `-u --hidden` also search ignored and hidden files |
| What a file or directory contains | `acus outline PATH… [--depth 1]` |
| A symbol, line range or file | `acus show src/a.rs#Parser::parse src/b.rs:40-80 README.md` |
| Several independent lookups | `acus run` with one command per line on stdin |
| Edit, rename across files, move or delete a symbol | `acus patch` with the patch on stdin |
| Run tests/build and see the failing code | `acus ctx 'cargo test -q'` (noise is dropped; `--drop 'RE'` for more) |
| What changed (before a commit, review or handoff) | `acus diff [REV] [-p]` |

## Patch

Codex `apply_patch` format; `@@` takes the start of an enclosing line. No prior Read needed.

```bash
acus patch --fmt <<'PATCH'
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
PATCH
```

Output lists the written lines with their new numbers, so no re-read is needed. All or nothing: on error nothing was written, so fix that hunk and resend the whole patch. `--check` validates only. `--fmt` runs the file's formatter (rustfmt, ruff/black, prettier, swift-format, gofmt) and prints the formatted lines, so the next patch's context still matches. Pick a heredoc delimiter that no line of the patch equals (`PATCH`, or `ACUS_END` when editing docs or scripts). Symbols may be written `Class.method` too.

More directives, mixable with the above in one patch:

```
*** Update File: src/a.rs
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
*** Delete Symbol: src/a.rs#unused
*** Move Symbol: src/a.rs#helper
*** After: src/b.rs#main
```

- SEARCH/REPLACE inside `Update File` matches whole lines (an `@@` line before it narrows the place).
- `Replace All` takes files, directories and globs and changes every occurrence; a block that matches nothing fails the patch.
- `Delete Symbol` and `Move Symbol` take the doc comments and attributes along; `Move Symbol` needs `*** Before:` or `*** After: path#Sym`, or `*** To: path`.

Batch lookups, one command per line:

```bash
acus run <<'EOF'
find 'fn parse' --block
show src/a.rs#f src/b.rs:10-30
EOF
```

## Reading the output

- Exit 0 = results, 1 = nothing found (not a failure), 2 = error plus a `hint:` line with the fix; `ctx` returns the command's own code.
- `… N more …` names the follow-up command; run it instead of searching again.
- Addresses from earlier output (`path#Sym`, `path:A-B`) can be pasted verbatim; a unique suffix such as `#parse` is enough, and an ambiguous one lists the candidates.

## Common mistakes

- Unquoted globs are expanded by the shell: write `-g '*.ts'`, and `-g '!dist'` to exclude.
- Patterns are Rust regex, so `foo(` fails; use `-F` for literal text.
- Symbols exist for Rust, Swift, TS/JS, Svelte, Python, Markdown and TOML/JSON/YAML keys. Other files: `find` and `show path:A-B`.

Flags: `acus help <command>`.
