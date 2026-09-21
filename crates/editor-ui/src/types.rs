//! Plain vocabulary types for `editor-ui`: edit mode, selection, search
//! state, indent options, font config, and the per-row highlight span the
//! render path reads.
//!
//! Definitions only — behavior lives elsewhere: the model in `state.rs`, the
//! view handle in `view.rs`, rendering in `lib.rs`. Kept small and
//! dependency-light on purpose: `diff_view.rs` and `wrap.rs` both need
//! `FontConfig` without pulling in the editor model.

use gpui::{px, Pixels, SharedString};
use std::ops::Range;
use syntax::Capture;

/// Edit capability + diff-view flag. Readonly disables undo history + input
/// handling. Only `ReadOnly` is implemented today — `ReadOnlyDiff`,
/// `Editable`, and `EditableDiff` are reserved variants for the edit (M2)
/// and diff-viewer (see `diff.md`) milestones, left unimplemented for now.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Mode {
    #[default]
    ReadOnly,
    ReadOnlyDiff,
    Editable,
    EditableDiff,
}

impl Mode {
    pub fn is_readonly(self) -> bool {
        matches!(self, Mode::ReadOnly | Mode::ReadOnlyDiff)
    }

    pub fn is_diff(self) -> bool {
        matches!(self, Mode::ReadOnlyDiff | Mode::EditableDiff)
    }
}

/// Single selection range in char offsets.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Selection {
    pub range: Range<usize>,
}

impl Selection {
    pub fn is_empty(&self) -> bool {
        self.range.start == self.range.end
    }
}

/// Search state: literal matches in char offsets.
#[derive(Debug, Clone, Default)]
pub struct SearchState {
    pub query: String,
    pub matches: Vec<Range<usize>>,
    pub current: Option<usize>,
    pub case_sensitive: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IndentOptions {
    pub tab_width: u32,
    pub hard_tabs: bool,
}

impl Default for IndentOptions {
    fn default() -> Self {
        Self {
            tab_width: 4,
            hard_tabs: false,
        }
    }
}

/// The font every row renders, is measured with, and is wrapped against —
/// one place instead of the string literal `"JetBrains Mono"` previously
/// duplicated across `render_line`, `measure_char_width`, and
/// `WrapCache::rebuild`. Those three disagreeing silently (paint vs. mouse
/// hit-testing vs. wrap points) was the actual bug class this fixes, not
/// just "it was hardcoded" — a host app can now override it in one place
/// and every consumer picks it up.
#[derive(Debug, Clone, PartialEq)]
pub struct FontConfig {
    pub family: SharedString,
    pub size: Pixels,
    /// Row height. Kept as an explicit field rather than derived from
    /// `size` by a ratio — virtualized rendering (`uniform_list`) needs an
    /// exact fixed row height, and letting a host app pick its own
    /// family/size without also being able to tune line height would just
    /// move the "these have to agree" problem instead of fixing it.
    pub line_height: Pixels,
}

impl Default for FontConfig {
    fn default() -> Self {
        Self {
            family: SharedString::from("JetBrains Mono"),
            size: px(14.0),
            line_height: px(22.0),
        }
    }
}

/// One highlight span clipped to a single row, as stored in
/// `EditorState::row_spans` and read per visible row by `lib.rs`'s render
/// path (and `resolve_color`).
#[derive(Debug, Clone)]
pub(crate) struct StyledSpan {
    /// Char range *within the row*.
    pub(crate) range: Range<usize>,
    pub(crate) capture: Capture,
    /// Theme's actual foreground color for this scope, from `lumis`.
    pub(crate) color: Option<(u8, u8, u8)>,
}
