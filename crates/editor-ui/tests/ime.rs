//! Platform text-input (IME) tests: drive `EditorView`'s
//! `EntityInputHandler` the way the OS input method does — UTF-16 ranges,
//! provisional "marked" text, then a commit.

use editor_ui::{EditorState, EditorView};
use gpui::{
    point, px, AppContext as _, Bounds, Entity, EntityInputHandler, Pixels, TestAppContext,
    VisualTestContext,
};
use std::ops::Range;

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

/// Composition update, as the input method sends it.
fn compose(
    view: &Entity<EditorView>,
    cx: &mut VisualTestContext,
    range: Option<Range<usize>>,
    text: &str,
    selected: Option<Range<usize>>,
) {
    cx.update(|window, cx| {
        view.update(cx, |v, cx| {
            v.replace_and_mark_text_in_range(range, text, selected, window, cx)
        })
    });
}

/// Commit (or plain insertion), as the input method sends it.
fn commit(view: &Entity<EditorView>, cx: &mut VisualTestContext, range: Option<Range<usize>>, text: &str) {
    cx.update(|window, cx| {
        view.update(cx, |v, cx| v.replace_text_in_range(range, text, window, cx))
    });
}

fn marked_utf16(view: &Entity<EditorView>, cx: &mut VisualTestContext) -> Option<Range<usize>> {
    cx.update(|window, cx| view.update(cx, |v, cx| v.marked_text_range(window, cx)))
}

fn selected_utf16(view: &Entity<EditorView>, cx: &mut VisualTestContext) -> Range<usize> {
    cx.update(|window, cx| {
        view.update(cx, |v, cx| v.selected_text_range(false, window, cx))
    })
    .expect("editor always reports a selection")
    .range
}

/// A dead-key accent: provisional "´" is marked, then replaced by "é".
#[gpui::test]
fn composition_marks_then_commits(cx: &mut TestAppContext) {
    let (state, view, cx) = open(cx, EditorState::editable("caf", None));
    state.update(cx, |s, _| s.set_selection(3..3));

    compose(&view, cx, None, "´", Some(1..1));
    assert_eq!(text(&state, cx), "caf´");
    assert_eq!(marked_utf16(&view, cx), Some(3..4));
    state.read_with(cx, |s, _| assert_eq!(s.marked_range(), Some(3..4)));

    commit(&view, cx, None, "é");
    assert_eq!(text(&state, cx), "café");
    assert_eq!(marked_utf16(&view, cx), None);
    state.read_with(cx, |s, _| assert_eq!(s.selection().range, 4..4));
}

/// Each composition update replaces the previous provisional text, and the
/// whole composition is one undo entry.
#[gpui::test]
fn composition_updates_replace_and_undo_as_one(cx: &mut TestAppContext) {
    let (state, view, cx) = open(cx, EditorState::editable("", None));
    compose(&view, cx, None, "n", None);
    compose(&view, cx, None, "ni", None);
    compose(&view, cx, None, "你", None);
    assert_eq!(text(&state, cx), "你");
    commit(&view, cx, None, "你好");
    assert_eq!(text(&state, cx), "你好");

    state.update(cx, |s, cx| s.undo(cx));
    assert_eq!(text(&state, cx), "");
    state.read_with(cx, |s, _| assert!(!s.can_undo()));
    state.update(cx, |s, cx| s.redo(cx));
    assert_eq!(text(&state, cx), "你好");
}

/// Cancelling (empty marked text) removes the provisional text.
#[gpui::test]
fn empty_marked_text_cancels_the_composition(cx: &mut TestAppContext) {
    let (state, view, cx) = open(cx, EditorState::editable("ab", None));
    state.update(cx, |s, _| s.set_selection(1..1));
    compose(&view, cx, None, "xyz", None);
    assert_eq!(text(&state, cx), "axyzb");
    compose(&view, cx, None, "", None);
    assert_eq!(text(&state, cx), "ab");
    assert_eq!(marked_utf16(&view, cx), None);
    state.read_with(cx, |s, _| assert_eq!(s.selection().range, 1..1));
}

/// `unmark_text` keeps the text and ends the composition.
#[gpui::test]
fn unmark_keeps_the_text(cx: &mut TestAppContext) {
    let (state, view, cx) = open(cx, EditorState::editable("", None));
    compose(&view, cx, None, "abc", None);
    cx.update(|window, cx| view.update(cx, |v, cx| v.unmark_text(window, cx)));
    assert_eq!(text(&state, cx), "abc");
    assert_eq!(marked_utf16(&view, cx), None);
    // Next input inserts at the caret instead of replacing "abc".
    commit(&view, cx, None, "d");
    assert_eq!(text(&state, cx), "abcd");
}

/// The input method's selection inside the marked text is honored.
#[gpui::test]
fn composition_selection_is_relative_to_the_marked_text(cx: &mut TestAppContext) {
    let (state, view, cx) = open(cx, EditorState::editable("ab", None));
    state.update(cx, |s, _| s.set_selection(2..2));
    compose(&view, cx, None, "xyz", Some(1..2));
    state.read_with(cx, |s, _| assert_eq!(s.selection().range, 3..4));
}

/// Ranges cross the boundary in UTF-16: "😀" is one char, two code units.
#[gpui::test]
fn ranges_are_utf16_on_the_platform_side(cx: &mut TestAppContext) {
    let (state, view, cx) = open(cx, EditorState::editable("a😀b", None));
    state.update(cx, |s, _| s.set_selection(2..3)); // "b"
    assert_eq!(selected_utf16(&view, cx), 3..4);

    let mut adjusted = None;
    let got = cx.update(|window, cx| {
        view.update(cx, |v, cx| v.text_for_range(1..3, &mut adjusted, window, cx))
    });
    assert_eq!(got.as_deref(), Some("😀"));
    assert_eq!(adjusted, Some(1..3));

    // Replace "b" (UTF-16 3..4) with an emoji; caret lands after it.
    commit(&view, cx, Some(3..4), "🎉");
    assert_eq!(text(&state, cx), "a😀🎉");
    assert_eq!(selected_utf16(&view, cx), 5..5);

    // Marked selection offsets are UTF-16 too: caret after the emoji.
    compose(&view, cx, None, "🙂x", Some(2..2));
    state.read_with(cx, |s, _| assert_eq!(s.selection().range, 4..4));
}

/// Readonly editors refuse text input entirely.
#[gpui::test]
fn readonly_rejects_platform_input(cx: &mut TestAppContext) {
    let (state, view, cx) = open(cx, EditorState::readonly("abc", None));
    let accepts = cx.update(|window, cx| view.update(cx, |v, cx| v.accepts_text_input(window, cx)));
    assert!(!accepts);
    commit(&view, cx, None, "x");
    compose(&view, cx, None, "y", None);
    assert_eq!(text(&state, cx), "abc");
    assert_eq!(marked_utf16(&view, cx), None);
}

/// The candidate window anchors at the caret: further along the line is
/// further right, a later line is further down, and the reported point
/// maps back to the same character.
#[gpui::test]
fn caret_bounds_track_the_text_position(cx: &mut TestAppContext) {
    let (_state, view, cx) = open(cx, EditorState::editable("hello\nworld", None));
    let element = Bounds::new(point(px(0.), px(0.)), gpui::size(px(800.), px(600.)));
    let bounds = |cx: &mut VisualTestContext, range: Range<usize>| -> Bounds<Pixels> {
        cx.update(|window, cx| {
            view.update(cx, |v, cx| v.bounds_for_range(range, element, window, cx))
        })
        .expect("bounds for a valid range")
    };
    let start = bounds(cx, 0..0);
    let mid = bounds(cx, 3..3);
    let next_line = bounds(cx, 9..9); // "wor|ld"
    assert!(mid.origin.x > start.origin.x);
    assert_eq!(mid.origin.y, start.origin.y);
    assert!(next_line.origin.y > start.origin.y);
    assert_eq!(next_line.origin.x, mid.origin.x);
    assert!(start.size.height > px(0.) && start.size.width > px(0.));

    let hit = point(next_line.origin.x + px(1.), next_line.origin.y + px(1.));
    let index = cx.update(|window, cx| {
        view.update(cx, |v, cx| v.character_index_for_point(hit, window, cx))
    });
    assert_eq!(index, Some(9));
}

/// Typed characters reach the buffer through the input handler (this is
/// the path a real keyboard takes), coalescing into one undo entry.
#[gpui::test]
fn typed_keys_arrive_through_the_input_handler(cx: &mut TestAppContext) {
    let (state, _view, cx) = open(cx, EditorState::editable("", None));
    cx.simulate_input("abc");
    assert_eq!(text(&state, cx), "abc");
    state.update(cx, |s, cx| s.undo(cx));
    assert_eq!(text(&state, cx), "");
}
