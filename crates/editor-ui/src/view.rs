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
use crate::{Copy, SelectAll};

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
        }
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
}
