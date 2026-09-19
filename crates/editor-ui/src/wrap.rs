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
    /// Per buffer row: leading whitespace char count that continuation
    /// sub-lines (not the first) should be left-padded by, so a wrapped
    /// line visually aligns under its own indent instead of restarting at
    /// column 0. Capped so it never eats the whole wrap width — see
    /// `rebuild`.
    row_indent_chars: Vec<u32>,
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
            row_indent_chars: Vec::new(),
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

    /// Rebuild for the given wrap width (`None` disables wrapping) and the
    /// monospace glyph width `char_width` was already measured with (same
    /// value `EditorView`'s render path uses for mouse hit-testing — passed
    /// in rather than re-measured here so indent padding at wrap time and
    /// indent padding at render time can never drift apart). Uses gpui's
    /// own `LineWrapper` (cached per-char glyph widths internally), so this
    /// is O(total buffer chars) once, not per row measured from scratch —
    /// same cost class as `EditorState::rebuild_row_spans`.
    pub(crate) fn rebuild(
        state: &EditorState,
        wrap_width: Option<Pixels>,
        font: &FontConfig,
        char_width: Pixels,
        window: &mut Window,
    ) -> Self {
        let rows = state.line_count();
        let mut row_breaks: Vec<Vec<usize>> = Vec::with_capacity(rows as usize);
        let mut row_indent_chars: Vec<u32> = Vec::with_capacity(rows as usize);
        let mut visual_index: Vec<(u32, u32)> = Vec::new();

        match wrap_width {
            None => {
                for row in 0..rows {
                    row_breaks.push(Vec::new());
                    row_indent_chars.push(0);
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
                        row_indent_chars.push(0);
                        visual_index.push((row, 0));
                        continue;
                    }

                    // Wrap only the text after the leading indent, at a
                    // narrower width that leaves room for that indent to be
                    // re-applied (as padding, not literal chars) on every
                    // continuation sub-line. This is what makes a wrapped
                    // line visually align under its own indent instead of
                    // restarting at column 0 — without it, wrap reads as
                    // broken on deeply-nested code (long generic bounds,
                    // method chains) even though it's "working."
                    let indent_char_count =
                        text.chars().take_while(|c| *c == ' ' || *c == '\t').count() as u32;
                    // Cap so indent can never consume the whole wrap width;
                    // always leave room for at least one content char.
                    let max_indent = ((width / char_width).floor().max(0.) as u32)
                        .saturating_sub(1);
                    let indent_chars = indent_char_count.min(max_indent);
                    let indent_px = char_width * (indent_chars as f32);
                    let content_width = (width - indent_px).max(char_width);

                    let indent_byte_len: usize = text
                        .chars()
                        .take(indent_chars as usize)
                        .map(|c| c.len_utf8())
                        .sum();
                    let rest = &text[indent_byte_len..];
                    let fragments = [LineFragment::text(rest)];
                    let breaks: Vec<usize> = wrapper
                        .wrap_line(&fragments, content_width)
                        .map(|boundary| {
                            let ix = boundary.ix.min(rest.len());
                            indent_chars as usize + rest[..ix].chars().count()
                        })
                        .collect();
                    let sub_count = breaks.len() as u32 + 1;
                    for sub in 0..sub_count {
                        visual_index.push((row, sub));
                    }
                    row_breaks.push(breaks);
                    row_indent_chars.push(indent_chars);
                }
            }
        }

        Self {
            wrap_width,
            buffer_version: state.version(),
            font: font.clone(),
            row_breaks,
            row_indent_chars,
            visual_index,
        }
    }

    /// Leading-whitespace char count continuation sub-lines of `row` should
    /// be left-padded by. Always `0` for a row's first sub-line (it already
    /// renders the real indent as ordinary leading text).
    pub(crate) fn indent_chars(&self, row: u32) -> u32 {
        self.row_indent_chars.get(row as usize).copied().unwrap_or(0)
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
