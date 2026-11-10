//! `EditorView` — the retained view handle (the host app's `Entity` seam):
//! wraps `Entity<EditorState>` plus the scroll/focus/caches that `render` and
//! its mouse handlers share frame to frame. The `Render` impl and paint
//! helpers live in `lib.rs` (shared with `diff_view.rs`); the model this
//! points at lives in `state.rs`.

use gpui::{
    App, ClipboardItem, Context, Entity, FocusHandle, Pixels, ScrollHandle,
    UniformListScrollHandle, Window, px,
};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

use crate::state::EditorState;
use crate::types::{CommentTarget, FontConfig};
use crate::wrap::WrapCache;
use crate::{Copy, Cut, Paste, Redo, SelectAll, Undo};

pub struct EditorView {
    pub(crate) state: Entity<EditorState>,
    pub(crate) scroll_handle: UniformListScrollHandle,
    pub(crate) focus_handle: FocusHandle,
    /// Last measured pixel height of the editor viewport, captured by a
    /// `canvas` each frame. Used to snap the visible list height to a whole
    /// multiple of `LINE_HEIGHT` so scrolling never shows a half-clipped row.
    pub(crate) viewport_height: Rc<Cell<Pixels>>,
    /// Monospace glyph advance width, measured lazily on first mouse
    /// interaction and cached alongside the `FontConfig` it was measured
    /// for, so a runtime font change invalidates it instead of silently
    /// reusing a stale width for hit-testing.
    pub(crate) char_width: Rc<RefCell<Option<(FontConfig, Pixels)>>>,
    /// True while a left-mouse selection drag is in progress.
    pub(crate) selecting: Rc<Cell<bool>>,
    /// Fixed end of the in-progress drag; the other end follows the mouse.
    pub(crate) drag_anchor: Rc<Cell<Option<usize>>>,
    /// Buffer offset the drag last actually moved the selection head to, so
    /// a mouse-move that resolves to the same character (e.g. y-only
    /// movement, or two pixel positions rounding to the same column) can be
    /// skipped instead of re-running `set_selection` + a repaint every time.
    pub(crate) last_head: Rc<Cell<Option<usize>>>,
    /// `Some((mouse_y_at_down, scroll_offset_y_at_down))` while the scrollbar
    /// thumb is being dragged; `None` otherwise.
    pub(crate) thumb_dragging: Rc<Cell<Option<(Pixels, Pixels)>>>,
    /// `Some((mouse_x_at_down, scroll_offset_x_at_down))` while the
    /// horizontal scrollbar thumb is being dragged; `None` otherwise.
    pub(crate) h_thumb_dragging: Rc<Cell<Option<(Pixels, Pixels)>>>,
    pub(crate) comment_hover_row: Rc<Cell<Option<u32>>>,
    pub(crate) comment_drag: Rc<Cell<Option<u32>>>,
    /// Cached widest-row text width: `(buffer version, font, max px)`.
    /// Recomputed only when the buffer or font changes, not per frame —
    /// the scan is O(total chars), same cost class as a wrap rebuild.
    pub(crate) content_width_cache: Rc<RefCell<Option<(u64, FontConfig, Pixels)>>>,
    /// Last measured pixel width of the editor viewport (same `canvas` that
    /// measures `viewport_height`). Drives word-wrap's wrap width.
    pub(crate) viewport_width: Rc<Cell<Pixels>>,
    /// Buffer-row <-> visual-row mapping for word wrap. `RefCell` since both
    /// `render` (rebuilds it) and row mouse handlers (read it, to resolve a
    /// click's visual row back to a buffer offset) need access.
    pub(crate) wrap_cache: Rc<RefCell<WrapCache>>,
    /// Anchor end for keyboard selection extension (shift + motion): where
    /// the selection started before the current keyboard drag, so the head
    /// moves while the anchor stays put. Cleared by any non-shift motion,
    /// click, or edit.
    pub(crate) key_anchor: Rc<Cell<Option<usize>>>,
    /// Column the caret last aimed at for vertical motion (up/down), so
    /// crossing a shorter line doesn't drift horizontally. In char columns;
    /// cleared by horizontal motion, clicks, and edits.
    pub(crate) goal_col: Rc<Cell<Option<u32>>>,
    /// Set by keyboard edits and caret motion; the next render scrolls the
    /// caret back into the viewport and clears it. Deferred to render
    /// because that's where the wrap cache and measurements are current.
    pub(crate) reveal_caret: Rc<Cell<bool>>,
}

impl EditorView {
    pub fn new(state: &Entity<EditorState>, cx: &mut Context<Self>) -> Self {
        Self {
            state: state.clone(),
            scroll_handle: UniformListScrollHandle::new(),
            focus_handle: cx.focus_handle(),
            viewport_height: Rc::new(Cell::new(px(0.))),
            char_width: Rc::new(RefCell::new(None)),
            selecting: Rc::new(Cell::new(false)),
            drag_anchor: Rc::new(Cell::new(None)),
            last_head: Rc::new(Cell::new(None)),
            thumb_dragging: Rc::new(Cell::new(None)),
            h_thumb_dragging: Rc::new(Cell::new(None)),
            comment_hover_row: Rc::new(Cell::new(None)),
            comment_drag: Rc::new(Cell::new(None)),
            content_width_cache: Rc::new(RefCell::new(None)),
            viewport_width: Rc::new(Cell::new(px(0.))),
            wrap_cache: Rc::new(RefCell::new(WrapCache::default())),
            key_anchor: Rc::new(Cell::new(None)),
            goal_col: Rc::new(Cell::new(None)),
            reveal_caret: Rc::new(Cell::new(false)),
        }
    }

    /// Scroll the caret into the viewport on the next frame (no-op if it's
    /// already visible). Keyboard edits and motion do this themselves; hosts
    /// call it after moving the selection programmatically.
    pub fn scroll_to_caret(&mut self, cx: &mut Context<Self>) {
        self.reveal_caret.set(true);
        cx.notify();
    }

    /// The end of the selection the keyboard is moving: the end opposite
    /// the shift-extension anchor, or the selection's end without one.
    pub(crate) fn selection_head(&self, cx: &App) -> usize {
        let range = self.state.read(cx).selection().range.clone();
        match self.key_anchor.get() {
            Some(anchor) if anchor == range.end => range.start,
            _ => range.end,
        }
    }

    /// Where `offset` paints: its visual row, and its x relative to where
    /// row text starts (continuation-line indent included).
    pub(crate) fn offset_position(
        &self,
        offset: usize,
        char_width: Pixels,
        cx: &App,
    ) -> (usize, Pixels) {
        let state = self.state.read(cx);
        let point = state.offset_to_point(offset);
        let col = point.col as usize;
        let (visual_row, sub_start, indent_chars) =
            self.wrap_cache.borrow().locate(point.row, col);
        let prefix: String = state
            .line_text(point.row)
            .chars()
            .skip(sub_start)
            .take(col.saturating_sub(sub_start))
            .collect();
        let x = char_width * (indent_chars as f32)
            + crate::text_display_width(&prefix, char_width, state.tab_width());
        (visual_row, x)
    }

    /// Bring the caret back into view: vertically via the list's own
    /// nearest-edge scroll, horizontally by nudging the shared scroll
    /// offset. `content_width` is `Some` only while h-scroll is active.
    pub(crate) fn scroll_caret_into_view(
        &self,
        char_width: Pixels,
        content_x: Pixels,
        content_width: Option<Pixels>,
        viewport_width: Pixels,
        cx: &App,
    ) {
        let (visual_row, x) = self.offset_position(self.selection_head(cx), char_width, cx);
        self.scroll_handle
            .scroll_to_item(visual_row, gpui::ScrollStrategy::Nearest);

        let Some(content_width) = content_width else {
            return;
        };
        if viewport_width <= px(0.) {
            return;
        }
        let base = self.scroll_handle.0.borrow().base_handle.clone();
        let offset = base.offset();
        let scroll_x = -offset.x;
        // Keep a few columns of context on whichever side the caret nears.
        let margin = char_width * 4.0;
        let caret_x = content_x + x;
        let mut target = scroll_x;
        if x - margin < scroll_x {
            target = x - margin;
        } else if caret_x + margin > scroll_x + viewport_width {
            target = caret_x + margin - viewport_width;
        }
        let max_x = (content_width - viewport_width).max(px(0.));
        let target = target.clamp(px(0.), max_x);
        if target != scroll_x {
            base.set_offset(gpui::point(-target, offset.y));
        }
    }

    /// Focus the editor so keyboard input (typing, motion, undo/redo)
    /// reaches it. Hosts call this after inserting the view, or users can
    /// just click into it.
    pub fn focus(&self, window: &mut Window, cx: &mut App) {
        window.focus(&self.focus_handle, cx);
    }

    /// Clone of this view's focus handle, for hosts that focus it via
    /// `window.focus` themselves.
    pub fn focus_handle(&self) -> FocusHandle {
        self.focus_handle.clone()
    }

    pub fn scroll_handle(&self) -> &UniformListScrollHandle {
        &self.scroll_handle
    }

    pub fn scroll_to_row(&mut self, row: u32, strategy: gpui::ScrollStrategy) {
        let visual_row = self
            .wrap_cache
            .borrow()
            .first_visual_row(row)
            .unwrap_or(row as usize);
        self.scroll_handle.scroll_to_item_strict(visual_row, strategy);
    }

    pub fn select_line(&mut self, line: u32, cx: &mut Context<Self>) {
        self.state.update(cx, |state, _| state.select_line(line));
        self.scroll_to_row(line.saturating_sub(1), gpui::ScrollStrategy::Center);
        cx.notify();
    }

    pub(crate) fn copy(&mut self, _: &Copy, _window: &mut Window, cx: &mut Context<Self>) {
        let text = self.state.read(cx).selected_text();
        if !text.is_empty() {
            cx.write_to_clipboard(ClipboardItem::new_string(text));
        }
    }

    /// Where the host should place UI attached below `target`'s last line.
    ///
    /// The editor's rows are virtualized, so a host cannot insert a
    /// variable-height card into the list. This reports the same origin the
    /// rows use (list padding + rows above the card + the list's live scroll
    /// offset), letting the host paint an overlay that rides scrolling.
    pub fn comment_anchor(
        &self,
        target: CommentTarget,
        window: &Window,
        cx: &App,
    ) -> Option<CommentAnchor> {
        let state = self.state.read(cx);
        if state.wrap_enabled() || !state.comment_gutter() {
            return None;
        }
        let rem = window.rem_size();
        Some(CommentAnchor {
            row_after: target.end_line,
            line_height: state.font().line_height,
            top_padding: rem * 0.5,
            content_x: rem * crate::CONTENT_X_REM,
            scroll_y: self.scroll_handle.0.borrow().base_handle.offset().y,
            viewport_height: self.viewport_height.get(),
        })
    }

    pub(crate) fn cancel_comment(
        &mut self,
        _: &crate::CancelComment,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.comment_drag.set(None);
        self.comment_hover_row.set(None);
        cx.notify();
    }

    pub(crate) fn select_all(
        &mut self,
        _: &SelectAll,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.state.update(cx, |state, _| state.select_all());
        cx.notify();
    }

    pub(crate) fn undo(&mut self, _: &Undo, _window: &mut Window, cx: &mut Context<Self>) {
        self.state.update(cx, |state, cx| state.undo(cx));
        self.clear_motion_caches();
        self.reveal_caret.set(true);
        cx.notify();
    }

    pub(crate) fn redo(&mut self, _: &Redo, _window: &mut Window, cx: &mut Context<Self>) {
        self.state.update(cx, |state, cx| state.redo(cx));
        self.clear_motion_caches();
        self.reveal_caret.set(true);
        cx.notify();
    }

    /// Cut: copy the selection to the clipboard, then delete it. No-op with
    /// an empty selection (no clipboard churn, no undo entry).
    pub(crate) fn cut(&mut self, _: &Cut, _window: &mut Window, cx: &mut Context<Self>) {
        let edited = self.state.update(cx, |state, cx| {
            let text = state.selected_text();
            if text.is_empty() {
                return false;
            }
            cx.write_to_clipboard(ClipboardItem::new_string(text));
            let range = state.selection().range.clone();
            state.replace_range(range, "", cx);
            true
        });
        if edited {
            self.clear_motion_caches();
            self.reveal_caret.set(true);
            cx.notify();
        }
    }

    /// Paste: replace the selection (or insert at the caret) with clipboard
    /// text. No-op with an empty clipboard.
    pub(crate) fn paste(&mut self, _: &Paste, _window: &mut Window, cx: &mut Context<Self>) {
        let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) else {
            return;
        };
        if text.is_empty() {
            return;
        }
        self.state.update(cx, |state, cx| {
            let range = state.selection().range.clone();
            state.replace_range(range, &text, cx);
        });
        self.clear_motion_caches();
        self.reveal_caret.set(true);
        cx.notify();
    }

    pub(crate) fn clear_motion_caches(&mut self) {
        self.key_anchor.set(None);
        self.goal_col.set(None);
    }
}

/// Geometry for a host-rendered card attached below a comment range.
///
/// All values are in the editor's coordinate space, so a host that wraps the
/// editor 1:1 can position the card with
/// `top_padding + row_after * line_height + scroll_y`.
#[derive(Clone, Copy, Debug)]
pub struct CommentAnchor {
    /// 1-based end line of the range: rows `0..row_after` sit above the card.
    pub row_after: u32,
    pub line_height: Pixels,
    /// The list's top padding.
    pub top_padding: Pixels,
    /// Where row text starts horizontally (gutter + row padding).
    pub content_x: Pixels,
    /// The list's live vertical scroll offset (negative once scrolled).
    pub scroll_y: Pixels,
    pub viewport_height: Pixels,
}

/// Shared, cheaply-clonable handles a rendered row's mouse closures need to
/// read/update selection state. Lives on `EditorView`; cloned once per row.
#[derive(Clone)]
pub(crate) struct RowInteraction {
    pub(crate) state: Entity<EditorState>,
    pub(crate) char_width: Rc<RefCell<Option<(FontConfig, Pixels)>>>,
    pub(crate) selecting: Rc<Cell<bool>>,
    pub(crate) drag_anchor: Rc<Cell<Option<usize>>>,
    pub(crate) last_head: Rc<Cell<Option<usize>>>,
    /// The list's own scroll handle's shared `base_handle` — same handle
    /// horizontal scroll rides on (see `Render`'s comment) — read live in
    /// mouse handlers so hit-testing stays correct while scrolled
    /// (render-time values would go stale between repaints during a scroll
    /// + drag combination).
    pub(crate) h_handle: ScrollHandle,
    pub(crate) comment_hover_row: Rc<Cell<Option<u32>>>,
    pub(crate) comment_drag: Rc<Cell<Option<u32>>>,
    /// Cleared on click (a click is caret placement, not keyboard extension).
    pub(crate) key_anchor: Rc<Cell<Option<usize>>>,
    pub(crate) goal_col: Rc<Cell<Option<u32>>>,
}
