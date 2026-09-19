//! Word wrap: buffer-row <-> visual-row mapping.
//!
//! Render-time concern (needs a `Window`/text system to measure glyph
//! widths), so it lives on `EditorView`, not the GPUI-free `EditorState`
//! (`types.rs`). Kept in its own module rather than folded into `types.rs`
//! or `lib.rs` since it's a distinct, self-contained algorithm — one more
//! thing to keep track of if merged into an already-busy file.

use gpui::*;
use std::ops::Range;

use crate::types::{EditorState, FontConfig};

/// Maps buffer rows to visual (wrapped) rows. Rebuilt whenever the buffer
/// version, wrap width, wrap-enabled flag, or font changes — never per
/// frame. `wrap_width: None` means wrap is off; every buffer row is exactly
/// one visual row (today's behavior), computed without touching the text
/// system.
pub(crate) struct WrapCache {
    wrap_width: Option<Pixels>,
    buffer_version: u64,
    font: FontConfig,
    /// Per buffer row: row-local char offsets where a visual break occurs
    /// (end-exclusive boundaries of every sub-line except the last). Empty
    /// means the row is a single visual line.
    row_breaks: Vec<Vec<usize>>,
    /// Flattened visual-row index -> (buffer_row, sub_line_index).
    visual_index: Vec<(u32, u32)>,
}

impl Default for WrapCache {
    fn default() -> Self {
        // `buffer_version: u64::MAX` guarantees `is_stale` is true on the
        // first real check even though a fresh buffer's version is 0.
        Self {
            wrap_width: None,
            buffer_version: u64::MAX,
            font: FontConfig::default(),
            row_breaks: Vec::new(),
            visual_index: Vec::new(),
        }
    }
}

impl WrapCache {
    pub(crate) fn is_stale(
        &self,
        wrap_width: Option<Pixels>,
        buffer_version: u64,
        font: &FontConfig,
    ) -> bool {
        self.wrap_width != wrap_width || self.buffer_version != buffer_version || &self.font != font
    }

    /// Rebuild for the given wrap width (`None` disables wrapping). Uses
    /// gpui's own `LineWrapper` (cached per-char glyph widths internally),
    /// so this is O(total buffer chars) once, not per row measured from
    /// scratch — same cost class as `EditorState::rebuild_row_spans`.
    pub(crate) fn rebuild(
        state: &EditorState,
        wrap_width: Option<Pixels>,
        font: &FontConfig,
        window: &mut Window,
    ) -> Self {
        let rows = state.line_count();
        let mut row_breaks: Vec<Vec<usize>> = Vec::with_capacity(rows as usize);
        let mut visual_index: Vec<(u32, u32)> = Vec::new();

        match wrap_width {
            None => {
                for row in 0..rows {
                    row_breaks.push(Vec::new());
                    visual_index.push((row, 0));
                }
            }
            Some(width) => {
                let mut wrapper = window
                    .text_system()
                    .line_wrapper(gpui::font(font.family.clone()), font.size);
                for row in 0..rows {
                    let text = state.line_text(row);
                    if text.is_empty() {
                        row_breaks.push(Vec::new());
                        visual_index.push((row, 0));
                        continue;
                    }
                    let fragments = [LineFragment::text(&text)];
                    let breaks: Vec<usize> = wrapper
                        .wrap_line(&fragments, width)
                        .map(|boundary| {
                            let ix = boundary.ix.min(text.len());
                            text[..ix].chars().count()
                        })
                        .collect();
                    let sub_count = breaks.len() as u32 + 1;
                    for sub in 0..sub_count {
                        visual_index.push((row, sub));
                    }
                    row_breaks.push(breaks);
                }
            }
        }

        Self {
            wrap_width,
            buffer_version: state.version(),
            font: font.clone(),
            row_breaks,
            visual_index,
        }
    }

    pub(crate) fn visual_row_count(&self) -> usize {
        self.visual_index.len()
    }

    /// Visual row index -> (buffer row, sub-line index within that row).
    pub(crate) fn resolve(&self, visual_ix: usize) -> (u32, u32) {
        self.visual_index[visual_ix]
    }

    /// Row-local char range covered by a given sub-line. `row_len` is the
    /// buffer row's total char count (caller already has it from the row's
    /// text), used as the open end for the last sub-line.
    pub(crate) fn sub_range(&self, row: u32, sub: u32, row_len: usize) -> Range<usize> {
        let breaks = &self.row_breaks[row as usize];
        let start = if sub == 0 {
            0
        } else {
            breaks[(sub - 1) as usize]
        };
        let end = breaks.get(sub as usize).copied().unwrap_or(row_len);
        start..end
    }
}

/// Clip `range` to `sub_range`, rebasing the result to be relative to
/// `sub_range.start`. Shared by syntax runs and selection when slicing a
/// buffer row's row-local ranges down to one wrapped visual sub-line.
pub(crate) fn clip_to_subrange(range: &Range<usize>, sub_range: &Range<usize>) -> Option<Range<usize>> {
    let lo = range.start.max(sub_range.start);
    let hi = range.end.min(sub_range.end);
    if lo < hi {
        Some(lo - sub_range.start..hi - sub_range.start)
    } else {
        None
    }
}
