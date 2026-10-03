---
name: acus
description: Use INSTEAD OF grep/rg/sed/cat/head/Read/Edit for ANY code search, reading or multi-file editing — one call replaces search→read chains. `acus find PAT [PATH] [-e PAT] [-g GLOB] [--block]` → hits grouped under `@Symbol start-end`, --block prints whole enclosing bodies; `acus outline FILE|DIR [--depth N]` → symbols+lines; `acus show ADDR...` where ADDR = file#Symbol (suffix ok) | file:10-40 | file; `acus patch` ← apply_patch on stdin (+ `*** Replace Symbol: file#Sym`), atomic; `acus run` ← JSON [[args]...] batches calls. Exit 0 found, 1 none, 2 error. `acus help <cmd>` for flags.
---

# acus

- Reading what you find? `find --block`. Unknown file? `outline` before reading.
- Pass several addresses to one `show`; copy addresses verbatim from earlier output.
- Independent lookups: one `acus run` with `[["find","x"],["show","a.rs#f"]]` on stdin.
- Edits across files or whole symbols: one `acus patch` (heredoc, `*** Begin Patch` … `*** End Patch`); `--check` dry-runs. All or nothing.
- A `… N more` line names the exact next command or flag. Errors carry a `hint:` line.
- Globs go to `-g` (quote them); `!` excludes. Patterns are Rust regex; `-F` for literals.
- Symbols for Rust, Swift, TS/TSX/JS, Svelte, Python, Markdown, TOML/JSON/YAML (top-level keys). Other files: `find` and `file:A-B` work.
