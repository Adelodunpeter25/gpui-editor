//! Platform text input for `EditorView`: every typed character, dead-key
//! accent, press-and-hold variant, emoji-picker insertion, and CJK
//! composition arrives through gpui's `EntityInputHandler` rather than raw
//! key-down events, so the OS input method sees the editor as a real text
//! field. `render.rs` registers the handler each frame the editor is
//! editable; control keys (Enter, Backspace, arrows…) stay in `input.rs`.
//!
//! The platform addresses text in UTF-16 code units; the buffer in chars.
//! Every range crossing this boundary is converted here.

use gpui::*;
use std::ops::Range;

use crate::measure::measure_char_width;
use crate::view::EditorView;

/// UTF-16 code-unit offset within `text` -> char offset within `text`.
fn utf16_to_char_in(text: &str, utf16: usize) -> usize {
    let mut units = 0;
    for (chars, ch) in text.chars().enumerate() {
        if units >= utf16 {
            return chars;
        }
        units += ch.len_utf16();
    }
    text.chars().count()
}

impl EditorView {
    fn range_from_utf16(&self, range: &Range<usize>, cx: &App) -> Range<usize> {
        let state = self.state.read(cx);
        let start = state.utf16_to_offset(range.start);
        let end = state.utf16_to_offset(range.end).max(start);
        start..end
    }

    fn range_to_utf16(&self, range: &Range<usize>, cx: &App) -> Range<usize> {
        let state = self.state.read(cx);
        state.offset_to_utf16(range.start)..state.offset_to_utf16(range.end)
    }

    /// The range an input-method edit replaces when it names none: the
    /// composition in progress, else the selection.
    fn input_target(&self, range: Option<Range<usize>>, cx: &App) -> Range<usize> {
        match range {
            Some(range) => self.range_from_utf16(&range, cx),
            None => {
                let state = self.state.read(cx);
                state
                    .marked_range()
                    .unwrap_or_else(|| state.selection().range.clone())
            }
        }
    }
}

impl EntityInputHandler for EditorView {
    fn text_for_range(
        &mut self,
        range_utf16: Range<usize>,
        adjusted_range: &mut Option<Range<usize>>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<String> {
        let range = self.range_from_utf16(&range_utf16, cx);
        *adjusted_range = Some(self.range_to_utf16(&range, cx));
        Some(self.state.read(cx).text_in_range(range))
    }

    fn selected_text_range(
        &mut self,
        _ignore_disabled_input: bool,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        let range = self.state.read(cx).selection().range.clone();
        Some(UTF16Selection {
            reversed: !range.is_empty() && self.key_anchor.get() == Some(range.end),
            range: self.range_to_utf16(&range, cx),
        })
    }

    fn marked_text_range(
        &self,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<Range<usize>> {
        let marked = self.state.read(cx).marked_range()?;
        Some(self.range_to_utf16(&marked, cx))
    }

    fn unmark_text(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        self.state.update(cx, |state, cx| state.unmark_text(cx));
    }

    fn replace_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        text: &str,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.state.read(cx).is_readonly() {
            return;
        }
        let range = self.input_target(range_utf16, cx);
        self.clear_motion_caches();
        self.state
            .update(cx, |state, cx| state.type_text(range, text, cx));
        self.reveal_caret.set(true);
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        new_selected_range_utf16: Option<Range<usize>>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.state.read(cx).is_readonly() {
            return;
        }
        let range = self.input_target(range_utf16, cx);
        // The new selection is relative to `new_text`, also in UTF-16.
        let selected = new_selected_range_utf16
            .map(|r| utf16_to_char_in(new_text, r.start)..utf16_to_char_in(new_text, r.end));
        self.clear_motion_caches();
        self.state.update(cx, |state, cx| {
            state.set_marked_text(range, new_text, selected, cx)
        });
        self.reveal_caret.set(true);
    }

    /// Screen rectangle of a text range — where the OS anchors the IME
    /// candidate window and emoji picker.
    fn bounds_for_range(
        &mut self,
        range_utf16: Range<usize>,
        element_bounds: Bounds<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        let range = self.range_from_utf16(&range_utf16, cx);
        let font = self.state.read(cx).font().clone();
        let char_width = measure_char_width(&self.char_width, &font, window);
        let (row, x) = self.offset_position(range.start, char_width, cx);
        let (end_row, end_x) = self.offset_position(range.end, char_width, cx);
        let width = if end_row == row && end_x > x {
            end_x - x
        } else {
            char_width
        };
        let origin = self.text_origin(element_bounds.origin, window.rem_size());
        Some(Bounds::new(
            point(origin.x + x, origin.y + font.line_height * row as f32),
            size(width, font.line_height),
        ))
    }

    fn character_index_for_point(
        &mut self,
        point: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<usize> {
        let font = self.state.read(cx).font().clone();
        let char_width = measure_char_width(&self.char_width, &font, window);
        let offset = self.offset_for_point(point, char_width, window.rem_size(), cx)?;
        Some(self.state.read(cx).offset_to_utf16(offset))
    }

    fn accepts_text_input(&self, _window: &mut Window, cx: &mut Context<Self>) -> bool {
        !self.state.read(cx).is_readonly()
    }
}
