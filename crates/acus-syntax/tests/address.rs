use acus_syntax::{Address, Target};

#[test]
fn parses_all_forms() {
    assert_eq!(
        Address::parse("src/a.rs#A::t").target,
        Target::Symbol("A::t".into())
    );
    assert_eq!(
        Address::parse("src/a.rs:10-20").target,
        Target::Lines(10, 20)
    );
    assert_eq!(Address::parse("src/a.rs:7").target, Target::Lines(7, 7));
    assert_eq!(Address::parse("src/a.rs").target, Target::Whole);
    let win = Address::parse(r"C:\x\a.rs:3");
    assert_eq!(
        (win.path.as_str(), win.target),
        (r"C:\x\a.rs", Target::Lines(3, 3))
    );
    assert_eq!(Address::parse(r"C:\x\a.rs").target, Target::Whole);
    assert_eq!(
        Address::parse("README.md#Sub One").target,
        Target::Symbol("Sub One".into())
    );
    for s in ["src/a.rs#A::t", "src/a.rs:10-20", "src/a.rs:7", "src/a.rs"] {
        assert_eq!(Address::parse(s).to_string(), s);
    }
}

#[cfg(feature = "lang-rust")]
#[test]
fn resolves_exact_suffix_and_ambiguity() {
    use acus_syntax::{Lang, Resolve, outline, resolve};
    let src = "struct A;\nimpl A {\n    fn new() {}\n}\nstruct B;\nimpl B {\n    fn new() {}\n}\nfn only() {}\n";
    let o = outline(Lang::Rust, src).unwrap();
    assert!(matches!(resolve(&o, "A::new"), Resolve::Found(s) if s.start_line == 3));
    assert!(matches!(resolve(&o, "only"), Resolve::Found(s) if s.qual == "only"));
    match resolve(&o, "new") {
        Resolve::Ambiguous(v) => assert_eq!(
            v.iter().map(|s| s.qual.as_str()).collect::<Vec<_>>(),
            ["A::new", "B::new"]
        ),
        _ => panic!("expected ambiguity"),
    }
    assert!(matches!(resolve(&o, "nope"), Resolve::Missing));
    // `A` names both the struct and its impl: prefer the container that has children.
    assert!(matches!(resolve(&o, "A"), Resolve::Found(s) if s.kind == "impl"));
}

#[cfg(feature = "lang-markdown")]
#[test]
fn twin_headings_are_ambiguous() {
    use acus_syntax::{Lang, Resolve, outline, resolve};
    let o = outline(Lang::Markdown, "# T\n## Usage\na\n## Usage\nb\n").unwrap();
    assert!(matches!(resolve(&o, "Usage"), Resolve::Ambiguous(v) if v.len() == 2));
}
