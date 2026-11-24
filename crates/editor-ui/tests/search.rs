//! End-to-end search tests: `EditorState` search/navigation and `EditorView`
//! `Cmd+F` / `Ctrl+F` find bar focus, input isolation, navigation, and close.

use editor_ui::{EditorState, EditorView};
use gpui::{AppContext as _, Entity, TestAppContext, VisualTestContext};

fn open(
    cx: &mut TestAppContext,
    state: EditorState,
) -> (Entity<EditorState>, Entity<EditorView>, &mut VisualTestContext) {
    cx.update(editor_ui::init);
    let state = cx.update(|cx| cx.new(|_| state));
    let view_state = state.clone();
    let (view, cx) = cx.add_window_view(move |_window, cx| EditorView::new(&view_state, cx));
    let handle = view.read_with(cx, |v, _| v.focus_handle());
    cx.update(|window, cx| window.focus(&handle, cx));
    cx.run_until_parked();
    (state, view, cx)
}

#[test]
fn state_search_navigates_forward_and_backward() {
    let mut s = EditorState::editable("alpha beta alpha gamma alpha", None);
    s.set_search("alpha", true);
    assert_eq!(s.search().matches, vec![0..5, 11..16, 23..28]);
    assert_eq!(s.search().current, Some(0));
    assert_eq!(s.current_match(), Some(0..5));

    s.next_match();
    assert_eq!(s.current_match(), Some(11..16));
    s.next_match();
    assert_eq!(s.current_match(), Some(23..28));
    s.next_match();
    assert_eq!(s.current_match(), Some(0..5));

    s.prev_match();
    assert_eq!(s.current_match(), Some(23..28));
    s.prev_match();
    assert_eq!(s.current_match(), Some(11..16));
}

#[gpui::test]
fn large_buffer_search_runs_in_background_and_updates_matches(cx: &mut TestAppContext) {
    let mut big = "let x = 1;
".repeat(6_500);
    big.push_str("let target_needle = 42;
");
    let state = cx.update(|cx| cx.new(|_| EditorState::editable(&big, None)));

    state.update(cx, |s, cx| s.update_search("target_needle", true, cx));
    cx.executor().advance_clock(std::time::Duration::from_millis(60));
    cx.run_until_parked();

    state.read_with(cx, |s, _| {
        assert_eq!(s.search().matches.len(), 1);
        assert_eq!(s.search().current, Some(0));
    });
}

#[gpui::test]
fn cmd_f_opens_search_without_editing_buffer_and_escape_restores_focus(cx: &mut TestAppContext) {
    let (state, view, cx) = open(cx, EditorState::editable("foo bar foo baz foo", None));

    cx.simulate_keystrokes("cmd-f");
    cx.run_until_parked();
    view.read_with(cx, |v, _| assert!(v.is_search_open()));

    cx.simulate_input("foo");
    cx.run_until_parked();

    // Buffer text must be untouched; matches must be populated.
    state.read_with(cx, |s, _| {
        assert_eq!(s.text(), "foo bar foo baz foo");
        assert_eq!(s.search().matches, vec![0..3, 8..11, 16..19]);
        assert_eq!(s.search().current, Some(0));
        assert_eq!(s.selection().range, 0..3);
    });

    // Enter advances to next match; Shift+Enter goes back.
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    state.read_with(cx, |s, _| {
        assert_eq!(s.search().current, Some(1));
        assert_eq!(s.selection().range, 8..11);
    });

    cx.simulate_keystrokes("shift-enter");
    cx.run_until_parked();
    state.read_with(cx, |s, _| {
        assert_eq!(s.search().current, Some(0));
        assert_eq!(s.selection().range, 0..3);
    });

    // Escape closes search, clears highlights/match selection, and restores focus.
    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    view.read_with(cx, |v, _| assert!(!v.is_search_open()));
    state.read_with(cx, |s, _| {
        assert!(s.search().matches.is_empty());
        assert!(s.selection().is_empty());
    });

    // Typing now goes back into the editor buffer at the restored caret.
    cx.simulate_input("X");
    cx.run_until_parked();
    state.read_with(cx, |s, _| {
        assert_eq!(s.text(), "Xfoo bar foo baz foo");
    });
}

#[gpui::test]
fn cmd_f_prefills_from_single_line_selection_and_works_in_readonly(cx: &mut TestAppContext) {
    let (state, view, cx) = open(cx, EditorState::readonly("hello world hello", None));
    state.update(cx, |s, _| s.set_selection(0..5));

    cx.simulate_keystrokes("ctrl-f");
    cx.run_until_parked();

    view.read_with(cx, |v, _| assert!(v.is_search_open()));
    state.read_with(cx, |s, _| {
        assert_eq!(s.search().query, "hello");
        assert_eq!(s.search().matches, vec![0..5, 12..17]);
    });
}

#[gpui::test]
fn cmd_a_in_search_bar_selects_query_not_editor_buffer(cx: &mut TestAppContext) {
    let (state, _view, cx) = open(cx, EditorState::editable("foo bar baz", None));

    cx.simulate_keystrokes("cmd-f");
    cx.run_until_parked();
    cx.simulate_input("foo");
    cx.run_until_parked();

    // Cmd+A must select the search query, not the editor buffer.
    cx.simulate_keystrokes("cmd-a");
    cx.run_until_parked();
    state.read_with(cx, |s, _| {
        assert_eq!(s.selection().range, 0..3);
    });

    // Typing replaces the selected search query.
    cx.simulate_input("bar");
    cx.run_until_parked();
    state.read_with(cx, |s, _| {
        assert_eq!(s.text(), "foo bar baz");
        assert_eq!(s.search().query, "bar");
        assert_eq!(s.search().matches, vec![4..7]);
        assert_eq!(s.selection().range, 4..7);
    });

    // Clearing the search bar removes all highlights and match selection.
    cx.simulate_keystrokes("cmd-a backspace");
    cx.run_until_parked();
    state.read_with(cx, |s, _| {
        assert!(s.search().query.is_empty());
        assert!(s.search().matches.is_empty());
        assert!(s.selection().is_empty());
    });
}
