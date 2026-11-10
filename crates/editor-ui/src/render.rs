//! `EditorView`'s `Render` impl: assembles the virtualized row list, wires
//! scroll handles and mouse/keyboard interaction, and decides what each
//! visible row paints. Row internals live in `row.rs`, scrollbars in
//! `scrollbar.rs`, input in `input.rs`, measurement in `measure.rs`.

use gpui::prelude::FluentBuilder;
use gpui::*;

use std::ops::Range;

use edit_buffer::Point;

use crate::view::RowInteraction;
use crate::wrap::{clip_to_subrange, WrapCache};

use crate::measure::{
    measure_char_width, round_wrap_width, CONTENT_X_REM, MAX_ROW_RENDER_CHARS,
    WRAP_GUTTER_RESERVE,
};
use crate::row::{render_line, resolve_color, row_local_selection, RowMeta};
use crate::scrollbar::{render_h_scrollbar, render_scrollbar, scrollbar_thumb_height, scrollbar_thumb_size};
use crate::types::comment_target_from;
use crate::view::EditorView;

impl Render for EditorView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let state_snapshot_version = self.state.read(cx).version();
        let wrap_enabled = self.state.read(cx).wrap_enabled();
        let comment_enabled = self.state.read(cx).comment_gutter() && !wrap_enabled;
        let font = self.state.read(cx).font().clone();

        let measured_height = self.viewport_height.get();
        let measured_width = self.viewport_width.get();
        let rows_that_fit = (measured_height / font.line_height).floor();
        let list_height = if rows_that_fit > 0. {
            font.line_height * rows_that_fit
        } else {
            measured_height
        };

        let wrap_width = if wrap_enabled && measured_width > WRAP_GUTTER_RESERVE {
            Some(round_wrap_width(measured_width - WRAP_GUTTER_RESERVE))
        } else if wrap_enabled {
            // Not measured yet (first frame): wrap at *something* rather
            // than not at all, corrected next frame once measured.
            Some(px(400.))
        } else {
            None
        };

        let char_width = measure_char_width(&self.char_width, &font, window);

        // Widest-row text width, cached per (buffer version, font); wrap-on
        // skips the scan entirely (wrapped text needs no horizontal sizing).
        // Bound to a local first: a `Ref` in the match scrutinee would
        // otherwise still be alive when the recompute arm takes `borrow_mut`.
        let max_text_px = if wrap_enabled {
            px(0.)
        } else {
            let cached_width = self.content_width_cache.borrow().clone();
            match cached_width {
                Some((v, f, w)) if v == state_snapshot_version && f == font => w,
                _ => {
                    let w = self.state.read(cx).max_line_width(char_width);
                    *self.content_width_cache.borrow_mut() =
                        Some((state_snapshot_version, font.clone(), w));
                    w
                }
            }
        };

        if self
            .wrap_cache
            .borrow()
            .is_stale(wrap_width, state_snapshot_version, &font)
        {
            let rebuilt = {
                let state = self.state.read(cx);
                WrapCache::rebuild(
                    state.line_count(),
                    |row| state.line_text(row),
                    state.version(),
                    wrap_width,
                    &font,
                    char_width,
                    window,
                )
            };
            *self.wrap_cache.borrow_mut() = rebuilt;
        }
        let visual_row_count = self.wrap_cache.borrow().visual_row_count();

        let content_x = window.rem_size() * CONTENT_X_REM;
        // H-scroll shares the list's one scroll handle/hitbox: gpui dispatches
        // nested scrollables' wheel events non-exclusively, so a second
        // scrollable div would turn a trackpad swipe's residual y noise into
        // vertical scroll. `restrict_scroll_to_axis` (set below) axis-locks
        // the one gesture instead.
        let base_handle = self.scroll_handle.0.borrow().base_handle.clone();
        // Only a line that actually overflows the viewport earns h-scroll.
        let can_h_scroll = !wrap_enabled && content_x + max_text_px > measured_width;
        if !can_h_scroll {
            // Drop a stale horizontal offset so we don't come back
            // pre-scrolled with nothing visible to have caused it.
            let y = base_handle.offset().y;
            base_handle.set_offset(point(px(0.), y));
        }
        // Trailing pad keeps the longest line's last glyph off the edge and
        // gives scroll-past-the-end room, Monaco-style.
        const H_TRAILING_PAD: Pixels = px(64.0);
        let content_width = can_h_scroll.then(|| content_x + max_text_px + H_TRAILING_PAD);

        if self.reveal_caret.take() {
            self.scroll_caret_into_view(char_width, content_x, content_width, measured_width, cx);
        }

        // Gutter geometry mirrors the old in-flow layout (`px_3` row pad +
        // `w_10` gutter = CONTENT_X_REM) so text starts at the same x.
        let rem = window.rem_size();
        let gutter_pad = rem * 0.75;
        let gutter_w = rem * 2.5;
        // The gutter is plain row content: it rides the row's translation
        // and the container's left edge clips it, so the whole row scrolls
        // as one unit (rows' hit-testers read the offset live via `h_handle`).
        // Bar visibility, from last frame's prepaint — same staleness
        // contract as the vertical bar's `is_scrollable()` check.
        let h_scrollable = can_h_scroll && base_handle.max_offset().x > px(0.);

        let viewport_height = self.viewport_height.clone();
        let viewport_width = self.viewport_width.clone();
        let viewport_origin = self.viewport_origin.clone();
        let editable = !self.state.read(cx).is_readonly();
        let input_focus = self.focus_handle.clone();
        let input_view = cx.entity();
        let wrap_cache = self.wrap_cache.clone();
        let row_font = font.clone();
        let interaction = RowInteraction {
            state: self.state.clone(),
            char_width: self.char_width.clone(),
            selecting: self.selecting.clone(),
            drag_anchor: self.drag_anchor.clone(),
            last_head: self.last_head.clone(),
            h_handle: base_handle.clone(),
            comment_hover_row: self.comment_hover_row.clone(),
            comment_drag: self.comment_drag.clone(),
            key_anchor: self.key_anchor.clone(),
            goal_col: self.goal_col.clone(),
        };

        div()
            .id("gpui-editor")
            .relative()
            .size_full()
            .bg(Hsla::black())
            .text_color(Hsla::white())
            .track_focus(&self.focus_handle)
            .key_context("Editor")
            .on_action(cx.listener(EditorView::copy))
            .on_action(cx.listener(EditorView::select_all))
            .on_action(cx.listener(EditorView::cancel_comment))
            .on_action(cx.listener(EditorView::undo))
            .on_action(cx.listener(EditorView::redo))
            .on_action(cx.listener(EditorView::cut))
            .on_action(cx.listener(EditorView::paste))
            .on_action(cx.listener(EditorView::save))
            .on_key_down(cx.listener(EditorView::key_down))
            .on_mouse_up(MouseButton::Left, {
                let selecting = self.selecting.clone();
                let comment_drag = self.comment_drag.clone();
                let comment_hover = self.comment_hover_row.clone();
                let state = self.state.clone();
                move |_event, window, cx| {
                    selecting.set(false);
                    if let Some(anchor) = comment_drag.take() {
                        let current = comment_hover.get().unwrap_or(anchor);
                        state.update(cx, |state, cx| {
                            let target = comment_target_from(
                                Point {
                                    row: anchor,
                                    col: 0,
                                },
                                Point {
                                    row: current,
                                    col: 0,
                                },
                                state.line_count(),
                            );
                            // Keep the range lit after release; the host
                            // clears it when the comment is removed.
                            state.add_comment_selection(target);
                            state.report_comment_target(target);
                            cx.notify();
                        });
                        window.refresh();
                    }
                }
            })
            // Paint-less sibling purely to measure the container's pixel
            // size each frame: height snaps the list to whole lines, width
            // drives word-wrap's wrap point.
            .child(
                canvas(
                    move |bounds, _window, _cx| {
                        viewport_height.set(bounds.size.height);
                        viewport_width.set(bounds.size.width);
                        viewport_origin.set(bounds.origin);
                    },
                    // Paint is the only phase an input handler can be
                    // registered in. It makes the focused, editable editor
                    // the window's text-input target (typing, IME).
                    move |bounds, _, window, cx| {
                        if editable {
                            window.handle_input(
                                &input_focus,
                                ElementInputHandler::new(bounds, input_view),
                                cx,
                            );
                        }
                    },
                )
                .absolute()
                .size_full(),
            )
            .child({
                let mut list = uniform_list(
                    "gpui-editor-lines",
                    visual_row_count,
                    cx.processor(move |this, range: Range<usize>, window, cx| {
                        let state = this.state.read(cx);
                        let selection = state.selection().range.clone();
                        let marked = state.marked_range();
                        // Caret: only an empty selection in a focused,
                        // editable editor paints one (a non-empty selection
                        // keeps today's highlight; readonly never shows it).
                        let caret_offset = if selection.is_empty()
                            && !state.is_readonly()
                            && this.focus_handle.is_focused(window)
                        {
                            Some(selection.start)
                        } else {
                            None
                        };
                        let cache = wrap_cache.borrow();
                        // Borrowed straight from state: no per-render Vec clone.
                        let comment_selections = state.comment_selections();
                        // Consecutive visual rows usually share a buffer row
                        // (wrapped lines): fetch/measure its text once and keep
                        // a forward (char, byte) cursor so slicing is linear.
                        let mut cur: Option<(u32, String, usize, usize, usize)> = None;
                        range
                            .map(|visual_ix| {
                                let (buffer_row, sub) = cache.resolve(visual_ix);
                                if cur.as_ref().map(|c| c.0) != Some(buffer_row) {
                                    let t = state.line_text(buffer_row);
                                    let n = t.chars().count();
                                    cur = Some((buffer_row, t, n, 0, 0));
                                }
                                let (_, full_text, row_len, cur_char, cur_byte) = cur
                                    .as_mut()
                                    .map(|c| (c.0, &c.1, c.2, &mut c.3, &mut c.4))
                                    .unwrap();
                                let sub_range = cache.sub_range(buffer_row, sub, row_len);
                                let sub_len = sub_range.end - sub_range.start;
                                let render_len = sub_len.min(MAX_ROW_RENDER_CHARS);
                                let sub_range = sub_range.start..sub_range.start + render_len;
                                if sub_range.start < *cur_char {
                                    *cur_char = 0;
                                    *cur_byte = 0;
                                }
                                let rest = &full_text[*cur_byte..];
                                let start_b = rest
                                    .char_indices()
                                    .nth(sub_range.start - *cur_char)
                                    .map_or(rest.len(), |(i, _)| i);
                                *cur_byte += start_b;
                                *cur_char = sub_range.start;
                                let rest = &full_text[*cur_byte..];
                                let end_b = rest
                                    .char_indices()
                                    .nth(render_len)
                                    .map_or(rest.len(), |(i, _)| i);
                                let text: String = rest[..end_b].to_string();
                                let runs = state
                                    .row_spans
                                    .get(buffer_row as usize)
                                    .map(|spans| {
                                        spans
                                            .iter()
                                            .filter_map(|s| {
                                                clip_to_subrange(&s.range, &sub_range)
                                                    .map(|r| (r, resolve_color(s)))
                                            })
                                            .collect::<Vec<_>>()
                                    })
                                    .unwrap_or_default();
                                let row_start = state.row_start_offset(buffer_row);
                                let row_end = state.row_end_offset(buffer_row);
                                let row_selection =
                                    row_local_selection(&selection, row_start, row_end)
                                        .and_then(|s| clip_to_subrange(&s, &sub_range));
                                let marked = marked
                                    .as_ref()
                                    .and_then(|m| row_local_selection(m, row_start, row_end))
                                    .and_then(|m| clip_to_subrange(&m, &sub_range));
                                // Caret column within this sub-line. A caret
                                // sitting exactly on a sub boundary would
                                // match two sub-lines (end of one, start of
                                // the next) — draw it only on the row's last
                                // sub-line, which is where a line-end caret
                                // belongs anyway.
                                let caret_col = caret_offset.and_then(|caret| {
                                    let local = caret.checked_sub(row_start)?;
                                    let sub_start = sub_range.start;
                                    let sub_end = sub_range.end;
                                    let is_last_sub = sub_range.end == row_len;
                                    if local >= sub_start
                                        && (local < sub_end || (local == sub_end && is_last_sub))
                                    {
                                        Some(local - sub_start)
                                    } else {
                                        None
                                    }
                                });
                                let indent_px = if sub > 0 {
                                    char_width * (cache.indent_chars(buffer_row) as f32)
                                } else {
                                    px(0.)
                                };
                                let comment_selected = comment_selections.iter().any(|target| {
                                    (target.start_line..=target.end_line)
                                        .contains(&(buffer_row + 1))
                                });
                                let meta = RowMeta {
                                    row: buffer_row,
                                    show_line_number: sub == 0,
                                    col_offset: sub_range.start as u32,
                                    content_x,
                                    indent_px,
                                    gutter_pad,
                                    gutter_w,
                                    row_width: content_width,
                                    comment_enabled,
                                    comment_selected,
                                    comment_hover_row: interaction.comment_hover_row.get(),
                                    comment_drag: interaction.comment_drag.get(),
                                    caret_col,
                                    marked,
                                    char_width,
                                    tab_width: state.tab_width(),
                                };
                                render_line(
                                    meta,
                                    text,
                                    runs,
                                    row_selection,
                                    row_font.clone(),
                                    interaction.clone(),
                                )
                            })
                            .collect()
                    }),
                )
                .track_scroll(&self.scroll_handle)
                .text_sm()
                .py_2()
                .h(list_height);

                if can_h_scroll {
                    // Rows must size to their content width, not the list's
                    // viewport width — otherwise gpui clamps the x offset to
                    // zero and h-scroll never moves. Poked (not chained)
                    // because `UniformList` isn't a
                    // `StatefulInteractiveElement`; both settings apply only
                    // when h-scroll is active, so vertical-only files are
                    // unaffected.
                    list = list.with_horizontal_sizing_behavior(
                        ListHorizontalSizingBehavior::Unconstrained,
                    );
                    list.interactivity().base_style.restrict_scroll_to_axis = Some(true);
                }
                // Always viewport-width: these are the scroll *bounds* gpui
                // clamps against, so `content_width` (measured from the
                // explicitly-sized rows above) minus these bounds is the
                // real horizontal scroll range. They're also the mask rows
                // paint through, so long lines clip at the viewport edge
                // instead of bleeding past it. `None` (no overflowing line,
                // or wrap on) is the common case and pays none of h-scroll's
                // extra cost.
                list.w_full().into_any_element()
            })
            .when(self.scroll_handle.is_scrollable(), |el| {
                el.child(render_scrollbar(
                    self.scroll_handle.clone(),
                    self.thumb_dragging.clone(),
                ))
            })
            .when(h_scrollable, |el| {
                el.child(render_h_scrollbar(
                    base_handle.clone(),
                    self.h_thumb_dragging.clone(),
                    self.scroll_handle.is_scrollable(),
                ))
            })
            .on_mouse_move({
                let scroll_handle = self.scroll_handle.clone();
                let thumb_dragging = self.thumb_dragging.clone();
                let h_thumb_dragging = self.h_thumb_dragging.clone();
                move |event, window, _cx| {
                    let base = scroll_handle.0.borrow().base_handle.clone();
                    if let Some((start_mouse_y, start_offset_y)) = thumb_dragging.get() {
                        let track_height = base.bounds().size.height;
                        let max_offset_y = base.max_offset().y;
                        let thumb_height = scrollbar_thumb_height(track_height, max_offset_y);
                        let track_range = (track_height - thumb_height).max(px(1.));
                        let delta = event.position.y - start_mouse_y;
                        let new_offset_y = (start_offset_y - delta * (max_offset_y / track_range))
                            .clamp(-max_offset_y, px(0.));
                        base.set_offset(point(base.offset().x, new_offset_y));
                        window.refresh();
                    }
                    if let Some((start_mouse_x, start_offset_x)) = h_thumb_dragging.get() {
                        let track_width = base.bounds().size.width;
                        let max_offset_x = base.max_offset().x;
                        let thumb_width = scrollbar_thumb_size(track_width, max_offset_x);
                        let track_range = (track_width - thumb_width).max(px(1.));
                        let delta = event.position.x - start_mouse_x;
                        let new_offset_x = (start_offset_x - delta * (max_offset_x / track_range))
                            .clamp(-max_offset_x, px(0.));
                        base.set_offset(point(new_offset_x, base.offset().y));
                        window.refresh();
                    }
                }
            })
            .on_mouse_up(MouseButton::Left, {
                let thumb_dragging = self.thumb_dragging.clone();
                let h_thumb_dragging = self.h_thumb_dragging.clone();
                move |_event, _window, _cx| {
                    thumb_dragging.set(None);
                    h_thumb_dragging.set(None);
                }
            })
    }
}

