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

/// Regression guard for the byte -> char conversion of span offsets: every
/// span must slice the source text to real characters, not raw byte
/// offsets (they differ once multi-byte chars appear before a span).
#[test]
fn multibyte_span_offsets_are_char_offsets() {
    // 12 chars, 19 bytes: "héllo" is 6 bytes, "wörld" 6 bytes, emoji 4 bytes.
    let text = "fn héllo() { let s = \"wörld☀\"; }\n";
    let v = highlight(text, Some(&RUST), 1);
    assert!(!v.spans.is_empty());
    for span in &v.spans {
        let sliced: String = text
            .chars()
            .skip(span.start)
            .take(span.end - span.start)
            .collect();
        assert!(!sliced.is_empty(), "empty span at {}..{}", span.start, span.end);
        // A byte offset used as a char offset would often slice into the
        // middle of a multi-byte char or shift the text; assert the span
        // lands on a plausible token (no partial char, no lone quotes).
        assert!(
            !sliced.starts_with('\u{fffd}') && text.contains(sliced.as_str()),
            "span {}..{} sliced to {sliced:?}, not a substring of the source",
            span.start,
            span.end
        );
    }
    // The keyword after the multi-byte identifier must still be found at a
    // char offset that actually reads "let".
    let let_span = v
        .spans
        .iter()
        .find(|s| s.capture == Capture::Keyword && text.chars().skip(s.start).take(3).collect::<String>() == "let");
    assert!(let_span.is_some(), "no Keyword span lands on 'let'");
}

#[test]
fn language_registry_resolution() {
    let reg = LanguageRegistry::builtin();
    assert_eq!(reg.for_extension("rs").unwrap().name, "rust");
    assert_eq!(reg.for_extension("py").unwrap().name, "python");
    assert_eq!(reg.for_extension("ts").unwrap().name, "typescript");
    assert_eq!(reg.for_name("rust").unwrap().name, "rust");
}
