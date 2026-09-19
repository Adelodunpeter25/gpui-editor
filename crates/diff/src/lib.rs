//! `diff`: line-level diff computation between two texts.
//!
//! No GPUI dependency — pure data in, pure data out (`DiffResult`), same
//! headless-reusable pattern as `edit-buffer`/`syntax`. Rendering a
//! `DiffResult` (e.g. a side-by-side or unified diff view) is a separate,
//! GPUI-aware concern left to `editor-ui` (see `diff.md`).

mod types;

pub use types::{diff_lines, DiffLine, DiffLineKind, DiffResult};
