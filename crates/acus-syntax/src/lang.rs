use serde::Serialize;
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Lang {
    Rust,
    Swift,
    TypeScript,
    Tsx,
    JavaScript,
    Python,
    Markdown,
    Svelte,
    Toml,
    Json,
    Yaml,
}

impl Lang {
    pub fn from_path(path: &Path) -> Option<Lang> {
        let ext = path.extension()?.to_str()?.to_ascii_lowercase();
        Some(match ext.as_str() {
            "rs" => Lang::Rust,
            "swift" => Lang::Swift,
            "ts" | "mts" | "cts" => Lang::TypeScript,
            "tsx" => Lang::Tsx,
            "js" | "mjs" | "cjs" | "jsx" => Lang::JavaScript,
            "py" | "pyi" => Lang::Python,
            "md" | "markdown" => Lang::Markdown,
            "svelte" => Lang::Svelte,
            "toml" => Lang::Toml,
            "json" => Lang::Json,
            "yaml" | "yml" => Lang::Yaml,
            _ => return None,
        })
    }

    pub fn name(self) -> &'static str {
        match self {
            Lang::Rust => "rust",
            Lang::Swift => "swift",
            Lang::TypeScript => "typescript",
            Lang::Tsx => "tsx",
            Lang::JavaScript => "javascript",
            Lang::Python => "python",
            Lang::Markdown => "markdown",
            Lang::Svelte => "svelte",
            Lang::Toml => "toml",
            Lang::Json => "json",
            Lang::Yaml => "yaml",
        }
    }

    /// The grammar, or `None` when its Cargo feature is disabled.
    // The `_` arm is live only when a language feature is off; with none, every arm diverges.
    #[allow(unreachable_patterns, unreachable_code)]
    pub(crate) fn grammar(self) -> Option<tree_sitter::Language> {
        Some(match self {
            #[cfg(feature = "lang-rust")]
            Lang::Rust => tree_sitter_rust::LANGUAGE.into(),
            #[cfg(feature = "lang-swift")]
            Lang::Swift => tree_sitter_swift::LANGUAGE.into(),
            #[cfg(feature = "lang-ts")]
            Lang::TypeScript => tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
            #[cfg(feature = "lang-ts")]
            Lang::Tsx => tree_sitter_typescript::LANGUAGE_TSX.into(),
            #[cfg(feature = "lang-ts")]
            Lang::JavaScript => tree_sitter_javascript::LANGUAGE.into(),
            #[cfg(feature = "lang-python")]
            Lang::Python => tree_sitter_python::LANGUAGE.into(),
            #[cfg(feature = "lang-markdown")]
            Lang::Markdown => tree_sitter_md::LANGUAGE.into(),
            #[cfg(feature = "lang-svelte")]
            Lang::Svelte => tree_sitter_svelte_ng::LANGUAGE.into(),
            #[cfg(feature = "lang-data")]
            Lang::Toml => tree_sitter_toml_ng::LANGUAGE.into(),
            #[cfg(feature = "lang-data")]
            Lang::Json => tree_sitter_json::LANGUAGE.into(),
            #[cfg(feature = "lang-data")]
            Lang::Yaml => tree_sitter_yaml::LANGUAGE.into(),
            _ => return None,
        })
    }
}
