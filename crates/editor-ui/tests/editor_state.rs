use edit_buffer::Point;
use editor_ui::{EditorState, FontConfig, Mode};
use gpui::{px, AppContext as _, TestAppContext};

// `insert` takes a `Context<EditorState>` (to spawn a background rehighlight
// on large files) — the first test in this crate needing a real GPUI
// executor rather than plain struct construction.
#[gpui::test]
fn readonly_ignores_insert(cx: &mut TestAppContext) {
    let state = cx.update(|cx| cx.new(|_| EditorState::readonly("hello", Some(&syntax::RUST))));
    state.update(cx, |s, cx| s.insert(0, "X", cx));
    state.read_with(cx, |s, _| assert_eq!(s.line_text(0), "hello"));
}

/// Above `SYNC_HIGHLIGHT_THRESHOLD`, `set_text` defers highlighting to a
/// background task (see `EditorState::rehighlight_from`) — but the buffer
/// content itself must update synchronously regardless, before that task
/// ever runs.
#[gpui::test]
fn large_set_text_updates_buffer_before_background_highlight_runs(cx: &mut TestAppContext) {
    let state = cx.update(|cx| cx.new(|_| EditorState::readonly("small", Some(&syntax::RUST))));
    let big = "let x = 1;\n".repeat(6_500); // ~71.5KB, just over the sync threshold
    state.update(cx, |s, cx| s.set_text(&big, cx));

    // Content is already there even though the background parse hasn't
    // been polled yet.
    state.read_with(cx, |s, _| assert_eq!(s.line_text(0), "let x = 1;"));

    cx.run_until_parked();
    state.read_with(cx, |s, _| assert_eq!(s.line_text(0), "let x = 1;"));
}

/// A second large `set_text` before the first's background highlight has
/// landed must not corrupt anything — the stale result (still in flight for
/// the old version) has to be discarded, not applied on top of the newer
/// buffer content.
#[gpui::test]
fn rapid_large_set_text_calls_leave_buffer_matching_the_latest_text(cx: &mut TestAppContext) {
    let state = cx.update(|cx| cx.new(|_| EditorState::readonly("small", Some(&syntax::RUST))));
    let big_a = "a\n".repeat(33_000); // ~66KB, just over the sync threshold
    let big_b = "b\n".repeat(33_000);

    state.update(cx, |s, cx| s.set_text(&big_a, cx));
    state.update(cx, |s, cx| s.set_text(&big_b, cx));
    cx.run_until_parked();

    state.read_with(cx, |s, _| assert_eq!(s.line_text(0), "b"));
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

#[test]
fn wrap_is_off_by_default_and_toggleable() {
    let mut s = EditorState::readonly("abc", Some(&syntax::RUST));
    assert!(!s.wrap_enabled());
    s.set_wrap_enabled(true);
    assert!(s.wrap_enabled());
    s.set_wrap_enabled(false);
    assert!(!s.wrap_enabled());
}

#[test]
fn with_wrap_builder_sets_initial_state() {
    let s = EditorState::readonly("abc", Some(&syntax::RUST)).with_wrap(true);
    assert!(s.wrap_enabled());
}

#[test]
fn default_font_matches_prior_hardcoded_values() {
    let s = EditorState::readonly("abc", Some(&syntax::RUST));
    assert_eq!(s.font().family.as_ref(), "JetBrains Mono");
    assert_eq!(s.font().size, px(14.0));
    assert_eq!(s.font().line_height, px(22.0));
}

#[test]
fn font_is_configurable_via_builder_and_setter() {
    let custom = FontConfig {
        family: "Menlo".into(),
        size: px(16.0),
        line_height: px(24.0),
    };

    let s = EditorState::readonly("abc", Some(&syntax::RUST)).with_font(custom.clone());
    assert_eq!(s.font(), &custom);

    let mut s = EditorState::readonly("abc", Some(&syntax::RUST));
    s.set_font(custom.clone());
    assert_eq!(s.font(), &custom);
}
