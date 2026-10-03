use crate::{Lang, rules};
use serde::Serialize;
use tree_sitter::{Node, Parser, Tree};

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Symbol {
    pub kind: &'static str,
    /// Name used in addresses (`impl Foo<T>` → `Foo`, heading text for Markdown).
    pub name: String,
    /// Extra label for outlines, e.g. `Display for Foo<T>`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    /// `::`-joined names of the enclosing symbols and this one.
    pub qual: String,
    pub depth: usize,
    /// 1-based, inclusive.
    pub start_line: usize,
    pub end_line: usize,
    #[serde(skip)]
    pub start_byte: usize,
    #[serde(skip)]
    pub end_byte: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct Outline {
    pub lang: Lang,
    pub lines: usize,
    pub symbols: Vec<Symbol>,
}

impl Outline {
    /// The innermost symbol containing 1-based `line`.
    pub fn enclosing(&self, line: usize) -> Option<&Symbol> {
        self.symbols
            .iter()
            .filter(|s| s.start_line <= line && line <= s.end_line)
            .max_by_key(|s| s.depth)
    }
}

/// Symbols of `src` in source order; `None` when the grammar is not compiled in.
pub fn outline(lang: Lang, src: &str) -> Option<Outline> {
    let mut symbols = Vec::new();
    let (rules_lang, tree) = match lang {
        Lang::Svelte => (Lang::TypeScript, svelte_scripts(src)?),
        _ => (lang, parse(lang, src, &[])?),
    };
    collect(
        rules_lang,
        tree.root_node(),
        src.as_bytes(),
        &mut Vec::new(),
        &mut symbols,
    );
    Some(Outline {
        lang,
        lines: src.lines().count(),
        symbols,
    })
}

fn parse(lang: Lang, src: &str, ranges: &[tree_sitter::Range]) -> Option<Tree> {
    // One parser per thread: its internal buffers are reused across files.
    thread_local!(static PARSER: std::cell::RefCell<Parser> = std::cell::RefCell::new(Parser::new()));
    PARSER.with_borrow_mut(|p| {
        p.set_language(&lang.grammar()?).ok()?;
        // An empty slice resets to the whole document.
        p.set_included_ranges(ranges).ok()?;
        p.parse(src, None)
    })
}

/// Parses every `<script>` body of a Svelte file as TypeScript, keeping file positions.
fn svelte_scripts(src: &str) -> Option<Tree> {
    let doc = parse(Lang::Svelte, src, &[])?;
    let mut ranges = Vec::new();
    let mut cur = doc.root_node().walk();
    for el in doc
        .root_node()
        .named_children(&mut cur)
        .filter(|n| n.kind() == "script_element")
    {
        let mut c2 = el.walk();
        ranges.extend(
            el.named_children(&mut c2)
                .filter(|n| n.kind() == "raw_text")
                .map(|n| n.range()),
        );
    }
    if ranges.is_empty() {
        return parse(Lang::TypeScript, "", &[]);
    }
    parse(Lang::TypeScript, src, &ranges)
}

fn collect(lang: Lang, node: Node, src: &[u8], stack: &mut Vec<String>, out: &mut Vec<Symbol>) {
    let mut cur = node.walk();
    for child in node.named_children(&mut cur) {
        let Some(c) = rules::classify(lang, child, src) else {
            collect(lang, child, src, stack, out);
            continue;
        };
        let qual = if stack.is_empty() {
            c.name.clone()
        } else {
            format!("{}::{}", stack.join("::"), c.name)
        };
        out.push(Symbol {
            kind: c.kind,
            name: c.name.clone(),
            detail: c.detail,
            qual,
            depth: stack.len(),
            start_line: child.start_position().row + 1,
            end_line: end_line(child),
            start_byte: child.start_byte(),
            end_byte: child.end_byte(),
        });
        if !c.leaf {
            stack.push(c.name);
            collect(lang, child, src, stack, out);
            stack.pop();
        }
    }
}

/// Last line with content: a node ending at column 0 really ends on the line before.
fn end_line(n: Node) -> usize {
    let e = n.end_position();
    if e.column == 0 && e.row > n.start_position().row {
        e.row
    } else {
        e.row + 1
    }
}
