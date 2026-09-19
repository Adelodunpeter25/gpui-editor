//! `editor-ui`: embeddable GPUI code view (raw `gpui 0.2.2`, no component kit).
//!
//! M0: readonly viewer + optional single-cursor edit stub.
//! State lives in `Entity<EditorState>`; view holds the handle only.
//! `edit-buffer` and `syntax` stay GPUI-free so the host app can reuse them
//! headless. Model/view types live in `types.rs`, word-wrap in `wrap.rs`;
//! this file holds the `Render` impl and its paint helpers.

mod diff_view;
mod types;
mod wrap;

use edit_buffer::Point;
use gpui::prelude::FluentBuilder;
use gpui::*;
use std::cell::{Cell, RefCell};
use std::ops::Range;
use std::rc::Rc;
use syntax::Capture;
use types::{RowInteraction, StyledSpan};
use wrap::{clip_to_subrange, WrapCache};

pub use diff::{DiffLine, DiffLineKind, DiffResult};
pub use diff_view::{DiffState, DiffView};
pub use types::{EditorState, EditorView, FontConfig, IndentOptions, Mode, SearchState, Selection};

// ---------------------------------------------------------------------------
// Actions + view rendering
// ---------------------------------------------------------------------------

actions!(editor_ui, [Copy, SelectAll]);

/// Bind default keys for `editor_ui`'s actions. Call once at app startup.
pub fn init(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("cmd-c", Copy, Some("Editor")),
        KeyBinding::new("ctrl-c", Copy, Some("Editor")),
        KeyBinding::new("cmd-a", SelectAll, Some("Editor")),
        KeyBinding::new("ctrl-a", SelectAll, Some("Editor")),
    ]);
}

/// Measure (and cache) the monospace glyph advance width for pixel -> column
/// hit testing. Cached alongside the `FontConfig` it was measured for, so a
/// runtime font change re-measures instead of reusing a stale width.
fn measure_char_width(
    cache: &Rc<RefCell<Option<(FontConfig, Pixels)>>>,
    font_config: &FontConfig,
    window: &mut Window,
) -> Pixels {
    if let Some((cached_font, width)) = cache.borrow().as_ref() {
        if cached_font == font_config {
            return *width;
        }
    }
    let run = TextRun {
        len: 1,
        font: font(font_config.family.clone()),
        color: Hsla::white(),
        background_color: None,
        underline: None,
        strikethrough: None,
    };
    let width = window
        .text_system()
        .shape_line("M".into(), font_config.size, &[run], None)
        .width();
    *cache.borrow_mut() = Some((font_config.clone(), width));
    width
}

/// Pixel x (relative to the row content's left edge) -> char column, clamped
/// to `row_len`.
fn column_for_x(local_x: Pixels, char_width: Pixels, row_len: usize) -> u32 {
    let col = (local_x.max(px(0.)) / char_width).round().max(0.) as usize;
    col.min(row_len) as u32
}

fn resolve_color(span: &StyledSpan) -> Hsla {
    span.color
        .map(|(r, g, b)| rgb(((r as u32) << 16) | ((g as u32) << 8) | b as u32).into())
        .unwrap_or_else(|| color_for(span.capture))
}

/// Approximate pixel width consumed by the line-number gutter (`w_12()`,
/// 3rem) + the row's horizontal padding (`px_3()` x2) before wrapped text
/// content actually starts. Used only to size the word-wrap width; a few
/// pixels of slack here just wraps a character or two earlier/later than
/// pixel-perfect, which is cosmetic (see wrap.md's scope notes).
const WRAP_GUTTER_RESERVE: Pixels = px(72.0);

/// Bucket size for wrap-width rounding: a live window resize fires the
/// measuring `canvas` (and so a potential `WrapCache` rebuild, which
/// re-shapes every row) on nearly every frame. Rounding to a coarse bucket
/// means a rebuild only happens once every ~4 monospace chars of resize
/// instead of every single pixel — the resize-drag equivalent of the
/// existing debounce-on-edit pattern the rest of the codebase uses.
const WRAP_WIDTH_BUCKET: f32 = 32.0;

fn round_wrap_width(width: Pixels) -> Pixels {
    px((f32::from(width) / WRAP_WIDTH_BUCKET).round() * WRAP_WIDTH_BUCKET)
}

/// Row left padding (`px_3()` = 0.75rem) + gutter width (`w_12()` = 3rem) =
/// where a row's text content actually starts, in rems. Fixed, non-dynamic
/// layout — computed once per render from `window.rem_size()` rather than
/// measured via a `canvas` per row (previously: one measurement canvas per
/// *visible row*, remeasuring an identical value ~30 times a render for no
/// reason, since every row has the same gutter/padding).
const CONTENT_X_REM: f32 = 3.75;

impl Render for EditorView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let state_snapshot_version = self.state.read(cx).version();
        let wrap_enabled = self.state.read(cx).wrap_enabled();
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

        if self
            .wrap_cache
            .borrow()
            .is_stale(wrap_width, state_snapshot_version, &font)
        {
            let rebuilt = {
                let state = self.state.read(cx);
                WrapCache::rebuild(state, wrap_width, &font, window)
            };
            *self.wrap_cache.borrow_mut() = rebuilt;
        }
        let visual_row_count = self.wrap_cache.borrow().visual_row_count();

        let content_x = window.rem_size() * CONTENT_X_REM;

        let viewport_height = self.viewport_height.clone();
        let viewport_width = self.viewport_width.clone();
        let wrap_cache = self.wrap_cache.clone();
        let row_font = font.clone();
        let interaction = RowInteraction {
            state: self.state.clone(),
            char_width: self.char_width.clone(),
            selecting: self.selecting.clone(),
            drag_anchor: self.drag_anchor.clone(),
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
            .on_mouse_up(MouseButton::Left, {
                let selecting = self.selecting.clone();
                move |_event, _window, _cx| selecting.set(false)
            })
            // Paint-less sibling purely to measure the container's pixel
            // size each frame: height snaps the list to whole lines, width
            // drives word-wrap's wrap point.
            .child(
                canvas(
                    move |bounds, _window, _cx| {
                        viewport_height.set(bounds.size.height);
                        viewport_width.set(bounds.size.width);
                    },
                    |_, _, _, _| {},
                )
                .absolute()
                .size_full(),
            )
            .child(
                uniform_list(
                    "gpui-editor-lines",
                    visual_row_count,
                    cx.processor(move |this, range: Range<usize>, _window, cx| {
                        let state = this.state.read(cx);
                        let selection = state.selection().range.clone();
                        let cache = wrap_cache.borrow();
                        range
                            .map(|visual_ix| {
                                let (buffer_row, sub) = cache.resolve(visual_ix);
                                let full_text = state.line_text(buffer_row);
                                let row_len = full_text.chars().count();
                                let sub_range = cache.sub_range(buffer_row, sub, row_len);
                                let text: String = full_text
                                    .chars()
                                    .skip(sub_range.start)
                                    .take(sub_range.end - sub_range.start)
                                    .collect();
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
                                let row_selection = row_local_selection(&selection, row_start, row_end)
                                    .and_then(|s| clip_to_subrange(&s, &sub_range));
                                let meta = RowMeta {
                                    row: buffer_row,
                                    show_line_number: sub == 0,
                                    col_offset: sub_range.start as u32,
                                };
                                render_line(
                                    meta,
                                    text,
                                    runs,
                                    row_selection,
                                    row_font.clone(),
                                    content_x,
                                    interaction.clone(),
                                )
                            })
                            .collect()
                    }),
                )
                .track_scroll(&self.scroll_handle)
                .text_sm()
                .py_2()
                .w_full()
                .h(list_height),
            )
            .when(self.scroll_handle.is_scrollable(), |el| {
                el.child(render_scrollbar(
                    self.scroll_handle.clone(),
                    self.thumb_dragging.clone(),
                ))
            })
            .on_mouse_move({
                let scroll_handle = self.scroll_handle.clone();
                let thumb_dragging = self.thumb_dragging.clone();
                move |event, window, _cx| {
                    let Some((start_mouse_y, start_offset_y)) = thumb_dragging.get() else {
                        return;
                    };
                    let base = scroll_handle.0.borrow().base_handle.clone();
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
            })
            .on_mouse_up(MouseButton::Left, {
                let thumb_dragging = self.thumb_dragging.clone();
                move |_event, _window, _cx| thumb_dragging.set(None)
            })
    }
}

/// Minimum thumb height so a huge file never shrinks it to invisibility.
const MIN_SCROLLBAR_THUMB: Pixels = px(24.0);

pub(crate) fn scrollbar_thumb_height(track_height: Pixels, max_offset_y: Pixels) -> Pixels {
    let content_height = track_height + max_offset_y;
    if content_height <= px(0.) {
        return track_height;
    }
    let ratio = track_height / content_height;
    (track_height * ratio).max(MIN_SCROLLBAR_THUMB).min(track_height)
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

/// Clip the whole-buffer selection range to `row_start..row_end`, in row-local
/// char coords. `None` if the row has no overlap with the selection.
fn row_local_selection(
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
struct RowMeta {
    row: u32,
    show_line_number: bool,
    col_offset: u32,
}

fn render_line(
    meta: RowMeta,
    text: String,
    runs: Vec<(Range<usize>, Hsla)>,
    selection: Option<Range<usize>>,
    font: FontConfig,
    content_x: Pixels,
    interaction: RowInteraction,
) -> impl IntoElement {
    let RowMeta {
        row,
        show_line_number,
        col_offset,
    } = meta;
    let line_no = if show_line_number {
        format!("{:>4}", row + 1)
    } else {
        String::new()
    };
    let row_len = text.chars().count();
    let line_height = font.line_height;

    let down = interaction.clone();
    let down_font = font.clone();
    let move_ = interaction.clone();
    let move_font = font.clone();

    div()
        .id(("editor-line", row as usize))
        .flex()
        .flex_row()
        .items_center()
        .px_3()
        .h(line_height)
        .font_family(font.family.clone())
        .text_size(font.size)
        .child(
            div()
                .w_12()
                .flex_shrink_0()
                .text_color(rgb(0x585b70))
                .child(line_no),
        )
        .child(
            div()
                .id(("editor-line-content", row as usize))
                .relative()
                .flex()
                .flex_row()
                .flex_1()
                .overflow_x_hidden()
                .child(render_spans(text, runs, selection))
                .on_mouse_down(MouseButton::Left, move |event, window, cx| {
                    let width = measure_char_width(&down.char_width, &down_font, window);
                    let local_x = event.position.x - content_x;
                    let col = col_offset + column_for_x(local_x, width, row_len);
                    let offset = down.state.read(cx).point_to_offset(Point { row, col });
                    down.drag_anchor.set(Some(offset));
                    down.selecting.set(true);
                    down.state
                        .update(cx, |state, _| state.set_selection(offset..offset));
                    window.refresh();
                })
                .on_mouse_move(move |event, window, cx| {
                    if !move_.selecting.get() {
                        return;
                    }
                    let Some(anchor) = move_.drag_anchor.get() else {
                        return;
                    };
                    let width = measure_char_width(&move_.char_width, &move_font, window);
                    let local_x = event.position.x - content_x;
                    let col = col_offset + column_for_x(local_x, width, row_len);
                    let head = move_.state.read(cx).point_to_offset(Point { row, col });
                    let range = if anchor <= head {
                        anchor..head
                    } else {
                        head..anchor
                    };
                    move_.state.update(cx, |state, _| state.set_selection(range));
                    window.refresh();
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

    div().flex().flex_row().children(children).into_any_element()
}

// Re-export for host apps.
pub use syntax::LanguageRegistry as Registry;
