//! `EditorView` — the retained view handle (the host app's `Entity` seam):
//! wraps `Entity<EditorState>` plus the scroll/focus/caches that `render` and
//! its mouse handlers share frame to frame. The `Render` impl and paint
//! helpers live in `lib.rs` (shared with `diff_view.rs`); the model this
//! points at lives in `state.rs`.

use gpui::{
    px, ClipboardItem, Context, Entity, FocusHandle, Pixels, ScrollHandle, UniformListScrollHandle,
    Window,
};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

use crate::state::EditorState;
use crate::types::FontConfig;
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
}
