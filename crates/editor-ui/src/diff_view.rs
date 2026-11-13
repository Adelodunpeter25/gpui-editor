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
use std::collections::HashMap;
use std::ops::Range;
use std::rc::Rc;
use syntax::{Language, ThemePreset};

use crate::types::{diff_comment_target, diff_line_number, DiffCommentTarget, DiffSide, FontConfig};
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
    h_thumb_dragging: Rc<Cell<Option<(Pixels, Pixels)>>>,
    /// Cached widest-line text width: `(diff version, font, max px)`.
    content_width_cache: Rc<RefCell<Option<(u64, FontConfig, Pixels)>>>,
    /// Opt-in comment gutter: set via `on_comment_target`.
    comment_handler: Option<Rc<dyn Fn(DiffCommentTarget)>>,
    comment_selections: Vec<DiffCommentTarget>,
    comment_hover_row: Rc<Cell<Option<usize>>>,
    /// `(side, starting row index)` while a gutter drag is in progress.
    comment_drag: Rc<Cell<Option<(DiffSide, usize)>>>,
    overlays: Vec<DiffRowOverlay>,
    overlay_gen: u64,
    overlay_layout: Rc<RefCell<Option<(OverlayKey, Rc<OverlayLayout>)>>>,
}

type OverlayKey = (u64, u64, Option<Pixels>, FontConfig);

/// A host-supplied element shown under a diff row (e.g. an inline comment
/// card). Anchored below the row carrying `line` on `side`; it occupies
/// `rows` extra line-heights, so rows below shift down and the list stays
/// uniform-height (virtualization math is unchanged).
#[derive(Clone)]
pub struct DiffRowOverlay {
    pub side: DiffSide,
    pub line: u32,
    /// Height in line-heights (min 1). The element is clipped to it.
    pub rows: u32,
    pub builder: Rc<dyn Fn(&mut Window, &mut App) -> AnyElement>,
}

impl DiffRowOverlay {
    pub fn new(
        side: DiffSide,
        line: u32,
        rows: u32,
        builder: impl Fn(&mut Window, &mut App) -> AnyElement + 'static,
    ) -> Self {
        Self {
            side,
            line,
            rows,
            builder: Rc::new(builder),
        }
    }
}

struct OverlayInsert {
    /// List index of the first filler row.
    start: usize,
    rows: usize,
    overlay: usize,
    /// Filler rows inserted before this one.
    before: usize,
}

#[derive(Default)]
struct OverlayLayout {
    inserts: Vec<OverlayInsert>,
    total_rows: usize,
}

enum ListItem {
    /// Visual row index.
    Line(usize),
    Filler { overlay: usize, slice: usize },
}

impl OverlayLayout {
    fn resolve(&self, ix: usize) -> ListItem {
        let p = self.inserts.partition_point(|i| i.start <= ix);
        if p == 0 {
            return ListItem::Line(ix);
        }
        let i = &self.inserts[p - 1];
        if ix < i.start + i.rows {
            ListItem::Filler {
                overlay: i.overlay,
                slice: ix - i.start,
            }
        } else {
            ListItem::Line(ix - i.before - i.rows)
        }
    }
}

impl DiffView {
    /// Opt in to the comment gutter: hover `+`, click or drag rows in the
    /// gutter, and receive the picked range.
    pub fn on_comment_target(&mut self, handler: impl Fn(DiffCommentTarget) + 'static) {
        self.comment_handler = Some(Rc::new(handler));
    }

    /// Opt back out (removes the callback, the `+` and gutter hit-testing).
    pub fn clear_comment_target_handler(&mut self) {
        self.comment_handler = None;
    }

    /// Ranges that currently have a comment/draft; their rows are tinted.
    pub fn set_comment_selections(
        &mut self,
        selections: Vec<DiffCommentTarget>,
        cx: &mut Context<Self>,
    ) {
        self.comment_selections = selections;
        cx.notify();
    }

    /// Replace the row overlays (see `DiffRowOverlay`).
    pub fn set_row_overlays(&mut self, overlays: Vec<DiffRowOverlay>, cx: &mut Context<Self>) {
        self.overlays = overlays;
        self.overlay_gen = self.overlay_gen.wrapping_add(1);
        cx.notify();
    }
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
            h_thumb_dragging: Rc::new(Cell::new(None)),
            content_width_cache: Rc::new(RefCell::new(None)),
            comment_handler: None,
            comment_selections: Vec::new(),
            comment_hover_row: Rc::new(Cell::new(None)),
            comment_drag: Rc::new(Cell::new(None)),
            overlays: Vec::new(),
            overlay_gen: 0,
            overlay_layout: Rc::new(RefCell::new(None)),
        }
    }
}

/// Gutter number column: right-aligned, blank (not `0`) when a line has no
/// counterpart on that side (a pure add/remove).
fn gutter_number(n: Option<u32>) -> String {
    n.map(|n| format!("{n:>4}"))
        .unwrap_or_else(|| " ".repeat(4))
}

/// Bundles a diff row's positional bookkeeping — keeps `render_diff_line`'s
/// own argument count from creeping past what a single glance can take in
/// (mirrors `EditorView`'s `RowMeta` in `lib.rs`).
struct DiffRowMeta {
    sub: u32,
    sub_range: Range<usize>,
    indent_px: Pixels,
    gutter_pad: Pixels,
    gutter_block_w: Pixels,
    content_ml: Pixels,
    /// `Some(content width)` while h-scrolling: the list is viewport-width
    /// then (gpui clamps the scroll offset against the scroll container's
    /// own bounds, so a content-width list would have zero horizontal range
    /// — see `EditorView::render` and `RowMeta::row_width` for the full
    /// explanation), so the row states its content width explicitly and
    /// keeps the layout it had when the list was content-width.
    row_width: Option<Pixels>,
    comment: RowComment,
}

/// Per-row comment-gutter state (all-default = feature off).
#[derive(Default)]
struct RowComment {
    row: usize,
    /// Handler set: row tracks hover and gutter takes mouse-down.
    enabled: bool,
    /// Inside a persisted selection or the in-progress drag.
    lit: bool,
    /// Show the `+` chip (sub-row 0 under the pointer).
    plus: bool,
    hover: Option<Rc<Cell<Option<usize>>>>,
    drag: Option<Rc<Cell<Option<(DiffSide, usize)>>>>,
}

fn render_diff_line(
    line: &diff::DiffLine,
    language: Option<&'static Language>,
    theme: ThemePreset,
    font: &FontConfig,
    meta: DiffRowMeta,
) -> impl IntoElement {
    let DiffRowMeta {
        sub,
        sub_range,
        indent_px,
        gutter_pad,
        gutter_block_w,
        content_ml,
        row_width,
        comment,
    } = meta;
    // No `+`/`-` glyphs — a colored left-edge bar marks a changed row
    // instead (matches how GitHub/most PR diff views do it), so `marker`
    // is now just the bar's color, `None` for context rows (no bar at all).
    let (marker, bg) = match line.kind {
        diff::DiffLineKind::Added => (Some(rgb(0x3fb950)), Some(rgba(0x2ea04326))),
        diff::DiffLineKind::Removed => (Some(rgb(0xf85149)), Some(rgba(0xf8514926))),
        diff::DiffLineKind::Context => (None, None),
    };

    let line_kind = line.kind;
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

    let row = div()
        .relative()
        .flex()
        .flex_row()
        .items_center()
        .h(font.line_height);
    // While h-scrolling, the row outgrows the (viewport-width) list on
    // purpose — see `DiffRowMeta::row_width`. Otherwise it stretches to
    // the list as always.
    let row = match row_width {
        Some(w) => row.w(w),
        None => row.w_full(),
    };
    let mut row = row
        .font_family(font.family.clone())
        .text_size(font.size)
        // Colored left-edge bar for a changed row; invisible (matching
        // border width reserved either way, so layout doesn't shift
        // between changed/context rows) when there's no marker color.
        // Lives at the row's own left edge: under h-scroll it slides out
        // with the row like everything else, clipped by the container.
        .border_l_4()
        .border_color(marker.unwrap_or(transparent_black().into()));
    if let Some(bg) = bg {
        row = row.bg(bg);
    }
    if comment.lit {
        row = row.bg(rgba(0xfacc1538));
    }
    if comment.enabled {
        let hover = comment.hover.clone().expect("hover handle when enabled");
        let row_ix = comment.row;
        row = row
            .on_mouse_move({
                let hover = hover.clone();
                move |_e, window, _cx| {
                    if hover.get() != Some(row_ix) {
                        hover.set(Some(row_ix));
                        window.refresh();
                    }
                }
            })
            .on_mouse_exit({
                let drag = comment.drag.clone().expect("drag handle when enabled");
                move |_e, window, _cx| {
                    if drag.get().is_none() {
                        hover.set(None);
                        window.refresh();
                    }
                }
            });
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
    // Gutter block (both number columns): plain row content, same as
    // `EditorView`'s gutter — it rides the row's horizontal translation
    // and the container's left edge clips it as it scrolls out of view
    // (no conditional unmount, no pinned overlay).
    .child(
        div()
            .absolute()
            .left(gutter_pad)
            .top_0()
            .w(gutter_block_w)
            .h_full()
            .bg(if comment.lit {
                rgb(0x3a3418).into()
            } else {
                Hsla::black()
            })
            .flex()
            .flex_row()
            .when(comment.plus, |el| {
                el.child(
                    div()
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
            })
            .when(comment.enabled, |el| {
                let hover = comment.hover.clone().expect("hover handle when enabled");
                let drag = comment.drag.clone().expect("drag handle when enabled");
                let row_ix = comment.row;
                let side = if line_kind == diff::DiffLineKind::Removed {
                    DiffSide::Old
                } else {
                    DiffSide::New
                };
                el.on_mouse_down(MouseButton::Left, move |_e, window, _cx| {
                    hover.set(Some(row_ix));
                    drag.set(Some((side, row_ix)));
                    window.refresh();
                })
            })
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

/// True when a row falls inside a persisted selection or the live drag.
fn row_is_lit(
    line: &diff::DiffLine,
    selections: &[DiffCommentTarget],
    drag: Option<DiffCommentTarget>,
) -> bool {
    selections.iter().chain(drag.as_ref()).any(|t| {
        diff_line_number(line, t.side).is_some_and(|n| n >= t.start_line && n <= t.end_line)
    })
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
        let line_visual_rows = wrap_cache.visual_row_count();
        let overlay_layout: Rc<OverlayLayout> = if self.overlays.is_empty() {
            Rc::new(OverlayLayout {
                inserts: Vec::new(),
                total_rows: line_visual_rows,
            })
        } else {
            let key: OverlayKey = (diff_version, self.overlay_gen, wrap_width, font.clone());
            let cached = self.overlay_layout.borrow().clone();
            match cached {
                Some((k, layout)) if k == key => layout,
                _ => {
                    let lines = &state_snapshot.result.lines;
                    let mut anchors: Vec<(usize, usize)> = Vec::new(); // (last visual row, overlay)
                    for (oi, o) in self.overlays.iter().enumerate() {
                        let Some(row) = lines
                            .iter()
                            .position(|l| diff_line_number(l, o.side) == Some(o.line))
                        else {
                            continue;
                        };
                        let last = wrap_cache
                            .first_visual_row(row as u32 + 1)
                            .map(|v| v - 1)
                            .unwrap_or(line_visual_rows.saturating_sub(1));
                        anchors.push((last, oi));
                    }
                    anchors.sort();
                    let mut before = 0;
                    let mut inserts = Vec::new();
                    for (last, oi) in anchors {
                        let rows = self.overlays[oi].rows.max(1) as usize;
                        inserts.push(OverlayInsert {
                            start: last + 1 + before,
                            rows,
                            overlay: oi,
                            before,
                        });
                        before += rows;
                    }
                    let layout = Rc::new(OverlayLayout {
                        inserts,
                        total_rows: line_visual_rows + before,
                    });
                    *self.overlay_layout.borrow_mut() = Some((key, layout.clone()));
                    layout
                }
            }
        };
        let visual_rows = overlay_layout.total_rows;
        drop(wrap_cache);

        let comment_enabled = self.comment_handler.is_some();
        let comment_selections = self.comment_selections.clone();
        let comment_hover = self.comment_hover_row.clone();
        let comment_drag = self.comment_drag.clone();
        let drag_target = comment_drag.get().and_then(|(_, start)| {
            let end = comment_hover.get().unwrap_or(start);
            diff_comment_target(&state_snapshot.result.lines, start, end)
        });
        let overlay_items: Rc<Vec<DiffRowOverlay>> = Rc::new(self.overlays.clone());

        // Same cached widest-line scan as `EditorView`: O(total chars) per
        // diff version, never per frame — and, same as `EditorView`, only
        // when wrap is off, since wrapped text never needs a horizontal
        // range. (Bound to a local first — see the borrow-guard note on
        // `EditorView`'s identical cache.)
        let max_text_px = if wrap_enabled {
            px(0.)
        } else {
            let cached_width = self.content_width_cache.borrow().clone();
            match cached_width {
                Some((v, f, w)) if v == diff_version && f == font => w,
                _ => {
                    let w = state_snapshot.max_line_width(char_width);
                    *self.content_width_cache.borrow_mut() = Some((diff_version, font.clone(), w));
                    w
                }
            }
        };

        let rem = window.rem_size();
        // Old layout was `px_2` row pad + two `w_10` number columns; text
        // started at 5.5rem. Kept identical so wrap widths don't shift.
        let gutter_pad = rem * 0.5;
        let gutter_block_w = rem * 5.0;
        let content_ml = rem * 5.5;
        // Same handle as the vertical scroll's `base_handle`, not a second
        // nested scrollable container — see `EditorView::render`'s comment
        // on why (avoids gpui dispatching the same wheel gesture to two
        // independent scroll regions, which is what let horizontal-swipe
        // noise leak into vertical scroll).
        let base_handle = self.scroll_handle.0.borrow().base_handle.clone();
        // Only earn the horizontal-scroll layout when a line actually
        // overflows the viewport — see `EditorView`'s identical gate.
        let can_h_scroll = !wrap_enabled && content_ml + max_text_px > measured_width;
        if !can_h_scroll {
            let y = base_handle.offset().y;
            base_handle.set_offset(point(px(0.), y));
        }
        const H_TRAILING_PAD: Pixels = px(64.0);
        let content_width = can_h_scroll.then(|| content_ml + max_text_px + H_TRAILING_PAD);
        let h_scrollable = can_h_scroll && base_handle.max_offset().x > px(0.);

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
            .child({
                let mut list = uniform_list(
                    "gpui-diff-lines",
                    visual_rows,
                    cx.processor(move |this, range: std::ops::Range<usize>, _window, cx| {
                        let line_height = this.state.read(cx).font().line_height;
                        // Overlay builders need `&mut App`, so build them
                        // before `state` borrows `cx`.
                        let mut fillers: HashMap<usize, AnyElement> = HashMap::new();
                        for ix in range.clone() {
                            if let ListItem::Filler { overlay, slice } = overlay_layout.resolve(ix) {
                                let o = &overlay_items[overlay];
                                let lh = line_height;
                                // Every filler row paints the card shifted up by its
                                // slice and clipped to its own row, so the card spans
                                // `rows` uniform rows seamlessly.
                                let card = div()
                                    .absolute()
                                    .top(-(lh * slice as f32))
                                    .left(content_ml)
                                    .right(px(8.))
                                    .h(lh * o.rows.max(1) as f32)
                                    .child((o.builder)(_window, cx));
                                let filler = div()
                                    .relative()
                                    .overflow_hidden()
                                    .h(lh)
                                    .border_l_4()
                                    .border_color(transparent_black());
                                let filler = match content_width {
                                    Some(w) => filler.w(w),
                                    None => filler.w_full(),
                                };
                                fillers.insert(ix, filler.child(card).into_any_element());
                            }
                        }
                        let state = this.state.read(cx);
                        let language = state.language();
                        let theme = state.theme();
                        let font = state.font();
                        let cache = wrap_cache_handle.borrow();
                        range
                            .map(|ix| {
                                let ix = match overlay_layout.resolve(ix) {
                                    ListItem::Line(v) => v,
                                    ListItem::Filler { .. } => {
                                        return fillers.remove(&ix).expect("prebuilt filler");
                                    }
                                };
                                let (row, sub) = cache.resolve(ix);
                                let line = &state.result.lines[row as usize];
                                let sub_range =
                                    cache.sub_range(row, sub, line.text.chars().count());
                                let indent_px = if sub > 0 {
                                    char_width * (cache.indent_chars(row) as f32)
                                } else {
                                    px(0.)
                                };
                                render_diff_line(
                                    line,
                                    language,
                                    theme,
                                    font,
                                    DiffRowMeta {
                                        sub,
                                        sub_range,
                                        indent_px,
                                        gutter_pad,
                                        gutter_block_w,
                                        content_ml,
                                        row_width: content_width,
                                        comment: RowComment {
                                            row: row as usize,
                                            enabled: comment_enabled,
                                            lit: true
                                                && row_is_lit(
                                                    line,
                                                    &comment_selections,
                                                    drag_target,
                                                ),
                                            plus: comment_enabled
                                                && sub == 0
                                                && (comment_hover.get() == Some(row as usize)
                                                    || comment_drag
                                                        .get()
                                                        .is_some_and(|(_, s)| s == row as usize)),
                                            hover: comment_enabled.then(|| comment_hover.clone()),
                                            drag: comment_enabled.then(|| comment_drag.clone()),
                                        },
                                    },
                                )
                                .into_any_element()
                            })
                            .collect()
                    }),
                )
                .track_scroll(&self.scroll_handle)
                .text_sm()
                .h(list_height);

                if can_h_scroll {
                    // `Unconstrained` (which also sets `overflow.x`) is what
                    // gives gpui a non-zero horizontal scroll range to clamp
                    // against; `restrict_scroll_to_axis` (poked — it's a
                    // `StatefulInteractiveElement` method `UniformList`
                    // doesn't implement) locks each gesture to its axis.
                    // Both only when h-scroll is possible — see the
                    // identical (and much more detailed) block in
                    // `EditorView::render`.
                    list = list.with_horizontal_sizing_behavior(
                        ListHorizontalSizingBehavior::Unconstrained,
                    );
                    list.interactivity().base_style.restrict_scroll_to_axis = Some(true);
                }
                // Always viewport-width — the scroll bounds gpui clamps
                // against, and the mask rows paint through; the rows
                // themselves carry `content_width` (see `row_width`).
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
                        let thumb_height =
                            crate::scrollbar_thumb_height(track_height, max_offset_y);
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
                let state = self.state.clone();
                let handler = self.comment_handler.clone();
                let comment_drag = self.comment_drag.clone();
                let comment_hover = self.comment_hover_row.clone();
                move |_event, window, cx| {
                    thumb_dragging.set(None);
                    h_thumb_dragging.set(None);
                    if let Some((_, start)) = comment_drag.take() {
                        let end = comment_hover.get().unwrap_or(start);
                        let target =
                            diff_comment_target(&state.read(cx).result.lines, start, end);
                        if let (Some(target), Some(handler)) = (target, &handler) {
                            handler(target);
                        }
                        window.refresh();
                    }
                }
            })
    }
}
