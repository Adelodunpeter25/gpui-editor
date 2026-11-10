use edit_buffer::Point;
use editor_ui::{comment_target_from, gutter_hit_test, EditorState, FontConfig, Mode};
use gpui::{px, AppContext as _, TestAppContext};

#[test]
fn comment_target_math_orders_and_clamps_lines() {
    assert_eq!(
        comment_target_from(Point { row: 8, col: 0 }, Point { row: 2, col: 0 }, 5),
        editor_ui::CommentTarget {
            start_line: 3,
            end_line: 5
        }
    );
    assert_eq!(
        comment_target_from(Point { row: 2, col: 0 }, Point { row: 2, col: 0 }, 5),
        editor_ui::CommentTarget {
            start_line: 3,
            end_line: 3
        }
    );
    assert_eq!(
        comment_target_from(Point { row: 0, col: 0 }, Point { row: 99, col: 0 }, 5),
        editor_ui::CommentTarget {
            start_line: 1,
            end_line: 5
        }
    );
}

#[test]
fn comment_gutter_hit_test_has_no_layout_shift() {
    assert!(gutter_hit_test(10.0, 8.0, 20.0));
    assert!(!gutter_hit_test(28.0, 8.0, 20.0));
}

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
fn select_line_selects_clamped_line_text_without_newline() {
    let mut s = EditorState::readonly("abc\ndef\nghi", Some(&syntax::RUST));

    s.select_line(2);
    assert_eq!(s.selected_text(), "def");
    s.select_line(0);
    assert_eq!(s.selected_text(), "abc");
    s.select_line(99);
    assert_eq!(s.selected_text(), "ghi");
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

#[test]
fn comment_selections_persist_and_dedupe() {
    let mut state = EditorState::readonly("a\nb\nc", Some(&syntax::RUST));
    let first = editor_ui::CommentTarget {
        start_line: 1,
        end_line: 2,
    };
    let second = editor_ui::CommentTarget {
        start_line: 3,
        end_line: 3,
    };

    state.add_comment_selection(first);
    state.add_comment_selection(first);
    state.add_comment_selection(second);
    assert_eq!(state.comment_selections(), &[first, second]);

    state.set_comment_selections(vec![second]);
    assert_eq!(state.comment_selections(), &[second]);
}

// -- edit mode (M2) ---------------------------------------------------------

/// `type_text` updates the buffer, bumps the version, and collapses the
/// selection onto the end of the inserted text.
#[gpui::test]
fn typing_updates_buffer_selection_and_version(cx: &mut TestAppContext) {
    let state = cx.update(|cx| cx.new(|_| EditorState::editable("hello", None)));
    state.update(cx, |s, cx| s.type_text(5..5, "!", cx));
    state.read_with(cx, |s, _| {
        assert_eq!(s.line_text(0), "hello!");
        assert_eq!(s.selection().range, 6..6);
        assert_eq!(s.version(), 1);
    });
}

/// Typing into a non-empty selection replaces it.
#[gpui::test]
fn typing_replaces_the_selection(cx: &mut TestAppContext) {
    let state = cx.update(|cx| cx.new(|_| EditorState::editable("hello world", None)));
    state.update(cx, |s, cx| {
        s.set_selection(5..11);
        s.type_text(5..11, " gpui", cx);
    });
    state.read_with(cx, |s, _| {
        assert_eq!(s.line_text(0), "hello gpui");
        assert_eq!(s.selection().range, 10..10);
    });
}

/// Readonly ignores both edit primitives — content, version, selection.
#[gpui::test]
fn readonly_ignores_replace_range_and_type_text(cx: &mut TestAppContext) {
    let state = cx.update(|cx| cx.new(|_| EditorState::readonly("hello", None)));
    state.update(cx, |s, cx| {
        s.type_text(5..5, "!", cx);
        s.replace_range(0..5, "bye", cx);
    });
    state.read_with(cx, |s, _| {
        assert_eq!(s.line_text(0), "hello");
        assert_eq!(s.version(), 0);
    });
}

#[gpui::test]
fn undo_redo_round_trips_a_typing_run(cx: &mut TestAppContext) {
    let state = cx.update(|cx| cx.new(|_| EditorState::editable("hello", None)));
    state.update(cx, |s, cx| {
        s.type_text(5..5, " ", cx);
        s.type_text(6..6, "world", cx);
    });
    state.read_with(cx, |s, _| assert_eq!(s.line_text(0), "hello world"));
    state.update(cx, |s, cx| s.undo(cx));
    state.read_with(cx, |s, _| {
        // Both typing keystrokes landed inside one 500ms run: one undo.
        assert_eq!(s.line_text(0), "hello");
        assert!(s.can_redo());
    });
    state.update(cx, |s, cx| s.redo(cx));
    state.read_with(cx, |s, _| assert_eq!(s.line_text(0), "hello world"));
}

/// A structural edit (`replace_range`) between keystrokes closes the typing
/// run, so undo stops there instead of reverting through it.
#[gpui::test]
fn structural_edit_closes_the_typing_run(cx: &mut TestAppContext) {
    let state = cx.update(|cx| cx.new(|_| EditorState::editable("hello", None)));
    state.update(cx, |s, cx| {
        s.type_text(5..5, "a", cx);
        s.replace_range(0..1, "H", cx); // structural: own undo entry
        s.type_text(6..6, "b", cx);
    });
    state.update(cx, |s, cx| s.undo(cx));
    state.read_with(cx, |s, _| assert_eq!(s.line_text(0), "Helloa"));
    state.update(cx, |s, cx| s.undo(cx));
    state.read_with(cx, |s, _| assert_eq!(s.line_text(0), "helloa"));
    state.update(cx, |s, cx| s.undo(cx));
    state.read_with(cx, |s, _| {
        assert_eq!(s.line_text(0), "hello");
        assert!(!s.can_undo());
    });
}

/// Typing past the end of a line grows the widest-row width (content-width
/// cache is keyed by buffer version, so h-scroll range follows edits), and
/// undo shrinks it back.
#[gpui::test]
fn editing_grows_and_undo_shrinks_max_line_width(cx: &mut TestAppContext) {
    let state = cx.update(|cx| cx.new(|_| EditorState::editable("ab\nabc", None)));
    let cw = px(10.0);
    state.read_with(cx, |s, _| assert_eq!(f32::from(s.max_line_width(cw)), 30.0));
    let v0 = state.read_with(cx, |s, _| s.version());
    state.update(cx, |s, cx| {
        s.set_selection(6..6);
        s.type_text(6..6, "abcdef", cx);
    });
    state.read_with(cx, |s, _| {
        assert_eq!(f32::from(s.max_line_width(cw)), 90.0);
        assert_ne!(s.version(), v0);
    });
    state.update(cx, |s, cx| s.undo(cx));
    state.read_with(cx, |s, _| assert_eq!(f32::from(s.max_line_width(cw)), 30.0));
}

// -- newline_edit (pure helper) ---------------------------------------------

#[test]
fn newline_edit_splits_mid_line_without_indent() {
    let s = EditorState::editable("abc", None);
    let (range, text) = s.newline_edit(1);
    assert_eq!(range, 1..1);
    assert_eq!(text, "\n");
}

/// Enter at a line's end inserts at the caret, so the new line opens
/// directly below and the following line is left untouched.
#[test]
fn newline_edit_at_line_end_inserts_at_the_caret() {
    let s = EditorState::editable("abc\nxyz", None);
    // Caret at offset 3 = visual end of line 0 (before its '\n').
    let (range, text) = s.newline_edit(3);
    assert_eq!(range, 3..3);
    assert_eq!(text, "\n");
}

#[test]
fn newline_edit_auto_indents_after_an_opener() {
    let s = EditorState::editable("    foo {\n}", None);
    let (range, text) = s.newline_edit(9); // end of "    foo {"
    assert_eq!(range, 9..9);
    assert_eq!(text, "\n        "); // base 4 + one 4-space level
}

/// The opener check only looks at text before the caret: splitting ahead
/// of the `{` keeps the base indent without adding a level.
#[test]
fn newline_edit_mid_line_ignores_an_opener_after_the_caret() {
    let s = EditorState::editable("    foo {", None);
    let (range, text) = s.newline_edit(7); // between "foo" and " {"
    assert_eq!(range, 7..7);
    assert_eq!(text, "\n    ");
}

/// End to end: Enter after `{` opens an indented line and leaves the
/// closing brace where it was, with the caret on the new line.
#[gpui::test]
fn enter_after_an_opener_leaves_the_next_line_alone(cx: &mut TestAppContext) {
    let state = cx.update(|cx| cx.new(|_| EditorState::editable("    foo {\n}", None)));
    state.update(cx, |s, cx| {
        let (range, text) = s.newline_edit(9);
        s.replace_range(range, &text, cx);
    });
    state.read_with(cx, |s, _| {
        assert_eq!(s.text(), "    foo {\n        \n}");
        assert_eq!(s.selection().range, 18..18);
        assert_eq!(s.offset_to_point(18), Point { row: 1, col: 8 });
    });
}

// -- offset helpers -----------------------------------------------------------

#[test]
fn line_end_offset_excludes_the_newline() {
    let s = EditorState::editable("ab\ncd\n", None);
    assert_eq!(s.line_end_offset(0), 2);
    assert_eq!(s.line_end_offset(1), 5);
    // Last line ("" after the trailing newline): end of text.
    assert_eq!(s.line_end_offset(2), 6);
    assert_eq!(s.len_chars(), 6);
}

// -- repaint ------------------------------------------------------------------

/// Counts how many times `state` notifies its observers while `f` runs.
fn notifications(
    cx: &mut TestAppContext,
    state: &gpui::Entity<EditorState>,
    f: impl FnOnce(&mut EditorState, &mut gpui::Context<EditorState>),
) -> usize {
    let count = std::rc::Rc::new(std::cell::Cell::new(0usize));
    let seen = count.clone();
    let subscription = cx.update(|cx| {
        cx.observe(state, move |_, _| seen.set(seen.get() + 1))
    });
    state.update(cx, f);
    cx.run_until_parked();
    drop(subscription);
    count.get()
}

/// Typing must notify observers, or the view never repaints the new text.
#[gpui::test]
fn typing_notifies_observers(cx: &mut TestAppContext) {
    let state = cx.update(|cx| cx.new(|_| EditorState::editable("hello", None)));
    assert!(notifications(cx, &state, |s, cx| s.type_text(5..5, "!", cx)) > 0);
    assert!(notifications(cx, &state, |s, cx| s.replace_range(0..1, "H", cx)) > 0);
}

#[gpui::test]
fn undo_redo_and_set_text_notify_observers(cx: &mut TestAppContext) {
    let state = cx.update(|cx| cx.new(|_| EditorState::editable("hello", None)));
    state.update(cx, |s, cx| s.type_text(5..5, "!", cx));
    assert!(notifications(cx, &state, |s, cx| s.undo(cx)) > 0);
    assert!(notifications(cx, &state, |s, cx| s.redo(cx)) > 0);
    assert!(notifications(cx, &state, |s, cx| s.set_text("new", cx)) > 0);
}

/// Readonly edits are no-ops, so they must not trigger a repaint either.
#[gpui::test]
fn readonly_edit_does_not_notify(cx: &mut TestAppContext) {
    let state = cx.update(|cx| cx.new(|_| EditorState::readonly("hello", None)));
    assert_eq!(notifications(cx, &state, |s, cx| s.type_text(5..5, "!", cx)), 0);
}

// -- undo grouping ------------------------------------------------------------

/// Type, then backspace, inside one run: undo restores the original text.
#[gpui::test]
fn undo_restores_text_after_a_type_then_backspace_run(cx: &mut TestAppContext) {
    let state = cx.update(|cx| cx.new(|_| EditorState::editable("ab", None)));
    state.update(cx, |s, cx| {
        s.type_text(2..2, "c", cx);
        s.type_text(3..3, "d", cx);
        s.type_text(3..4, "", cx);
        s.type_text(2..3, "", cx);
    });
    state.update(cx, |s, cx| s.undo(cx));
    state.read_with(cx, |s, _| assert_eq!(s.text(), "ab"));
}

/// Moving the caret (click / arrow) between keystrokes splits the typing
/// run, so each side undoes on its own.
#[gpui::test]
fn moving_the_caret_splits_the_typing_run(cx: &mut TestAppContext) {
    let state = cx.update(|cx| cx.new(|_| EditorState::editable("hello", None)));
    state.update(cx, |s, cx| {
        s.type_text(5..5, "!", cx);
        s.set_selection(0..0);
        s.type_text(0..0, ">", cx);
    });
    state.read_with(cx, |s, _| assert_eq!(s.text(), ">hello!"));
    state.update(cx, |s, cx| s.undo(cx));
    state.read_with(cx, |s, _| assert_eq!(s.text(), "hello!"));
    state.update(cx, |s, cx| s.undo(cx));
    state.read_with(cx, |s, _| assert_eq!(s.text(), "hello"));
}
