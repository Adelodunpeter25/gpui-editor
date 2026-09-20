//! Diff viewer: `DiffState` + `DiffView`, per `diff.md`'s plan. Separate
//! from `EditorState`/`EditorView` on purpose — a diff's data model (a flat
//! list of added/removed/context lines) doesn't map onto a single rope
//! buffer + selection, so reusing `EditorState` would mean bolting a
//! second, incompatible shape onto it. Reuses the same virtualization
//! (`uniform_list`) and hand-rolled scrollbar as `EditorView` via the
//! `pub(crate)` paint helpers in `lib.rs`.
//!
//! Per-line syntax highlighting here re-highlights each `DiffLine`'s text in
//! isolation (one `syntax::highlight_themed` call per line), not the whole
//! old/new text once. This is a deliberate v1 simplification: it's simpler
//! and fine for single-line constructs, but a multi-line construct (a block
//! comment or triple-quoted string) that only partially changed will lose
//! its surrounding context and can highlight incorrectly on unchanged
//! lines next to a change. Fixing this means highlighting the full old and
//! new text once each and slicing spans per line (the same approach
//! `EditorState::rebuild_row_spans` already uses) — left for later since it
//! adds real bookkeeping for a cosmetic-only gap.

use gpui::prelude::FluentBuilder;
use gpui::*;
use std::cell::{Cell, RefCell};
use std::ops::Range;
use std::rc::Rc;
use syntax::{Language, ThemePreset};

use crate::types::FontConfig;
use crate::wrap::{clip_to_subrange, WrapCache};
use crate::{color_for, measure_char_width, render_h_scrollbar, render_scrollbar};
use crate::{render_spans, round_wrap_width, scrollbar_thumb_size, text_display_width};

/// Gutter width before wrapped text starts: two `w_10()` line-number
/// columns + `px_2()` padding on both sides + the row's own `border_l_4()`
/// change-marker bar — wider than the editor's own `WRAP_GUTTER_RESERVE`
/// (single gutter) since diff rows have two line-number columns.
const DIFF_WRAP_GUTTER_RESERVE: Pixels = px(100.0);

pub struct DiffState {
    pub(crate) result: diff::DiffResult,
    language: Option<&'static Language>,
    theme: ThemePreset,
    font: FontConfig,
    wrap_enabled: bool,
    version: u64,
}

impl DiffState {
    pub fn new(old: &str, new: &str, language: Option<&'static Language>) -> Self {
        Self::from_result(diff::diff_lines(old, new), language)
    }

    pub fn from_result(result: diff::DiffResult, language: Option<&'static Language>) -> Self {
        Self {
            result,
            language,
            theme: ThemePreset::GitHubDark,
            font: FontConfig::default(),
            wrap_enabled: true,
            version: 0,
        }
    }

    pub fn with_theme(mut self, theme: ThemePreset) -> Self {
        self.theme = theme;
        self
    }

    pub fn with_font(mut self, font: FontConfig) -> Self {
        self.font = font;
        self
    }

    pub fn with_wrap(mut self, enabled: bool) -> Self {
        self.wrap_enabled = enabled;
        self
    }

    pub fn set_texts(&mut self, old: &str, new: &str) {
        self.result = diff::diff_lines(old, new);
        self.version = self.version.wrapping_add(1);
    }

    pub fn set_result(&mut self, result: diff::DiffResult) {
        self.result = result;
        self.version = self.version.wrapping_add(1);
    }

    pub fn set_language(&mut self, language: Option<&'static Language>) {
        self.language = language;
    }

    pub fn set_theme(&mut self, theme: ThemePreset) {
        self.theme = theme;
    }

    pub fn set_font(&mut self, font: FontConfig) {
        self.font = font;
    }

    pub fn set_wrap_enabled(&mut self, enabled: bool) {
        self.wrap_enabled = enabled;
        self.version = self.version.wrapping_add(1);
    }

    pub fn language(&self) -> Option<&'static Language> {
        self.language
    }

    pub fn theme(&self) -> ThemePreset {
        self.theme
    }

    pub fn font(&self) -> &FontConfig {
        &self.font
    }

    pub fn wrap_enabled(&self) -> bool {
        self.wrap_enabled
    }

    pub fn version(&self) -> u64 {
        self.version
    }

    pub fn line_count(&self) -> usize {
        self.result.lines.len()
    }

    pub fn added(&self) -> usize {
        self.result.added
    }

    pub fn removed(&self) -> usize {
        self.result.removed
    }

    pub fn is_empty(&self) -> bool {
        self.result.is_empty()
    }

    /// Widest diff line in pixels (tab stops honored, capped at the same
    /// render limit the view paints) — drives horizontal-scroll content
    /// width. Diff lines carry no indent config; tabs use a fixed width.
    pub fn max_line_width(&self, char_width: Pixels) -> Pixels {
        const DIFF_TAB_WIDTH: u32 = 4;
        let mut max = px(0.);
        for line in &self.result.lines {
            let w = text_display_width(&line.text, char_width, DIFF_TAB_WIDTH);
            if w > max {
                max = w;
            }
        }
        max
    }
}

pub struct DiffView {
    state: Entity<DiffState>,
    scroll_handle: UniformListScrollHandle,
    viewport_height: Rc<Cell<Pixels>>,
    viewport_width: Rc<Cell<Pixels>>,
    thumb_dragging: Rc<Cell<Option<(Pixels, Pixels)>>>,
    wrap_cache: Rc<RefCell<WrapCache>>,
    /// Only needed to feed `WrapCache::rebuild`'s indent-width math (diff
    /// view has no mouse-driven selection, so unlike `EditorView` this
    /// cache is never used for hit-testing).
    char_width: Rc<RefCell<Option<(FontConfig, Pixels)>>>,
    /// Horizontal scroll state for the wrap-off path (same nesting as
    /// `EditorView`: the `uniform_list` scrolls vertically inside an
    /// `overflow_scroll` div tracked by this handle).
    h_handle: ScrollHandle,
    h_thumb_dragging: Rc<Cell<Option<(Pixels, Pixels)>>>,
    /// Cached widest-line text width: `(diff version, font, max px)`.
    content_width_cache: Rc<RefCell<Option<(u64, FontConfig, Pixels)>>>,
}

impl DiffView {
    pub fn new(state: &Entity<DiffState>) -> Self {
        Self {
            state: state.clone(),
            scroll_handle: UniformListScrollHandle::new(),
            viewport_height: Rc::new(Cell::new(px(0.))),
            viewport_width: Rc::new(Cell::new(px(0.))),
            thumb_dragging: Rc::new(Cell::new(None)),
            wrap_cache: Rc::new(RefCell::new(WrapCache::default())),
            char_width: Rc::new(RefCell::new(None)),
            h_handle: ScrollHandle::new(),
            h_thumb_dragging: Rc::new(Cell::new(None)),
            content_width_cache: Rc::new(RefCell::new(None)),
        }
    }
}

/// Gutter number column: right-aligned, blank (not `0`) when a line has no
/// counterpart on that side (a pure add/remove).
fn gutter_number(n: Option<u32>) -> String {
    n.map(|n| format!("{n:>4}")).unwrap_or_else(|| " ".repeat(4))
}

fn render_diff_line(
    line: &diff::DiffLine,
    sub: u32,
    sub_range: Range<usize>,
    indent_px: Pixels,
    language: Option<&'static Language>,
    theme: ThemePreset,
    font: &FontConfig,
    gutter_left: Pixels,
    gutter_block_w: Pixels,
    content_ml: Pixels,
) -> impl IntoElement {
    // No `+`/`-` glyphs — a colored left-edge bar marks a changed row
    // instead (matches how GitHub/most PR diff views do it), so `marker`
    // is now just the bar's color, `None` for context rows (no bar at all).
    let (marker, bg) = match line.kind {
        diff::DiffLineKind::Added => (Some(rgb(0x3fb950)), Some(rgba(0x2ea04326))),
        diff::DiffLineKind::Removed => (Some(rgb(0xf85149)), Some(rgba(0xf8514926))),
        diff::DiffLineKind::Context => (None, None),
    };

    let full_chars: Vec<char> = line.text.chars().collect();
    let row_len = full_chars.len();
    let sub_start = sub_range.start.min(row_len);
    let sub_end = sub_range.end.min(row_len);
    let sub_text: String = if sub_start < sub_end {
        full_chars[sub_start..sub_end].iter().collect()
    } else {
        String::new()
    };

    let highlighted = syntax::highlight_themed(&line.text, language, 0, Some(theme));
    let runs: Vec<(std::ops::Range<usize>, Hsla)> = highlighted
        .spans
        .iter()
        .filter_map(|s| {
            let clipped = clip_to_subrange(&(s.start..s.end), &sub_range)?;
            let color = s
                .color
                .map(|(r, g, b)| rgb(((r as u32) << 16) | ((g as u32) << 8) | b as u32).into())
                .unwrap_or_else(|| color_for(s.capture));
            Some((clipped, color))
        })
        .collect();

    let mut row = div()
        .relative()
        .flex()
        .flex_row()
        .items_center()
        .w_full()
        .h(font.line_height)
        .font_family(font.family.clone())
        .text_size(font.size)
        // Colored left-edge bar for a changed row; invisible (matching
        // border width reserved either way, so layout doesn't shift
        // between changed/context rows) when there's no marker color.
        // Lives at the row's scrolled edge (not in the frozen gutter), so
        // it slides away with content — the numbers stay, the bar doesn't.
        .border_l_4()
        .border_color(marker.unwrap_or(transparent_black().into()));
    if let Some(bg) = bg {
        row = row.bg(bg);
    }

    let old_label = if sub == 0 {
        gutter_number(line.old_line)
    } else {
        " ".repeat(4)
    };
    let new_label = if sub == 0 {
        gutter_number(line.new_line)
    } else {
        " ".repeat(4)
    };
    row.child(
        div()
            .flex()
            .flex_row()
            .flex_1()
            .flex_shrink_0()
            .ml(content_ml)
            .pl(indent_px)
            .child(render_spans(sub_text, runs, None)),
    )
    // Frozen gutter block (both number columns): pinned at the live scroll
    // shift with an opaque cover, same trick as `EditorView`'s row gutter.
    .child(
        div()
            .absolute()
            .left(gutter_left)
            .top_0()
            .w(gutter_block_w)
            .h_full()
            .bg(Hsla::black())
            .flex()
            .flex_row()
            .child(
                div()
                    .w_10()
                    .flex_shrink_0()
                    .text_color(rgb(0x585b70))
                    .child(old_label),
            )
            .child(
                div()
                    .w_10()
                    .h_full()
                    .flex_shrink_0()
                    .border_r_2()
                    .border_color(rgba(0xffffff1a))
                    .text_color(rgb(0x585b70))
                    .child(new_label),
            ),
    )
}

impl Render for DiffView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let state_snapshot = self.state.read(cx);
        let diff_version = state_snapshot.version();
        let wrap_enabled = state_snapshot.wrap_enabled();
        let font = state_snapshot.font().clone();

        let measured_height = self.viewport_height.get();
        let measured_width = self.viewport_width.get();
        let rows_that_fit = (measured_height / font.line_height).floor();
        let list_height = if rows_that_fit > 0. {
            font.line_height * rows_that_fit
        } else {
            measured_height
        };

        let wrap_width = if wrap_enabled && measured_width > DIFF_WRAP_GUTTER_RESERVE {
            Some(round_wrap_width(measured_width - DIFF_WRAP_GUTTER_RESERVE))
        } else if wrap_enabled {
            Some(px(400.))
        } else {
            None
        };

        let char_width = measure_char_width(&self.char_width, &font, window);

        let mut wrap_cache = self.wrap_cache.borrow_mut();
        if wrap_cache.is_stale(wrap_width, diff_version, &font) {
            let lines = &state_snapshot.result.lines;
            *wrap_cache = WrapCache::rebuild(
                lines.len() as u32,
                |row| lines[row as usize].text.clone(),
                diff_version,
                wrap_width,
                &font,
                char_width,
                window,
            );
        }
        let visual_rows = wrap_cache.visual_row_count();
        drop(wrap_cache);

        // Same cached widest-line scan as `EditorView`: O(total chars) per
        // diff version, never per frame.
        let max_text_px = match self.content_width_cache.borrow().clone() {
            Some((v, f, w)) if v == diff_version && f == font => w,
            _ => {
                let w = state_snapshot.max_line_width(char_width);
                *self.content_width_cache.borrow_mut() =
                    Some((diff_version, font.clone(), w));
                w
            }
        };

        let rem = window.rem_size();
        // Old layout was `px_2` row pad + two `w_10` number columns; text
        // started at 5.5rem. Kept identical so wrap widths don't shift.
        let gutter_pad = rem * 0.5;
        let gutter_block_w = rem * 5.0;
        let content_ml = rem * 5.5;
        const H_TRAILING_PAD: Pixels = px(64.0);
        let content_width = if wrap_enabled {
            None
        } else {
            Some(content_ml + max_text_px + H_TRAILING_PAD)
        };
        let scroll_x = (-self.h_handle.offset().x).max(px(0.));
        let gutter_left = scroll_x + gutter_pad;
        let h_scrollable = self.h_handle.max_offset().x > px(0.);

        let viewport_height = self.viewport_height.clone();
        let viewport_width = self.viewport_width.clone();
        let wrap_cache_handle = self.wrap_cache.clone();

        div()
            .id("gpui-diff-view")
            .relative()
            .size_full()
            .bg(Hsla::black())
            .text_color(Hsla::white())
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
                div()
                    .id("gpui-diff-hscroll")
                    // Both axes set (see `EditorView`): vertical clamps at
                    // zero here, so wheel gestures never drift sideways.
                    .overflow_scroll()
                    .track_scroll(&self.h_handle)
                    .size_full()
                    .child({
                        let list = uniform_list(
                            "gpui-diff-lines",
                            visual_rows,
                            cx.processor(move |this, range: std::ops::Range<usize>, _window, cx| {
                        let state = this.state.read(cx);
                        let language = state.language();
                        let theme = state.theme();
                        let font = state.font();
                        let cache = wrap_cache_handle.borrow();
                        range
                            .map(|ix| {
                                let (row, sub) = cache.resolve(ix);
                                let line = &state.result.lines[row as usize];
                                let sub_range = cache.sub_range(row, sub, line.text.chars().count());
                                let indent_px = if sub > 0 {
                                    char_width * (cache.indent_chars(row) as f32)
                                } else {
                                    px(0.)
                                };
                                render_diff_line(
                                    line,
                                    sub,
                                    sub_range,
                                    indent_px,
                                    language,
                                    theme,
                                    font,
                                    gutter_left,
                                    gutter_block_w,
                                    content_ml,
                                )
                            })
                            .collect()
                    }))
                        .track_scroll(&self.scroll_handle)
                        .text_sm()
                        .h(list_height);
                        match content_width {
                            Some(w) => list.w(w),
                            None => list.w_full(),
                        }
                    }),
            )
            .when(self.scroll_handle.is_scrollable(), |el| {
                el.child(render_scrollbar(
                    self.scroll_handle.clone(),
                    self.thumb_dragging.clone(),
                ))
            })
            .when(h_scrollable, |el| {
                el.child(render_h_scrollbar(
                    self.h_handle.clone(),
                    self.h_thumb_dragging.clone(),
                ))
            })
            .on_mouse_move({
                let scroll_handle = self.scroll_handle.clone();
                let thumb_dragging = self.thumb_dragging.clone();
                let h_handle = self.h_handle.clone();
                let h_thumb_dragging = self.h_thumb_dragging.clone();
                move |event, window, _cx| {
                    if let Some((start_mouse_y, start_offset_y)) = thumb_dragging.get() {
                        let base = scroll_handle.0.borrow().base_handle.clone();
                        let track_height = base.bounds().size.height;
                        let max_offset_y = base.max_offset().y;
                        let thumb_height = crate::scrollbar_thumb_height(track_height, max_offset_y);
                        let track_range = (track_height - thumb_height).max(px(1.));
                        let delta = event.position.y - start_mouse_y;
                        let new_offset_y = (start_offset_y - delta * (max_offset_y / track_range))
                            .clamp(-max_offset_y, px(0.));
                        base.set_offset(point(base.offset().x, new_offset_y));
                        window.refresh();
                    }
                    if let Some((start_mouse_x, start_offset_x)) = h_thumb_dragging.get() {
                        let track_width = h_handle.bounds().size.width;
                        let max_offset_x = h_handle.max_offset().x;
                        let thumb_width = scrollbar_thumb_size(track_width, max_offset_x);
                        let track_range = (track_width - thumb_width).max(px(1.));
                        let delta = event.position.x - start_mouse_x;
                        let new_offset_x = (start_offset_x - delta * (max_offset_x / track_range))
                            .clamp(-max_offset_x, px(0.));
                        h_handle.set_offset(point(new_offset_x, h_handle.offset().y));
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
