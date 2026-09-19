//! `edit-buffer`: single-owner text storage for gpui-editor.
//!
//! No GPUI dependency. Text lives once in a `ropey::Rope`.
//! All offsets are **char offsets** (not bytes) to stay O(1)-ish with ropey.
//! Byte conversion happens at the syntax/render boundary.

mod types;

pub use types::{Buffer, BufferSnapshot, Edit, Point};
