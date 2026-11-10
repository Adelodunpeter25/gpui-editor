//! End-to-end edit-mode tests: a real `EditorView` in a test window, driven
//! by simulated keystrokes, so the key routing, repaint, and scroll paths
//! are covered — not just the `EditorState` model.

use editor_ui::{EditorState, EditorView};
use gpui::{px, AppContext as _, Entity, TestAppContext, VisualTestContext};

/// Open a focused editor window over `state`.
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

fn text(state: &Entity<EditorState>, cx: &mut VisualTestContext) -> String {
    state.read_with(cx, |s, _| s.text())
}

fn caret(state: &Entity<EditorState>, cx: &mut VisualTestContext) -> usize {
    state.read_with(cx, |s, _| {
        assert!(s.selection().is_empty(), "expected a caret, got a selection");
        s.selection().range.start
    })
}

fn scroll_offset(view: &Entity<EditorView>, cx: &mut VisualTestContext) -> gpui::Point<gpui::Pixels> {
    view.read_with(cx, |v, _| v.scroll_handle().0.borrow().base_handle.offset())
}

#[gpui::test]
fn typing_inserts_at_the_caret(cx: &mut TestAppContext) {
    let (state, _view, cx) = open(cx, EditorState::editable("", None));
    cx.simulate_input("hi");
    cx.simulate_keystrokes("space");
    cx.simulate_input("there");
    assert_eq!(text(&state, cx), "hi there");
    assert_eq!(caret(&state, cx), 8);
}

#[gpui::test]
fn readonly_ignores_typing(cx: &mut TestAppContext) {
    let (state, _view, cx) = open(cx, EditorState::readonly("abc", None));
    cx.simulate_input("x");
    cx.simulate_keystrokes("backspace enter");
    assert_eq!(text(&state, cx), "abc");
}

#[gpui::test]
fn backspace_delete_and_arrows_edit_around_the_caret(cx: &mut TestAppContext) {
    let (state, _view, cx) = open(cx, EditorState::editable("abcd", None));
    cx.simulate_keystrokes("end backspace");
    assert_eq!(text(&state, cx), "abc");
    cx.simulate_keystrokes("home delete");
    assert_eq!(text(&state, cx), "bc");
    cx.simulate_keystrokes("right");
    cx.simulate_input("X");
    assert_eq!(text(&state, cx), "bXc");
    assert_eq!(caret(&state, cx), 2);
}

#[gpui::test]
fn enter_after_an_opener_indents_the_new_line(cx: &mut TestAppContext) {
    let (state, _view, cx) = open(cx, EditorState::editable("fn f() {\n}", None));
    cx.simulate_keystrokes("end enter");
    cx.simulate_input("x");
    assert_eq!(text(&state, cx), "fn f() {\n    x\n}");
}

#[gpui::test]
fn shift_arrows_select_and_typing_replaces(cx: &mut TestAppContext) {
    let (state, _view, cx) = open(cx, EditorState::editable("hello", None));
    cx.simulate_keystrokes("shift-right shift-right");
    state.read_with(cx, |s, _| assert_eq!(s.selection().range, 0..2));
    cx.simulate_input("J");
    assert_eq!(text(&state, cx), "Jllo");
}

#[gpui::test]
fn undo_and_redo_keys_round_trip(cx: &mut TestAppContext) {
    let (state, _view, cx) = open(cx, EditorState::editable("a", None));
    cx.simulate_keystrokes("end");
    cx.simulate_input("bc");
    cx.simulate_keystrokes("cmd-z");
    assert_eq!(text(&state, cx), "a");
    cx.simulate_keystrokes("cmd-shift-z");
    assert_eq!(text(&state, cx), "abc");
}

#[gpui::test]
fn cut_and_paste_go_through_the_clipboard(cx: &mut TestAppContext) {
    let (state, _view, cx) = open(cx, EditorState::editable("hello world", None));
    state.update(cx, |s, _| s.set_selection(0..6));
    cx.simulate_keystrokes("cmd-x");
    assert_eq!(text(&state, cx), "world");
    cx.simulate_keystrokes("end cmd-v");
    assert_eq!(text(&state, cx), "worldhello ");
}

/// Moving the caret below the fold scrolls the list to follow it.
#[gpui::test]
fn caret_motion_scrolls_vertically(cx: &mut TestAppContext) {
    let source = (0..400).map(|i| format!("line {i}")).collect::<Vec<_>>().join("\n");
    let (state, view, cx) = open(cx, EditorState::editable(&source, None));
    assert_eq!(scroll_offset(&view, cx).y, px(0.));
    for _ in 0..150 {
        cx.simulate_keystrokes("down");
    }
    assert_eq!(state.read_with(cx, |s, _| s.offset_to_point(s.selection().range.start).row), 150);
    assert!(scroll_offset(&view, cx).y < px(0.), "list should have scrolled down");
    for _ in 0..150 {
        cx.simulate_keystrokes("up");
    }
    assert_eq!(scroll_offset(&view, cx).y, px(0.));
}

/// Typing past the right edge scrolls horizontally; Home scrolls back.
#[gpui::test]
fn caret_motion_scrolls_horizontally(cx: &mut TestAppContext) {
    let (_state, view, cx) = open(cx, EditorState::editable(&"x".repeat(2000), None));
    assert_eq!(scroll_offset(&view, cx).x, px(0.));
    cx.simulate_keystrokes("end");
    cx.run_until_parked();
    assert!(scroll_offset(&view, cx).x < px(0.), "list should have scrolled right");
    cx.simulate_keystrokes("home");
    cx.run_until_parked();
    assert_eq!(scroll_offset(&view, cx).x, px(0.));
}

/// A programmatic selection far away + `scroll_to_caret` reveals it.
#[gpui::test]
fn scroll_to_caret_reveals_a_programmatic_selection(cx: &mut TestAppContext) {
    let source = (0..400).map(|i| format!("line {i}")).collect::<Vec<_>>().join("\n");
    let (state, view, cx) = open(cx, EditorState::editable(&source, None));
    state.update(cx, |s, _| {
        let end = s.len_chars();
        s.set_selection(end..end);
    });
    view.update(cx, |v, cx| v.scroll_to_caret(cx));
    cx.run_until_parked();
    assert!(scroll_offset(&view, cx).y < px(0.));
}
