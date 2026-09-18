use editor_ui::EditorState;

#[test]
fn readonly_ignores_insert() {
    let mut s = EditorState::readonly("hello", Some(&syntax::RUST));
    s.insert(0, "X");
    assert_eq!(s.line_text(0), "hello");
}

#[test]
fn bracket_match_finds_pair() {
    let s = EditorState::readonly("fn f() {}", Some(&syntax::RUST));
    // offset of '(' is 4
    let m = s.bracket_match(4);
    assert_eq!(m, Some((4, 5)));
}

#[test]
fn indent_adds_level_after_brace() {
    let s = EditorState::readonly("fn f() {", Some(&syntax::RUST));
    assert_eq!(s.indent_for_newline(0), "    ");
}
