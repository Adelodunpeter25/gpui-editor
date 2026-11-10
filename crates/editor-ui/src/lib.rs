//! `editor-ui`: embeddable GPUI code view (raw `gpui 0.2.2`, no component kit).
//!
//! State lives in `Entity<EditorState>`; the view handle in `view.rs`. The
//! `edit-buffer` and `syntax` crates stay GPUI-free so a host app can reuse
//! them headless. Inside this crate: the model in `state.rs`, plain
//! vocabulary types in `types.rs`, word wrap in `wrap.rs`, the diff viewer
//! in `diff_view.rs`, and — split out of what used to be one large `lib.rs`
//! — the `Render` impl (`render.rs`), per-row painting (`row.rs`),
//! scrollbars (`scrollbar.rs`), measurement/hit-testing (`measure.rs`), and
//! edit-mode key routing (`input.rs`) and platform text input / IME
//! (`ime.rs`). This root file is module
//! declarations + re-exports only.

mod diff_view;
mod ime;
mod input;
mod measure;
mod render;
mod row;
mod scrollbar;
mod state;
mod types;
mod view;
mod wrap;

pub use diff::{DiffLine, DiffLineKind, DiffResult};
pub use diff_view::{DiffState, DiffView};
pub use input::{init, CancelComment, Copy, Cut, Paste, Redo, SelectAll, Undo};
pub use scrollbar::scrollbar_thumb_size;
pub use state::EditorState;
pub use types::{
    comment_target_from, gutter_hit_test, CommentTarget, FontConfig, IndentOptions, Mode,
    SearchState, Selection,
};
pub use view::{CommentAnchor, EditorView};

// Re-export for host apps.
pub use syntax::LanguageRegistry as Registry;

// Crate-internal seams: `state.rs`, `view.rs`, and `diff_view.rs` read these
// through the crate root (paths like `crate::CONTENT_X_REM`).
pub(crate) use measure::{
    chars_display_width, measure_char_width, round_wrap_width, text_display_width, CONTENT_X_REM,
};
pub(crate) use row::{color_for, render_spans};
pub(crate) use scrollbar::{render_h_scrollbar, render_scrollbar, scrollbar_thumb_height};
