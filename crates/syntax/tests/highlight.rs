use syntax::{highlight, Capture, LanguageRegistry, PLAIN_TEXT, PYTHON, RUST, TOML};

#[test]
fn highlights_rust_keyword_and_comment() {
    let v = highlight("fn main() {} // hi\n", Some(&RUST), 1);
    assert!(v.spans.iter().any(|s| s.capture == Capture::Keyword));
    assert!(v.spans.iter().any(|s| s.capture == Capture::Comment));
    assert_eq!(v.buffer_version, 1);
}

#[test]
fn highlights_python_and_toml() {
    let py = highlight("def test():\n    return 42\n", Some(&PYTHON), 1);
    assert!(py.spans.iter().any(|s| s.capture == Capture::Keyword || s.capture == Capture::Function));

    let toml = highlight("[package]\nname = \"editor\"\n", Some(&TOML), 1);
    assert!(toml.spans.iter().any(|s| s.capture == Capture::String || s.capture == Capture::Type || s.capture == Capture::Keyword));
}

#[test]
fn plain_text_handles_gracefully() {
    let v = highlight("hello world", Some(&PLAIN_TEXT), 0);
    assert_eq!(v.buffer_version, 0);

    let v_none = highlight("hello world", None, 2);
    assert_eq!(v_none.buffer_version, 2);
}

#[test]
fn language_registry_resolution() {
    let reg = LanguageRegistry::builtin();
    assert_eq!(reg.for_extension("rs").unwrap().name, "rust");
    assert_eq!(reg.for_extension("py").unwrap().name, "python");
    assert_eq!(reg.for_extension("ts").unwrap().name, "typescript");
    assert_eq!(reg.for_name("rust").unwrap().name, "rust");
}
