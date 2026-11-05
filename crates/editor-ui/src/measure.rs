//! Measurement helpers shared by the editor's render path, `DiffView`, and
//! the wrap cache: monospace glyph advance (cached per font), pixel -> column
//! hit-testing (tab stops honored), and the display-width math every width
//! scan agrees on.

use gpui::*;
use std::cell::RefCell;
use std::rc::Rc;

use crate::types::FontConfig;

/// hit testing. Cached alongside the `FontConfig` it was measured for, so a
/// runtime font change re-measures instead of reusing a stale width.
pub(crate) fn measure_char_width(
    cache: &Rc<RefCell<Option<(FontConfig, Pixels)>>>,
    font_config: &FontConfig,
    window: &mut Window,
) -> Pixels {
    if let Some((cached_font, width)) = cache.borrow().as_ref() {
        if cached_font == font_config {
            return *width;
        }
    }
    let run = TextRun {
        len: 1,
        font: font(font_config.family.clone()),
        color: Hsla::white(),
        background_color: None,
        underline: None,
        strikethrough: None,
    };
    let width = window
        .text_system()
        .shape_line("M".into(), font_config.size, &[run], None)
        .width();
    *cache.borrow_mut() = Some((font_config.clone(), width));
    width
}

/// Pixel x (relative to the row content's left edge) -> char column. Walks
/// `text`'s chars accumulating pixel width rather than dividing by a flat
/// `char_width`, since a tab doesn't occupy one glyph-width column — it
/// advances to the next `tab_width`-column stop, same as `indent_for_newline`
/// assumes. A single division here (the old approach) put every column after
/// the first tab in a line off by however wide that tab rendered.
pub(crate) fn column_for_x(local_x: Pixels, char_width: Pixels, text: &str, tab_width: u32) -> u32 {
    let target = local_x.max(px(0.));
    let mut x = px(0.);
    let mut col: u32 = 0;
    for ch in text.chars() {
        let advance = if ch == '\t' {
            let tab_px = f32::from(char_width) * tab_width.max(1) as f32;
            let steps = (f32::from(x) / tab_px).floor() + 1.0;
            px(tab_px * steps - f32::from(x))
        } else {
            char_width
        };
        let mid = x + advance / 2.0;
        if target < mid {
            return col;
        }
        x += advance;
        col += 1;
    }
    col
}


/// Approximate pixel width consumed by the line-number gutter (`w_10()`,
/// 2.5rem) + the row's horizontal padding (`px_3()` x2) before wrapped text
/// content actually starts. Used only to size the word-wrap width; a few
/// pixels of slack here just wraps a character or two earlier/later than
/// pixel-perfect, which is cosmetic (see wrap.md's scope notes).
pub(crate) const WRAP_GUTTER_RESERVE: Pixels = px(64.0);

/// Bucket size for wrap-width rounding: a live window resize fires the
/// measuring `canvas` (and so a potential `WrapCache` rebuild, which
/// re-shapes every row) on nearly every frame. Rounding to a coarse bucket
/// means a rebuild only happens once every ~4 monospace chars of resize
/// instead of every single pixel — the resize-drag equivalent of the
/// existing debounce-on-edit pattern the rest of the codebase uses.
const WRAP_WIDTH_BUCKET: f32 = 32.0;

pub(crate) fn round_wrap_width(width: Pixels) -> Pixels {
    px((f32::from(width) / WRAP_WIDTH_BUCKET).round() * WRAP_WIDTH_BUCKET)
}

/// Row left padding (`px_3()` = 0.75rem) + gutter width (`w_10()` = 2.5rem) =
/// where a row's text content actually starts, in rems. Fixed, non-dynamic
/// layout — computed once per render from `window.rem_size()` rather than
/// measured via a `canvas` per row (previously: one measurement canvas per
/// *visible row*, remeasuring an identical value ~30 times a render for no
/// reason, since every row has the same gutter/padding).
pub(crate) const CONTENT_X_REM: f32 = 3.25;

/// Cap on chars rendered/hit-tested per visual row. Without this, a single
/// pathologically long line (a minified file, a giant JSON blob) with wrap
/// off rebuilds a `chars().collect()` + one-div-per-syntax/selection-segment
/// tree that size on *every* mouse-move during a drag — `.overflow_x_hidden()`
/// only hides the overflow visually after all that work already happened.
/// Clamping what's built also clamps what's selectable in the row, matching
/// how most editors treat absurdly long lines.
pub(crate) const MAX_ROW_RENDER_CHARS: usize = 4000;

/// Pixel width of a char stream with tab stops, skipping line breaks.
/// Capped at `MAX_ROW_RENDER_CHARS` so a width scan describes the same
/// painted prefix the row renderer builds — otherwise dead scroll range
/// would trail every pathologically long line.
pub(crate) fn chars_display_width(
    chars: impl Iterator<Item = char>,
    char_width: Pixels,
    tab_width: u32,
) -> Pixels {
    let cw = f32::from(char_width);
    let tab_px = cw * tab_width.max(1) as f32;
    let mut x = 0f32;
    for ch in chars
        .filter(|c| *c != '\n' && *c != '\r')
        .take(MAX_ROW_RENDER_CHARS)
    {
        if ch == '\t' {
            x = tab_px * ((x / tab_px).floor() + 1.0);
        } else {
            x += cw;
        }
    }
    px(x)
}

pub(crate) fn text_display_width(text: &str, char_width: Pixels, tab_width: u32) -> Pixels {
    chars_display_width(text.chars(), char_width, tab_width)
}

