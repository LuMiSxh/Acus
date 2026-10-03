use crate::Lang;
use tree_sitter::Node;

pub(crate) struct Class {
    pub kind: &'static str,
    pub name: String,
    pub detail: Option<String>,
    /// Leaves are not searched for nested symbols (function bodies, structs, keys).
    pub leaf: bool,
}

pub(crate) fn text<'a>(n: Node, src: &'a [u8]) -> &'a str {
    n.utf8_text(src).unwrap_or("")
}

fn field<'a>(n: Node, f: &str, src: &'a [u8]) -> Option<&'a str> {
    n.child_by_field_name(f).map(|c| text(c, src))
}

fn named(kind: &'static str, n: Node, src: &[u8], leaf: bool) -> Option<Class> {
    Some(Class {
        kind,
        name: field(n, "name", src)?.trim().to_owned(),
        detail: None,
        leaf,
    })
}

pub(crate) fn classify(lang: Lang, n: Node, src: &[u8]) -> Option<Class> {
    match lang {
        Lang::Rust => rust(n, src),
        _ => None,
    }
}

fn rust(n: Node, src: &[u8]) -> Option<Class> {
    let leaf = |k: &'static str| named(k, n, src, true);
    match n.kind() {
        "function_item" | "function_signature_item" => leaf("fn"),
        "struct_item" => leaf("struct"),
        "union_item" => leaf("union"),
        "enum_item" => leaf("enum"),
        "const_item" => leaf("const"),
        "static_item" => leaf("static"),
        "type_item" => leaf("type"),
        "macro_definition" => leaf("macro"),
        "mod_item" => named("mod", n, src, false),
        "trait_item" => named("trait", n, src, false),
        "impl_item" => {
            let ty = field(n, "type", src)?;
            let name = ty.split('<').next().unwrap_or(ty).trim().to_owned();
            let detail = field(n, "trait", src).map(|t| format!("{t} for {ty}"));
            Some(Class {
                kind: "impl",
                name,
                detail,
                leaf: false,
            })
        }
        _ => None,
    }
}
