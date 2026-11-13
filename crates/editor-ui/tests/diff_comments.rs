//! Diff comment gutter: side/range rules and the opt-in `DiffView` API.

use editor_ui::{
    diff_comment_target, DiffCommentTarget, DiffRowOverlay, DiffSide, DiffState, DiffView,
};
use gpui::{AppContext as _, IntoElement, ParentElement, TestAppContext};
use gpui::{div, Styled};
use std::cell::RefCell;
use std::rc::Rc;

// old: a b c   new: a X c d  ->  rows: ctx a(1,1) | rm b(2,-) | add X(-,2) | ctx c(3,3) | add d(-,4)
fn lines() -> Vec<editor_ui::DiffLine> {
    diff::diff_lines("a\nb\nc\n", "a\nX\nc\nd\n").lines
}

fn t(side: DiffSide, s: u32, e: u32) -> Option<DiffCommentTarget> {
    Some(DiffCommentTarget { side, start_line: s, end_line: e })
}

#[test]
fn removed_row_uses_old_line() {
    assert_eq!(diff_comment_target(&lines(), 1, 1), t(DiffSide::Old, 2, 2));
}

#[test]
fn added_and_context_rows_use_new_line() {
    assert_eq!(diff_comment_target(&lines(), 2, 2), t(DiffSide::New, 2, 2));
    assert_eq!(diff_comment_target(&lines(), 0, 0), t(DiffSide::New, 1, 1));
}

#[test]
fn drag_clamps_to_starting_side() {
    // Start on added row 2 and drag down through ctx/add: new 2..4.
    assert_eq!(diff_comment_target(&lines(), 2, 4), t(DiffSide::New, 2, 4));
    // Start on the removed row: only rows with an old line count (b, c).
    assert_eq!(diff_comment_target(&lines(), 1, 4), t(DiffSide::Old, 2, 3));
}

#[test]
fn drag_upward_is_ordered() {
    assert_eq!(diff_comment_target(&lines(), 4, 2), t(DiffSide::New, 2, 4));
}

#[test]
fn out_of_range_start_is_none() {
    assert_eq!(diff_comment_target(&lines(), 99, 99), None);
}

#[gpui::test]
fn diff_view_renders_with_gutter_selections_and_overlays(cx: &mut TestAppContext) {
    let state = cx.update(|cx| cx.new(|_| DiffState::new("a\nb\nc\n", "a\nX\nc\nd\n", None)));
    let (view, vcx) = cx.add_window_view(|_, _| DiffView::new(&state));
    let picked: Rc<RefCell<Vec<DiffCommentTarget>>> = Rc::default();
    view.update(vcx, |v, cx| {
        let p = picked.clone();
        v.on_comment_target(move |t| p.borrow_mut().push(t));
        v.set_comment_selections(
            vec![DiffCommentTarget { side: DiffSide::New, start_line: 2, end_line: 3 }],
            cx,
        );
        v.set_row_overlays(
            vec![
                DiffRowOverlay::new(DiffSide::New, 2, 3, |_, _| {
                    div().h_full().child("card").into_any_element()
                }),
                // Unknown anchor line: ignored, must not panic.
                DiffRowOverlay::new(DiffSide::Old, 99, 1, |_, _| div().into_any_element()),
            ],
            cx,
        );
    });
    vcx.run_until_parked();
    vcx.simulate_resize(gpui::size(gpui::px(800.), gpui::px(400.)));
    vcx.run_until_parked();
    // Nothing is reported until a gutter gesture completes.
    assert!(picked.borrow().is_empty());
}
