use editor_ui::{scrollbar_thumb_size, DiffState, EditorState};
use gpui::px;

fn width(state: &EditorState, cw: f32) -> f32 {
    f32::from(state.max_line_width(px(cw)))
}

#[test]
fn max_line_width_scales_with_char_count() {
    let s = EditorState::readonly("abc", None);
    assert_eq!(width(&s, 10.0), 30.0);
}

#[test]
fn max_line_width_empty_buffer_is_zero() {
    let s = EditorState::readonly("", None);
    assert_eq!(width(&s, 10.0), 0.0);
}

#[test]
fn max_line_width_honors_tab_stops() {
    // Default tab width is 4: "\ta" is one 40px tab stop plus one 10px char.
    let s = EditorState::readonly("\ta", None);
    assert_eq!(width(&s, 10.0), 50.0);
}

#[test]
fn max_line_width_picks_the_longest_row() {
    let s = EditorState::readonly("ab\nabcde\nabc", None);
    assert_eq!(width(&s, 10.0), 50.0);
}

#[test]
fn max_line_width_caps_at_the_render_limit() {
    // Rows render at most 4000 chars, so width must describe that prefix —
    // otherwise dead scroll range trails every pathological line.
    let s = EditorState::readonly(&"x".repeat(5000), None);
    assert_eq!(width(&s, 10.0), 40000.0);
}

#[test]
fn diff_max_line_width_picks_the_longest_line() {
    let d = DiffState::new("ab", "abcde", None);
    assert_eq!(f32::from(d.max_line_width(px(10.0))), 50.0);
}

#[test]
fn scrollbar_thumb_is_full_track_without_overflow() {
    assert_eq!(f32::from(scrollbar_thumb_size(px(100.0), px(0.0))), 100.0);
}

#[test]
fn scrollbar_thumb_shrinks_with_content() {
    // Track 100, overflow 100: viewport shows half the content.
    assert_eq!(f32::from(scrollbar_thumb_size(px(100.0), px(100.0))), 50.0);
}

#[test]
fn scrollbar_thumb_never_shrinks_below_minimum() {
    assert_eq!(f32::from(scrollbar_thumb_size(px(100.0), px(10000.0))), 24.0);
}
