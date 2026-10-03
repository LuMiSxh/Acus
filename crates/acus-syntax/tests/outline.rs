use acus_syntax::{Lang, outline};

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
