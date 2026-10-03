use acus_syntax::{Lang, outline};

#[allow(dead_code)] // unused when every language feature is off
fn rows(lang: Lang, src: &str) -> Vec<String> {
    outline(lang, src)
        .unwrap()
        .symbols
        .iter()
        .map(|s| {
            format!(
                "{}{} {} {}-{}",
                "  ".repeat(s.depth),
                s.kind,
                s.qual,
                s.start_line,
                s.end_line
            )
        })
        .collect()
}

#[cfg(feature = "lang-rust")]
#[test]
fn rust_outline() {
    let src = "mod m {\n    pub struct A;\n    impl Display for A<T> {\n        fn fmt(&self) {}\n    }\n}\nfn main() {\n    fn inner() {}\n}\n";
    assert_eq!(
        rows(Lang::Rust, src),
        [
            "mod m 1-6",
            "  struct m::A 2-2",
            "  impl m::A 3-5",
            "    fn m::A::fmt 4-4",
            "fn main 7-9",
        ]
    );
    let o = outline(Lang::Rust, src).unwrap();
    assert_eq!(o.symbols[2].detail.as_deref(), Some("Display for A<T>"));
    assert_eq!(o.enclosing(4).unwrap().qual, "m::A::fmt");
    assert_eq!(o.enclosing(8).unwrap().qual, "main"); // fn bodies are leaves
    assert!(o.enclosing(10).is_none());
    assert_eq!(o.lines, 9);
}

#[test]
fn lang_from_path() {
    use std::path::Path;
    assert_eq!(Lang::from_path(Path::new("a/b.RS")), Some(Lang::Rust));
    assert_eq!(Lang::from_path(Path::new("x.yml")), Some(Lang::Yaml));
    assert_eq!(Lang::from_path(Path::new("x.c")), None);
}

#[cfg(feature = "lang-swift")]
#[test]
fn swift_outline() {
    let src = "public struct S {\n  var p: Int = 1\n  init() {}\n  func f() {}\n}\nextension S: P {\n  func g() {}\n}\nprotocol P {\n  func h()\n}\nenum E { case a }\ntypealias T = Int\n";
    assert_eq!(
        rows(Lang::Swift, src),
        [
            "struct S 1-5",
            "  var S::p 2-2",
            "  init S::init 3-3",
            "  func S::f 4-4",
            "extension S 6-8",
            "  func S::g 7-7",
            "protocol P 9-11",
            "  func P::h 10-10",
            "enum E 12-12",
            "typealias T 13-13",
        ]
    );
}

#[cfg(feature = "lang-ts")]
#[test]
fn typescript_outline() {
    let src = "export class A {\n  m() {}\n}\ninterface I { x: number }\ntype T = string\nenum E { A }\nexport const f = () => 1\nconst n = 1\nfunction g() {\n  const inner = () => 2\n}\n";
    assert_eq!(
        rows(Lang::TypeScript, src),
        [
            "class A 1-3",
            "  method A::m 2-2",
            "interface I 4-4",
            "type T 5-5",
            "enum E 6-6",
            "function f 7-7",
            "function g 9-11",
        ]
    );
}

#[cfg(feature = "lang-python")]
#[test]
fn python_outline() {
    let src = "class A:\n    def b(self):\n        pass\n\n@dec\ndef c():\n    pass\n";
    assert_eq!(
        rows(Lang::Python, src),
        ["class A 1-3", "  def A::b 2-3", "def c 6-7"]
    );
}

#[cfg(feature = "lang-markdown")]
#[test]
fn markdown_outline() {
    let src = "# Title\n\ntext\n\n## Sub One\n\nx\n\n# Two\n";
    assert_eq!(
        rows(Lang::Markdown, src),
        ["h1 Title 1-8", "  h2 Title::Sub One 5-8", "h1 Two 9-9"]
    );
}

#[cfg(feature = "lang-svelte")]
#[test]
fn svelte_outline_keeps_file_lines() {
    let src =
        "<script lang=\"ts\">\n  function f() {}\n  const g = () => 1\n</script>\n\n<p>hi</p>\n";
    assert_eq!(
        rows(Lang::Svelte, src),
        ["function f 2-2", "function g 3-3"]
    );
}

#[cfg(feature = "lang-data")]
#[test]
fn data_outlines_list_top_level_keys() {
    assert_eq!(
        rows(Lang::Toml, "[a]\nb = 1\n[[c.d]]\ne = 2\n"),
        ["table a 1-2", "array c.d 3-4"]
    );
    assert_eq!(
        rows(Lang::Json, "{\n  \"a\": {\"x\": 1},\n  \"b\": 2\n}\n"),
        ["key a 2-2", "key b 3-3"]
    );
    assert_eq!(
        rows(Lang::Yaml, "a:\n  x: 1\nb: 2\n"),
        ["key a 1-2", "key b 3-3"]
    );
}
