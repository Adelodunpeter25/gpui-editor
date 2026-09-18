use syntax::{Capture, RUST, highlight};

#[test]
fn highlights_rust_keyword_and_comment() {
    let v = highlight("fn main() {} // hi\n", Some(&RUST), 1);
    assert!(v.spans.iter().any(|s| s.capture == Capture::Keyword));
    assert!(v.spans.iter().any(|s| s.capture == Capture::Comment));
    assert_eq!(v.buffer_version, 1);
}

#[test]
fn unknown_language_still_scans() {
    let v = highlight("hello 42", None, 0);
    assert!(v.spans.iter().any(|s| s.capture == Capture::Number));
}
