---
name: acus
description: Code search, reading and editing CLI that replaces grep/rg/find/cat/sed/head and Read→Edit chains with one call. Use when searching a codebase, locating a definition or its usages, reading a file, function, class or line range, getting an overview of an unfamiliar file or directory, editing several places or files at once, or running a build or tests and inspecting the failing code — i.e. whenever about to run grep, rg, find, cat, head, tail or sed -n, or to Read a file just to look something up.
---

# acus

One call answers what usually takes search → read → read. Output is line-numbered, grouped by enclosing symbol, and every truncation names the exact next command.

## Pick the command

| You want | Run |
|---|---|
| Where X is used or defined | `acus find 'PAT' [PATH…] [-g '*.rs']` |
| …plus the code around each hit | `acus find 'PAT' --block` |
| What a file or directory contains | `acus outline PATH… [--depth 1]` |
| A symbol, line range or file | `acus show src/a.rs#Parser::parse src/b.rs:40-80 README.md` |
| Several independent lookups | `echo '[["find","X"],["show","a.rs#f"]]' \| acus run` |
| Edit files or replace a symbol | `acus patch` with the patch on stdin |
| Run tests/build and see the failing code | `acus ctx 'cargo test -q'` |

## Patch

Codex `apply_patch` format; `@@` takes the start of an enclosing line. No prior Read needed.

```bash
acus patch <<'EOF'
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
EOF
```

Output lists the written lines with their new numbers, so no re-read is needed. All or nothing: on error nothing was written, so fix that hunk and resend the whole patch. `--check` validates only.

## Reading the output

- Exit 0 = results, 1 = nothing found (not a failure), 2 = error plus a `hint:` line with the fix.
- `… N more …` names the follow-up command; run it instead of searching again.
- Addresses from earlier output (`path#Sym`, `path:A-B`) can be pasted verbatim; a unique suffix such as `#parse` is enough, and an ambiguous one lists the candidates.

## Common mistakes

- Unquoted globs are expanded by the shell: write `-g '*.ts'`, and `-g '!dist'` to exclude.
- Patterns are Rust regex, so `foo(` fails; use `-F` for literal text.
- Symbols exist for Rust, Swift, TS/JS, Svelte, Python, Markdown and TOML/JSON/YAML keys. Other files: `find` and `show path:A-B`.

Flags: `acus help <command>`.
