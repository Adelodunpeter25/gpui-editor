//! Per-row painting: line-number gutter, syntax/selection runs, the caret,
//! and the comment-gutter chip. Pure element builders — the `Render` impl
//! (`render.rs`) decides which rows get built and what state they read.

use gpui::prelude::FluentBuilder;
use gpui::*;

use std::ops::Range;
use std::rc::Rc;

use edit_buffer::Point;
use syntax::Capture;

use crate::view::RowInteraction;

use crate::measure::{column_for_x, measure_char_width, text_display_width};
use crate::types::{FontConfig, StyledSpan};

/// Clip the whole-buffer selection range to `row_start..row_end`, in row-local
/// char coords. `None` if the row has no overlap with the selection.
pub(crate) fn row_local_selection(
    selection: &Range<usize>,
    row_start: usize,
    row_end: usize,
) -> Option<Range<usize>> {
    let lo = selection.start.max(row_start);
    let hi = selection.end.min(row_end);
    if lo < hi {
        Some(lo - row_start..hi - row_start)
    } else {
        None
    }
}

pub(crate) fn resolve_color(span: &StyledSpan) -> Hsla {
    span.color
        .map(|(r, g, b)| rgb(((r as u32) << 16) | ((g as u32) << 8) | b as u32).into())
        .unwrap_or_else(|| color_for(span.capture))
}

pub(crate) fn color_for(capture: Capture) -> Hsla {
    match capture {
        Capture::Keyword => rgb(0x89b4fa).into(),
        Capture::String => rgb(0xa6e3a1).into(),
        Capture::Comment => rgb(0x6c7086).into(),
        Capture::Number => rgb(0xfab387).into(),
        Capture::Function => rgb(0x89dceb).into(),
        Capture::Type => rgb(0xf9e2af).into(),
        Capture::Plain => rgb(0xcdd6f4).into(),
    }
}

/// Selection wash color: translucent blue behind selected glyphs.
const SELECTION_BG: u32 = 0x3b82f680;

/// Bundles a rendered row's positional bookkeeping (as opposed to its
/// content/styling) into one value — keeps `render_line`'s own argument
/// count from creeping past what a single glance can take in.
pub(crate) struct RowMeta {
    pub(crate) row: u32,
    pub(crate) show_line_number: bool,
    pub(crate) col_offset: u32,
    /// Where this row's text content starts, x-wise (gutter + padding).
    pub(crate) content_x: Pixels,
    /// Extra left padding for a wrapped continuation line, so it aligns
    /// under its own indent instead of restarting at column 0. `0` for a
    /// row's first sub-line.
    pub(crate) indent_px: Pixels,
    /// Gutter's left edge in row-local coordinates — moves with the row
    /// under h-scroll like everything else (the container clips it, the
    /// gutter isn't pinned or hidden).
    pub(crate) gutter_pad: Pixels,
    /// Gutter width (`w_10`); text starts at `gutter_pad + gutter_w + gap`
    /// = `content_x` in row coordinates.
    pub(crate) gutter_w: Pixels,
    /// `Some(content width)` while h-scroll is active (the `content_width`
    /// local in `Render`), `None` otherwise. The list itself is
    /// *viewport*-width when h-scrolling (see `Render`'s comment on why —
    /// gpui clamps the scroll offset against the scroll container's own
    /// bounds, so a content-width list has zero horizontal scroll range),
    /// so each row states its full content width explicitly instead of
    /// stretching to fill the list. That keeps row layout byte-for-byte
    /// what it was when the list *was* content-width: text laid out once at
    /// its real width (never re-wrapped at the viewport edge), and the
    /// width gpui measures for its scroll range matches `content_width`,
    /// trailing pad included.
    pub(crate) row_width: Option<Pixels>,
    pub(crate) comment_enabled: bool,
    /// Inside a persisted comment range (highlighted after the drag ends).
    pub(crate) comment_selected: bool,
    pub(crate) comment_hover_row: Option<u32>,
    pub(crate) comment_drag: Option<u32>,
    /// Caret column within this sub-line's chars, when the caret is on this
    /// sub-line and the editor is focused + editable (empty selection).
    pub(crate) caret_col: Option<usize>,
    /// In-progress IME composition clipped to this sub-line's chars —
    /// underlined so provisional text reads as not yet committed.
    pub(crate) marked: Option<Range<usize>>,
    /// Measured monospace glyph advance — drives the caret's x position.
    pub(crate) char_width: Pixels,
    pub(crate) tab_width: u32,
}

pub(crate) fn render_line(
    meta: RowMeta,
    text: String,
    runs: Vec<(Range<usize>, Hsla)>,
    selection: Option<Range<usize>>,
    font: FontConfig,
    interaction: RowInteraction,
) -> impl IntoElement {
    let RowMeta {
        row,
        show_line_number,
        col_offset,
        content_x,
        indent_px,
        gutter_pad,
        gutter_w,
        row_width,
        comment_enabled,
        comment_selected,
        comment_hover_row,
        comment_drag,
        caret_col,
        marked,
        char_width,
        tab_width,
    } = meta;
    let line_no = if show_line_number {
        format!("{:>4}", row + 1)
    } else {
        String::new()
    };
    let line_height = font.line_height;
    // Comment targets intentionally use a stable review yellow, independent
    // of syntax scope colors. This keeps the annotation affordance and the
    // selected range recognizable when a file theme changes.
    let comment_accent = rgba(0xfacc1538);
    // Continuation sub-lines are left-padded by `indent_px` so a wrapped
    // line visually aligns under its own indent (see `WrapCache::rebuild`);
    // hit-testing has to account for that same offset or clicks would land
    // on the wrong character on any indented, wrapped line.
    let hit_test_x = content_x + indent_px;

    let down = interaction.clone();
    let down_font = font.clone();
    let down_text = Rc::new(text.clone());
    let move_ = interaction.clone();
    let move_font = font.clone();
    let move_text = down_text.clone();

    // Caret: a 2px bar at the head column, tab stops honored, painted after
    // the text so it's visible over any run/selection background. No blink
    // in v1 — always on while the editor is focused.
    let caret_el = caret_col.map(|cc| {
        let prefix: String = text.chars().take(cc).collect();
        let caret_x = text_display_width(&prefix, char_width, tab_width);
        div()
            .absolute()
            .top_0()
            .left(indent_px + caret_x)
            .w(px(2.0))
            .h_full()
            .bg(rgba(0xffffffcc))
            .rounded_xs()
    });

    let marked_el = marked.map(|range| {
        let before: String = text.chars().take(range.start).collect();
        let through: String = text.chars().take(range.end).collect();
        let start_x = text_display_width(&before, char_width, tab_width);
        let end_x = text_display_width(&through, char_width, tab_width);
        div()
            .absolute()
            .bottom(px(1.0))
            .left(indent_px + start_x)
            .w(end_x - start_x)
            .h(px(1.0))
            .bg(rgba(0xffffffcc))
    });

    let row_el = div()
        .id(("editor-line", row as usize))
        .relative()
        .flex()
        .flex_row()
        .items_center()
        .h(line_height);
    // While h-scrolling, the row outgrows the (viewport-width) list on
    // purpose — see `RowMeta::row_width`. Otherwise it stretches to the
    // list as always.
    let row_el = match row_width {
        Some(w) => row_el.w(w),
        None => row_el.w_full(),
    };
    let row_el = row_el.when(
        (comment_enabled || comment_selected)
            && (comment_selected
                || comment_drag
                    .zip(comment_hover_row)
                    .is_some_and(|(a, b)| row >= a.min(b) && row <= a.max(b))),
        |el| el.bg(comment_accent),
    );
    row_el
        .font_family(font.family.clone())
        .text_size(font.size)
        .on_mouse_move({
            let interaction = interaction.clone();
            move |_event, window, _cx| {
                if comment_enabled && interaction.comment_hover_row.get() != Some(row) {
                    interaction.comment_hover_row.set(Some(row));
                    window.refresh();
                }
            }
        })
        .on_mouse_exit({
            let interaction = interaction.clone();
            move |_event, window, _cx| {
                if interaction.comment_drag.get().is_none() {
                    interaction.comment_hover_row.set(None);
                    window.refresh();
                }
            }
        })
        .child(
            div()
                .id(("editor-line-content", row as usize))
                .relative()
                .flex()
                .flex_row()
                .flex_1()
                .flex_shrink_0()
                .ml(content_x)
                .pl(indent_px)
                .child(render_spans(text, runs, selection))
                .children(marked_el)
                .children(caret_el)
                .on_mouse_down(MouseButton::Left, move |event, window, cx| {
                    let width = measure_char_width(&down.char_width, &down_font, window);
                    let scroll_x = (-down.h_handle.offset().x).max(px(0.));
                    let local_x = event.position.x - hit_test_x + scroll_x;
                    let tab_width = down.state.read(cx).tab_width();
                    let col = col_offset + column_for_x(local_x, width, &down_text, tab_width);
                    let offset = down.state.read(cx).point_to_offset(Point { row, col });
                    down.drag_anchor.set(Some(offset));
                    down.last_head.set(None);
                    down.selecting.set(true);
                    // A click is caret placement, not keyboard extension.
                    down.key_anchor.set(None);
                    down.goal_col.set(None);
                    down.state.update(cx, |state, cx| {
                        state.set_selection(offset..offset);
                        cx.notify();
                    });
                })
                .on_mouse_move(move |event, window, cx| {
                    if !move_.selecting.get() {
                        return;
                    }
                    let Some(anchor) = move_.drag_anchor.get() else {
                        return;
                    };
                    let width = measure_char_width(&move_.char_width, &move_font, window);
                    let scroll_x = (-move_.h_handle.offset().x).max(px(0.));
                    let local_x = event.position.x - hit_test_x + scroll_x;
                    let tab_width = move_.state.read(cx).tab_width();
                    let col = col_offset + column_for_x(local_x, width, &move_text, tab_width);
                    let head = move_.state.read(cx).point_to_offset(Point { row, col });
                    if move_.last_head.get() == Some(head) {
                        return;
                    }
                    let range = if anchor <= head {
                        anchor..head
                    } else {
                        head..anchor
                    };
                    move_.state.update(cx, |state, cx| {
                        state.set_selection(range);
                        cx.notify();
                    });
                    move_.last_head.set(Some(head));
                }),
        )
        // Gutter: plain row content — it rides the row's horizontal
        // translation and the container's left edge clips it as it scrolls
        // out of view (no conditional unmount, no pinned overlay). Opaque
        // background keeps the number column a clean block over the
        // editor's black; painted after content so its right border rule
        // always wins the seam.
        .child(
            div()
                .absolute()
                .left(gutter_pad)
                .top_0()
                .w(gutter_w)
                .h_full()
                .bg(Hsla::black())
                .border_r_2()
                .border_color(rgba(0xffffff1a))
                .flex()
                .items_center()
                .justify_end()
                .pr_1()
                .text_color(rgb(0x585b70))
                .child(line_no)
                .when(
                    comment_enabled
                        && (comment_hover_row == Some(row) || comment_drag == Some(row)),
                    |el| {
                        el.child(
                            div()
                                .id(("comment-plus", row as usize))
                                .absolute()
                                .left(px(2.0))
                                .top_0()
                                .h_full()
                                .flex()
                                .items_center()
                                .text_color(rgb(0xcdd6f4))
                                .text_size(px(16.0))
                                .child("+"),
                        )
                    },
                )
                .on_mouse_down(MouseButton::Left, {
                    let drag = interaction.comment_drag.clone();
                    let hover = interaction.comment_hover_row.clone();
                    let selecting = interaction.selecting.clone();
                    move |_event, window, _cx| {
                        selecting.set(false);
                        hover.set(Some(row));
                        drag.set(Some(row));
                        window.refresh();
                    }
                }),
        )
}

pub(crate) fn render_spans(
    text: String,
    runs: Vec<(Range<usize>, Hsla)>,
    selection: Option<Range<usize>>,
) -> impl IntoElement {
    let chars: Vec<char> = text.chars().collect();
    let len = chars.len();
    if len == 0 {
        return div().child(" ".to_string()).into_any_element();
    }

    // Merge syntax-run and selection edges into one sorted breakpoint list so
    // every resulting segment has a single color + selected flag.
    let mut breaks: Vec<usize> = vec![0, len];
    for (range, _) in &runs {
        breaks.push(range.start.min(len));
        breaks.push(range.end.min(len));
    }
    if let Some(sel) = &selection {
        breaks.push(sel.start.min(len));
        breaks.push(sel.end.min(len));
    }
    breaks.sort_unstable();
    breaks.dedup();

    let mut children: Vec<AnyElement> = Vec::new();
    for pair in breaks.windows(2) {
        let (lo, hi) = (pair[0], pair[1]);
        if lo >= hi {
            continue;
        }
        let color = runs
            .iter()
            .find(|(range, _)| range.start <= lo && hi <= range.end)
            .map(|(_, color)| *color)
            .unwrap_or_else(|| rgb(0xcdd6f4).into());
        let selected = selection
            .as_ref()
            .is_some_and(|sel| sel.start <= lo && hi <= sel.end);
        let word: String = chars[lo..hi].iter().collect();
        let mut el = div().text_color(color);
        if selected {
            el = el.bg(rgba(SELECTION_BG));
        }
        children.push(el.child(word).into_any_element());
    }
    if children.is_empty() {
        children.push(div().child(" ".to_string()).into_any_element());
    }

    div()
        .flex()
        .flex_row()
        .children(children)
        .into_any_element()
}
