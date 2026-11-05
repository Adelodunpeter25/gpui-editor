//! Hand-rolled vertical + horizontal scrollbars (raw gpui has no standalone
//! scrollbar widget; its `list()` element's is baked in and this editor uses
//! `uniform_list`). Both bars agree on one thumb-size formula.

use gpui::*;

use std::cell::Cell;
use std::rc::Rc;

const MIN_SCROLLBAR_THUMB: Pixels = px(24.0);

/// Thumb length for a scrollbar axis: viewport share of total content,
/// clamped to stay visible and inside the track. Shared by the vertical and
/// horizontal bars so both axes agree on the math.
pub fn scrollbar_thumb_size(track_len: Pixels, max_offset: Pixels) -> Pixels {
    let content_len = track_len + max_offset;
    if content_len <= px(0.) {
        return track_len;
    }
    let ratio = track_len / content_len;
    (track_len * ratio).max(MIN_SCROLLBAR_THUMB).min(track_len)
}

pub(crate) fn scrollbar_thumb_height(track_height: Pixels, max_offset_y: Pixels) -> Pixels {
    scrollbar_thumb_size(track_height, max_offset_y)
}

/// Thin draggable scrollbar for the editor's `uniform_list`, hand-rolled
/// since raw gpui has no standalone scrollbar widget (only baked into its
/// `list()` element, which this editor doesn't use).
pub(crate) fn render_scrollbar(
    scroll_handle: UniformListScrollHandle,
    thumb_dragging: Rc<Cell<Option<(Pixels, Pixels)>>>,
) -> impl IntoElement {
    let base = scroll_handle.0.borrow().base_handle.clone();
    let track_height = base.bounds().size.height;
    let max_offset_y = base.max_offset().y;
    let thumb_height = scrollbar_thumb_height(track_height, max_offset_y);
    let scroll_ratio = if max_offset_y > px(0.) {
        (-base.offset().y / max_offset_y).clamp(0., 1.)
    } else {
        0.
    };
    let thumb_top = (track_height - thumb_height).max(px(0.)) * scroll_ratio;

    div()
        .id("gpui-editor-scrollbar-track")
        .absolute()
        .top_0()
        .right_0()
        .w(px(8.))
        .h_full()
        .child(
            div()
                .id("gpui-editor-scrollbar-thumb")
                .absolute()
                .top(thumb_top)
                .right(px(1.))
                .w(px(6.))
                .h(thumb_height)
                .rounded_md()
                .bg(rgba(0xffffff33))
                .hover(|s| s.bg(rgba(0xffffff55)))
                .on_mouse_down(MouseButton::Left, move |event, _window, _cx| {
                    thumb_dragging.set(Some((event.position.y, base.offset().y)));
                }),
        )
}

/// Bottom scrollbar for the wrap-off horizontal range. Mirrors
/// `render_scrollbar` on the x axis; driven by the `uniform_list`'s own
/// `base_handle` — the same handle vertical scroll rides on (and the same
/// one rows read for hit-testing), so the thumb always agrees with what's
/// painted.
pub(crate) fn render_h_scrollbar(
    scroll_handle: ScrollHandle,
    thumb_dragging: Rc<Cell<Option<(Pixels, Pixels)>>>,
    v_scrollbar_visible: bool,
) -> impl IntoElement {
    // Leave the bottom-right corner to the vertical track (same width as
    // its own `w(px(8.))`) instead of the two bars overlapping there.
    let right_inset = if v_scrollbar_visible { px(8.) } else { px(0.) };
    let track_width = scroll_handle.bounds().size.width - right_inset;
    let max_offset_x = scroll_handle.max_offset().x;
    let thumb_width = scrollbar_thumb_size(track_width, max_offset_x);
    let scroll_ratio = if max_offset_x > px(0.) {
        (-scroll_handle.offset().x / max_offset_x).clamp(0., 1.)
    } else {
        0.
    };
    let thumb_left = (track_width - thumb_width).max(px(0.)) * scroll_ratio;

    div()
        .id("gpui-editor-hscrollbar-track")
        .absolute()
        .bottom_0()
        .left_0()
        .right(right_inset)
        .h(px(10.))
        .child(
            div()
                .id("gpui-editor-hscrollbar-thumb")
                .absolute()
                .left(thumb_left)
                .bottom(px(1.))
                .h(px(6.))
                .w(thumb_width)
                .rounded_md()
                .bg(rgba(0xffffff33))
                .hover(|s| s.bg(rgba(0xffffff55)))
                .on_mouse_down(MouseButton::Left, move |event, _window, _cx| {
                    thumb_dragging.set(Some((event.position.x, scroll_handle.offset().x)));
                }),
        )
}
