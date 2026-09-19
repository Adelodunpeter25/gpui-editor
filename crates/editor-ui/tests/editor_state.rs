use edit_buffer::Point;
use editor_ui::{EditorState, Mode};

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

#[test]
fn readonly_mode_reports_readonly_and_not_diff() {
    assert!(Mode::ReadOnly.is_readonly());
    assert!(!Mode::ReadOnly.is_diff());
    assert!(Mode::ReadOnlyDiff.is_readonly());
    assert!(Mode::ReadOnlyDiff.is_diff());
    assert!(!Mode::Editable.is_readonly());
    assert!(Mode::EditableDiff.is_diff());
}

#[test]
fn selected_text_returns_the_selection_slice() {
    let mut s = EditorState::readonly("fn main() {}\nlet x = 1;", Some(&syntax::RUST));
    assert_eq!(s.selected_text(), "");
    s.set_selection(3..7);
    assert_eq!(s.selected_text(), "main");
}

#[test]
fn select_all_selects_the_whole_buffer() {
    let mut s = EditorState::readonly("abc\ndef", Some(&syntax::RUST));
    s.select_all();
    assert_eq!(s.selected_text(), "abc\ndef");
}

#[test]
fn row_offsets_bound_each_row_and_include_the_newline() {
    let s = EditorState::readonly("abc\nde\nf", Some(&syntax::RUST));
    assert_eq!(s.row_start_offset(0), 0);
    assert_eq!(s.row_end_offset(0), 4); // "abc\n"
    assert_eq!(s.row_start_offset(1), 4);
    assert_eq!(s.row_end_offset(1), 7); // "de\n"
    assert_eq!(s.row_start_offset(2), 7);
    assert_eq!(s.row_end_offset(2), 8); // "f", no trailing newline
}

#[test]
fn point_to_offset_matches_row_start() {
    let s = EditorState::readonly("abc\ndef", Some(&syntax::RUST));
    assert_eq!(s.point_to_offset(Point { row: 1, col: 0 }), 4);
    assert_eq!(s.point_to_offset(Point { row: 1, col: 2 }), 6);
}
