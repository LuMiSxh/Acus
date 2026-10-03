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
        Lang::Swift => swift(n, src),
        Lang::TypeScript | Lang::Tsx | Lang::JavaScript | Lang::Svelte => ts(n, src),
        Lang::Python => python(n, src),
        Lang::Markdown => markdown(n, src),
        Lang::Toml | Lang::Json | Lang::Yaml => data(n, src),
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

fn swift(n: Node, src: &[u8]) -> Option<Class> {
    let leaf = |k: &'static str| named(k, n, src, true);
    match n.kind() {
        "class_declaration" => {
            let kind = match field(n, "declaration_kind", src)? {
                "struct" => "struct",
                "enum" => "enum",
                "extension" => "extension",
                "actor" => "actor",
                _ => "class",
            };
            named(kind, n, src, false)
        }
        "protocol_declaration" => named("protocol", n, src, false),
        "function_declaration" | "protocol_function_declaration" => leaf("func"),
        "init_declaration" => Some(Class {
            kind: "init",
            name: "init".into(),
            detail: None,
            leaf: true,
        }),
        "property_declaration" => leaf("var"),
        "typealias_declaration" => leaf("typealias"),
        _ => None,
    }
}

fn ts(n: Node, src: &[u8]) -> Option<Class> {
    let leaf = |k: &'static str| named(k, n, src, true);
    match n.kind() {
        "class_declaration" | "abstract_class_declaration" | "class" => {
            named("class", n, src, false)
        }
        "internal_module" => named("namespace", n, src, false),
        "method_definition" => leaf("method"),
        "function_declaration" | "generator_function_declaration" => leaf("function"),
        "interface_declaration" => leaf("interface"),
        "type_alias_declaration" => leaf("type"),
        "enum_declaration" => leaf("enum"),
        "variable_declarator" => {
            let v = n.child_by_field_name("value")?.kind();
            if !matches!(v, "arrow_function" | "function_expression" | "function") {
                return None;
            }
            leaf("function")
        }
        _ => None,
    }
}

fn python(n: Node, src: &[u8]) -> Option<Class> {
    match n.kind() {
        "class_definition" => named("class", n, src, false),
        "function_definition" => named("def", n, src, true),
        _ => None,
    }
}

fn markdown(n: Node, src: &[u8]) -> Option<Class> {
    if n.kind() != "section" {
        return None;
    }
    let mut cur = n.walk();
    let heading = n
        .named_children(&mut cur)
        .find(|c| c.kind() == "atx_heading")?;
    let mut c2 = heading.walk();
    let level = heading
        .named_children(&mut c2)
        .find_map(|c| c.kind().strip_prefix("atx_h")?.strip_suffix("_marker"));
    let kind = match level? {
        "1" => "h1",
        "2" => "h2",
        "3" => "h3",
        "4" => "h4",
        "5" => "h5",
        _ => "h6",
    };
    let name = field(heading, "heading_content", src)?.trim().to_owned();
    Some(Class {
        kind,
        name,
        detail: None,
        leaf: false,
    })
}

/// Top-level tables/keys only; nested values stay folded.
fn data(n: Node, src: &[u8]) -> Option<Class> {
    let (kind, key) = match n.kind() {
        "table" => ("table", n.named_child(0)?),
        "table_array_element" => ("array", n.named_child(0)?),
        "pair" | "block_mapping_pair" => ("key", n.child_by_field_name("key")?),
        _ => return None,
    };
    let name = text(key, src)
        .trim()
        .trim_matches('"')
        .trim_matches('\'')
        .to_owned();
    Some(Class {
        kind,
        name,
        detail: None,
        leaf: true,
    })
}
