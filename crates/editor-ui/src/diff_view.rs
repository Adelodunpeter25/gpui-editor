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
use std::cell::Cell;
use std::rc::Rc;
use syntax::{Language, ThemePreset};

use crate::types::FontConfig;
use crate::{color_for, render_scrollbar, render_spans};

pub struct DiffState {
    pub(crate) result: diff::DiffResult,
    language: Option<&'static Language>,
    theme: ThemePreset,
    font: FontConfig,
}

impl DiffState {
    pub fn new(old: &str, new: &str, language: Option<&'static Language>) -> Self {
        Self {
            result: diff::diff_lines(old, new),
            language,
            theme: ThemePreset::GitHubDark,
            font: FontConfig::default(),
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

    pub fn set_texts(&mut self, old: &str, new: &str) {
        self.result = diff::diff_lines(old, new);
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

    pub fn language(&self) -> Option<&'static Language> {
        self.language
    }

    pub fn theme(&self) -> ThemePreset {
        self.theme
    }

    pub fn font(&self) -> &FontConfig {
        &self.font
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
}

pub struct DiffView {
    state: Entity<DiffState>,
    scroll_handle: UniformListScrollHandle,
    viewport_height: Rc<Cell<Pixels>>,
    thumb_dragging: Rc<Cell<Option<(Pixels, Pixels)>>>,
}

impl DiffView {
    pub fn new(state: &Entity<DiffState>) -> Self {
        Self {
            state: state.clone(),
            scroll_handle: UniformListScrollHandle::new(),
            viewport_height: Rc::new(Cell::new(px(0.))),
            thumb_dragging: Rc::new(Cell::new(None)),
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
    language: Option<&'static Language>,
    theme: ThemePreset,
    font: &FontConfig,
) -> impl IntoElement {
    let (marker, bg, marker_color) = match line.kind {
        diff::DiffLineKind::Added => ("+", Some(rgba(0x2ea04326)), rgb(0x3fb950)),
        diff::DiffLineKind::Removed => ("-", Some(rgba(0xf8514926)), rgb(0xf85149)),
        diff::DiffLineKind::Context => (" ", None, rgb(0x585b70)),
    };

    let highlighted = syntax::highlight_themed(&line.text, language, 0, Some(theme));
    let runs: Vec<(std::ops::Range<usize>, Hsla)> = highlighted
        .spans
        .iter()
        .map(|s| {
            let color = s
                .color
                .map(|(r, g, b)| rgb(((r as u32) << 16) | ((g as u32) << 8) | b as u32).into())
                .unwrap_or_else(|| color_for(s.capture));
            (s.start..s.end, color)
        })
        .collect();

    let mut row = div()
        .flex()
        .flex_row()
        .items_center()
        .px_2()
        .h(font.line_height)
        .font_family(font.family.clone())
        .text_size(font.size);
    if let Some(bg) = bg {
        row = row.bg(bg);
    }

    row.child(
        div()
            .w_10()
            .flex_shrink_0()
            .text_color(rgb(0x585b70))
            .child(gutter_number(line.old_line)),
    )
    .child(
        div()
            .w_10()
            .flex_shrink_0()
            .text_color(rgb(0x585b70))
            .child(gutter_number(line.new_line)),
    )
    .child(
        div()
            .w_4()
            .flex_shrink_0()
            .text_color(marker_color)
            .child(marker),
    )
    .child(
        div()
            .flex()
            .flex_row()
            .flex_1()
            .overflow_x_hidden()
            .child(render_spans(line.text.clone(), runs, None)),
    )
}

impl Render for DiffView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let line_count = self.state.read(cx).line_count();
        let font = self.state.read(cx).font().clone();

        let measured = self.viewport_height.get();
        let rows_that_fit = (measured / font.line_height).floor();
        let list_height = if rows_that_fit > 0. {
            font.line_height * rows_that_fit
        } else {
            measured
        };
        let viewport_height = self.viewport_height.clone();

        div()
            .id("gpui-diff-view")
            .relative()
            .size_full()
            .bg(Hsla::black())
            .text_color(Hsla::white())
            .child(
                canvas(
                    move |bounds, _window, _cx| viewport_height.set(bounds.size.height),
                    |_, _, _, _| {},
                )
                .absolute()
                .size_full(),
            )
            .child(
                uniform_list(
                    "gpui-diff-lines",
                    line_count,
                    cx.processor(move |this, range: std::ops::Range<usize>, _window, cx| {
                        let state = this.state.read(cx);
                        let language = state.language();
                        let theme = state.theme();
                        let font = state.font();
                        range
                            .map(|ix| {
                                render_diff_line(&state.result.lines[ix], language, theme, font)
                            })
                            .collect()
                    }),
                )
                .track_scroll(&self.scroll_handle)
                .text_sm()
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
                    let thumb_height = crate::scrollbar_thumb_height(track_height, max_offset_y);
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
