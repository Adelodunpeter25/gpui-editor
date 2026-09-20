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
pub(crate) fn measure_char_width(
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

/// Pixel x (relative to the row content's left edge) -> char column. Walks
/// `text`'s chars accumulating pixel width rather than dividing by a flat
/// `char_width`, since a tab doesn't occupy one glyph-width column — it
/// advances to the next `tab_width`-column stop, same as `indent_for_newline`
/// assumes. A single division here (the old approach) put every column after
/// the first tab in a line off by however wide that tab rendered.
fn column_for_x(local_x: Pixels, char_width: Pixels, text: &str, tab_width: u32) -> u32 {
    let target = local_x.max(px(0.));
    let mut x = px(0.);
    let mut col: u32 = 0;
    for ch in text.chars() {
        let advance = if ch == '\t' {
            let tab_px = f32::from(char_width) * tab_width.max(1) as f32;
            let steps = (f32::from(x) / tab_px).floor() + 1.0;
            px(tab_px * steps - f32::from(x))
        } else {
            char_width
        };
        let mid = x + advance / 2.0;
        if target < mid {
            return col;
        }
        x += advance;
        col += 1;
    }
    col
}

fn resolve_color(span: &StyledSpan) -> Hsla {
    span.color
        .map(|(r, g, b)| rgb(((r as u32) << 16) | ((g as u32) << 8) | b as u32).into())
        .unwrap_or_else(|| color_for(span.capture))
}

/// Approximate pixel width consumed by the line-number gutter (`w_10()`,
/// 2.5rem) + the row's horizontal padding (`px_3()` x2) before wrapped text
/// content actually starts. Used only to size the word-wrap width; a few
/// pixels of slack here just wraps a character or two earlier/later than
/// pixel-perfect, which is cosmetic (see wrap.md's scope notes).
const WRAP_GUTTER_RESERVE: Pixels = px(64.0);

/// Bucket size for wrap-width rounding: a live window resize fires the
/// measuring `canvas` (and so a potential `WrapCache` rebuild, which
/// re-shapes every row) on nearly every frame. Rounding to a coarse bucket
/// means a rebuild only happens once every ~4 monospace chars of resize
/// instead of every single pixel — the resize-drag equivalent of the
/// existing debounce-on-edit pattern the rest of the codebase uses.
const WRAP_WIDTH_BUCKET: f32 = 32.0;

pub(crate) fn round_wrap_width(width: Pixels) -> Pixels {
    px((f32::from(width) / WRAP_WIDTH_BUCKET).round() * WRAP_WIDTH_BUCKET)
}

/// Row left padding (`px_3()` = 0.75rem) + gutter width (`w_10()` = 2.5rem) =
/// where a row's text content actually starts, in rems. Fixed, non-dynamic
/// layout — computed once per render from `window.rem_size()` rather than
/// measured via a `canvas` per row (previously: one measurement canvas per
/// *visible row*, remeasuring an identical value ~30 times a render for no
/// reason, since every row has the same gutter/padding).
const CONTENT_X_REM: f32 = 3.25;

/// Cap on chars rendered/hit-tested per visual row. Without this, a single
/// pathologically long line (a minified file, a giant JSON blob) with wrap
/// off rebuilds a `chars().collect()` + one-div-per-syntax/selection-segment
/// tree that size on *every* mouse-move during a drag — `.overflow_x_hidden()`
/// only hides the overflow visually after all that work already happened.
/// Clamping what's built also clamps what's selectable in the row, matching
/// how most editors treat absurdly long lines.
pub(crate) const MAX_ROW_RENDER_CHARS: usize = 4000;

/// Pixel width of a char stream with tab stops, skipping line breaks.
/// Capped at `MAX_ROW_RENDER_CHARS` so a width scan describes the same
/// painted prefix the row renderer builds — otherwise dead scroll range
/// would trail every pathologically long line.
pub(crate) fn chars_display_width(
    chars: impl Iterator<Item = char>,
    char_width: Pixels,
    tab_width: u32,
) -> Pixels {
    let cw = f32::from(char_width);
    let tab_px = cw * tab_width.max(1) as f32;
    let mut x = 0f32;
    for ch in chars
        .filter(|c| *c != '\n' && *c != '\r')
        .take(MAX_ROW_RENDER_CHARS)
    {
        if ch == '\t' {
            x = tab_px * ((x / tab_px).floor() + 1.0);
        } else {
            x += cw;
        }
    }
    px(x)
}

pub(crate) fn text_display_width(text: &str, char_width: Pixels, tab_width: u32) -> Pixels {
    chars_display_width(text.chars(), char_width, tab_width)
}

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

        let char_width = measure_char_width(&self.char_width, &font, window);

        // Widest-row text width, cached per (buffer version, font). The scan
        // is O(total chars), so beyond not running per frame (the cache),
        // it also only runs at all when wrap is off — wrapped text never
        // needs a horizontal range, so there's nothing to size for. This is
        // pure arithmetic (no shaping), so even a full-buffer scan is cheap;
        // if a pathologically huge file ever makes it show up in profiling,
        // move it to a background task the same way `rehighlight_from` does.
        // (Bound to a local first: the `Ref` guard in a match scrutinee
        // would otherwise still be alive when the recompute arm takes
        // `borrow_mut`.)
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
        // Horizontal scroll rides the *same* handle/hitbox as the list's own
        // vertical scroll (its `base_handle`, which already carries a full
        // x/y offset) instead of a second, independently-scrollable div
        // wrapped around it. Two separate scrollable regions each install
        // their own wheel listener and gpui dispatches both (nesting isn't
        // exclusive) — the inner one has no idea a horizontal gesture is in
        // progress on the outer one, so a trackpad swipe's small residual
        // y noise (real hardware never reports an exactly-zero cross-axis
        // delta) gets scrolled as if it were an intentional vertical flick.
        // One handle + `restrict_scroll_to_axis` (set below, only when
        // h-scroll is actually active) means gpui's own axis-exclusivity
        // logic runs once, in one place, on the real gesture.
        let base_handle = self.scroll_handle.0.borrow().base_handle.clone();
        // Only the widest line actually overflowing the viewport earns the
        // horizontal-scroll layout — a short file (or a long file where
        // every line still fits) renders as a plain, non-scrollable list,
        // same as before this feature existed. This is what makes h-scroll
        // "only work when needed" instead of wrapping every file in the
        // extra gutter-hiding machinery below.
        let can_h_scroll = !wrap_enabled && content_x + max_text_px > measured_width;
        if !can_h_scroll {
            // Drop any stale horizontal offset from a previous file/width
            // that *did* scroll, so a later transition back to h-scroll (or
            // just re-measuring at a wider viewport) doesn't start "pre-
            // scrolled" with no visible way to have caused it.
            let y = base_handle.offset().y;
            base_handle.set_offset(point(px(0.), y));
        }
        // Trailing pad keeps the longest line's last glyph off the edge and
        // gives scroll-past-the-end room, Monaco-style.
        const H_TRAILING_PAD: Pixels = px(64.0);
        let content_width = can_h_scroll.then(|| content_x + max_text_px + H_TRAILING_PAD);

        // Render-time horizontal shift. Scrolling notifies this view (GPUI
        // re-renders on scroll-offset change), so this stays live frame to
        // frame.
        let scroll_x = (-base_handle.offset().x).max(px(0.));
        // Gutter geometry mirrors the old in-flow layout (`px_3` row pad +
        // `w_10` gutter = CONTENT_X_REM) so text starts at the same x.
        let rem = window.rem_size();
        let gutter_pad = rem * 0.75;
        let gutter_w = rem * 2.5;
        // Rather than pin the gutter in place while text slides under it,
        // hide it entirely once the user has scrolled right at all — line
        // numbers for rows whose start is off-screen to the left aren't
        // useful pinned in place anyway, and this avoids an absolute-
        // position-plus-opaque-background overlay hack just to fake it.
        let show_gutter = scroll_x <= px(0.5);
        // Bottom bar visibility, from last frame's prepaint — same staleness
        // contract as the existing vertical bar's `is_scrollable()` check.
        let h_scrollable = can_h_scroll && base_handle.max_offset().x > px(0.);

        let viewport_height = self.viewport_height.clone();
        let viewport_width = self.viewport_width.clone();
        let wrap_cache = self.wrap_cache.clone();
        let row_font = font.clone();
        let interaction = RowInteraction {
            state: self.state.clone(),
            char_width: self.char_width.clone(),
            selecting: self.selecting.clone(),
            drag_anchor: self.drag_anchor.clone(),
            last_head: self.last_head.clone(),
            h_handle: base_handle.clone(),
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
            .child({
                let mut list = uniform_list(
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
                                let sub_len = sub_range.end - sub_range.start;
                                let render_len = sub_len.min(MAX_ROW_RENDER_CHARS);
                                let sub_range = sub_range.start..sub_range.start + render_len;
                                let text: String = full_text
                                    .chars()
                                    .skip(sub_range.start)
                                    .take(render_len)
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
                                let indent_px = if sub > 0 {
                                    char_width * (cache.indent_chars(buffer_row) as f32)
                                } else {
                                    px(0.)
                                };
                                let meta = RowMeta {
                                    row: buffer_row,
                                    show_line_number: sub == 0,
                                    col_offset: sub_range.start as u32,
                                    content_x,
                                    indent_px,
                                    gutter_pad,
                                    gutter_w,
                                    show_gutter,
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
                    // Same hitbox/handle as the vertical scroll (see the
                    // `base_handle` comment above) instead of a second
                    // nested scrollable container, so gpui's own
                    // axis-exclusivity logic sees the whole gesture in one
                    // place. `restrict_scroll_to_axis` locks each individual
                    // gesture to whichever axis it's predominantly moving
                    // on, only turned on here (when h-scroll is actually
                    // possible) so a normal file's vertical-only scrolling
                    // is completely unaffected.
                    list.interactivity().base_style.overflow.x = Some(Overflow::Scroll);
                    list.interactivity().base_style.restrict_scroll_to_axis = Some(true);
                }
                match content_width {
                    Some(w) => list.w(w).into_any_element(),
                    // Nothing overflows: plain list, no h-scroll at all —
                    // the common case (most files, and any file with wrap
                    // on) pays none of h-scroll's extra cost.
                    None => list.w_full().into_any_element(),
                }
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

/// Minimum thumb size so a huge file never shrinks a scrollbar to invisibility.
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
/// `render_scrollbar` on the x axis; driven by the outer `overflow_scroll`
/// div's `ScrollHandle` rather than the vertical `uniform_list` handle.
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
    /// Where this row's text content starts, x-wise (gutter + padding).
    content_x: Pixels,
    /// Extra left padding for a wrapped continuation line, so it aligns
    /// under its own indent instead of restarting at column 0. `0` for a
    /// row's first sub-line.
    indent_px: Pixels,
    /// Gutter's left edge in row-local (unscrolled) coordinates — constant,
    /// since the gutter is only ever shown at (or near) zero horizontal
    /// scroll (see `show_gutter`).
    gutter_pad: Pixels,
    /// Gutter width (`w_10`); text starts at `gutter_pad + gutter_w + gap`
    /// = `content_x` in row coordinates.
    gutter_w: Pixels,
    /// False once the row's horizontal scroll container has been scrolled
    /// right at all — line numbers for a row whose start is off-screen
    /// aren't useful, so the gutter just hides rather than staying pinned
    /// over sliding text.
    show_gutter: bool,
}

fn render_line(
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
        show_gutter,
    } = meta;
    let line_no = if show_line_number {
        format!("{:>4}", row + 1)
    } else {
        String::new()
    };
    let line_height = font.line_height;
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
    let move_text = Rc::new(text.clone());

    div()
        .id(("editor-line", row as usize))
        .relative()
        .flex()
        .flex_row()
        .items_center()
        .w_full()
        .h(line_height)
        .font_family(font.family.clone())
        .text_size(font.size)
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
        // Gutter: hidden once the row is scrolled right at all (see
        // `show_gutter`'s doc comment) rather than pinned in place over
        // sliding text. Opaque background covers text sliding underneath
        // while shown; painted after content so it wins the overlap.
        .when(show_gutter, |el| {
            el.child(
                div()
                    .absolute()
                    .left(gutter_pad)
                    .top_0()
                    .w(gutter_w)
                    .h_full()
                    .bg(Hsla::black())
                    .border_r_2()
                    .border_color(rgba(0xffffff1a))
                    .text_color(rgb(0x585b70))
                    .child(line_no),
            )
        })
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
