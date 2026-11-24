//! Floating find bar overlay (`Cmd+F` / `Ctrl+F`) for `EditorView`.
//!
//! Owns its own `FocusHandle` and `EntityInputHandler` so typing a search
//! query never routes into the editor buffer.

use gpui::prelude::FluentBuilder;
use gpui::*;
use std::ops::Range;

use crate::cursor::CursorBlink;
use crate::state::EditorState;
use crate::view::EditorView;
use crate::{
    CloseSearch, Copy, Cut, Find, FindNext, FindPrevious, Paste, SelectAll,
};

pub(crate) struct SearchBar {
    state: Entity<EditorState>,
    editor_view: WeakEntity<EditorView>,
    focus_handle: FocusHandle,
    query: String,
    /// Selection anchor in `query` as a char offset.
    anchor: usize,
    /// Selection head (caret) in `query` as a char offset.
    head: usize,
    blink: CursorBlink,
}

impl SearchBar {
    pub(crate) fn new(
        state: Entity<EditorState>,
        editor_view: WeakEntity<EditorView>,
        cx: &mut Context<Self>,
    ) -> Self {
        Self {
            state,
            editor_view,
            focus_handle: cx.focus_handle(),
            query: String::new(),
            anchor: 0,
            head: 0,
            blink: CursorBlink::default(),
        }
    }

    pub(crate) fn reset_blink(&mut self, cx: &mut Context<Self>) {
        self.blink.reset(cx, |bar, visible, _| {
            bar.blink.set_visible(visible);
        });
    }

    pub(crate) fn stop_blink(&mut self) {
        self.blink.stop();
    }

    pub(crate) fn focus_handle(&self) -> &FocusHandle {
        &self.focus_handle
    }

    pub(crate) fn query(&self) -> &str {
        &self.query
    }

    pub(crate) fn set_query(&mut self, query: String, cx: &mut Context<Self>) {
        let len = query.chars().count();
        self.query = query;
        self.anchor = 0;
        self.head = len;
        let q = self.query.clone();
        self.state.update(cx, |state, cx| {
            state.update_search(&q, false, cx);
            state.select_current_match();
        });
        cx.notify();
    }

    fn selection_range(&self) -> Range<usize> {
        self.anchor.min(self.head)..self.anchor.max(self.head)
    }

    fn char_to_byte(&self, char_ix: usize) -> usize {
        self.query
            .char_indices()
            .nth(char_ix)
            .map_or(self.query.len(), |(b, _)| b)
    }

    fn utf16_to_char(&self, utf16: usize) -> usize {
        let mut units = 0;
        for (chars, ch) in self.query.chars().enumerate() {
            if units >= utf16 {
                return chars;
            }
            units += ch.len_utf16();
        }
        self.query.chars().count()
    }

    fn char_to_utf16(&self, char_ix: usize) -> usize {
        self.query.chars().take(char_ix).map(char::len_utf16).sum()
    }

    fn run_search(&mut self, cx: &mut Context<Self>) {
        let query = self.query.clone();
        self.state.update(cx, |state, cx| {
            state.update_search(&query, false, cx);
            if query.is_empty() {
                let pos = state.selection().range.start;
                state.set_selection(pos..pos);
            } else {
                state.select_current_match();
            }
        });
        let _ = self.editor_view.update(cx, |view, cx| {
            view.scroll_to_caret(cx);
        });
        cx.notify();
    }

    fn replace_char_range(&mut self, range: Range<usize>, text: &str, cx: &mut Context<Self>) {
        let len = self.query.chars().count();
        let start = range.start.min(len);
        let end = range.end.min(len).max(start);
        let start_b = self.char_to_byte(start);
        let end_b = self.char_to_byte(end);
        self.query.replace_range(start_b..end_b, text);
        let caret = start + text.chars().count();
        self.anchor = caret;
        self.head = caret;
        self.reset_blink(cx);
        self.run_search(cx);
    }

    fn key_down(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if !self.focus_handle.is_focused(window) {
            return;
        }
        let key = event.keystroke.key.as_str();
        let shift = event.keystroke.modifiers.shift;
        let len = self.query.chars().count();
        let sel = self.selection_range();
        match key {
            "backspace" => {
                cx.stop_propagation();
                if !sel.is_empty() {
                    self.replace_char_range(sel, "", cx);
                } else if self.head > 0 {
                    let c = self.head;
                    self.replace_char_range(c - 1..c, "", cx);
                }
            }
            "delete" => {
                cx.stop_propagation();
                if !sel.is_empty() {
                    self.replace_char_range(sel, "", cx);
                } else if self.head < len {
                    let c = self.head;
                    self.replace_char_range(c..c + 1, "", cx);
                }
            }
            "left" => {
                cx.stop_propagation();
                if shift {
                    self.head = self.head.saturating_sub(1);
                } else if !sel.is_empty() {
                    self.anchor = sel.start;
                    self.head = sel.start;
                } else {
                    self.head = self.head.saturating_sub(1);
                    self.anchor = self.head;
                }
                cx.notify();
            }
            "right" => {
                cx.stop_propagation();
                if shift {
                    self.head = (self.head + 1).min(len);
                } else if !sel.is_empty() {
                    self.anchor = sel.end;
                    self.head = sel.end;
                } else {
                    self.head = (self.head + 1).min(len);
                    self.anchor = self.head;
                }
                cx.notify();
            }
            "home" => {
                cx.stop_propagation();
                self.head = 0;
                if !shift {
                    self.anchor = 0;
                }
                cx.notify();
            }
            "end" => {
                cx.stop_propagation();
                self.head = len;
                if !shift {
                    self.anchor = len;
                }
                cx.notify();
            }
            _ => {}
        }
    }

    fn select_all(&mut self, _: &SelectAll, _window: &mut Window, cx: &mut Context<Self>) {
        cx.stop_propagation();
        self.anchor = 0;
        self.head = self.query.chars().count();
        cx.notify();
    }

    fn copy(&mut self, _: &Copy, _window: &mut Window, cx: &mut Context<Self>) {
        cx.stop_propagation();
        let sel = self.selection_range();
        if !sel.is_empty() {
            let sb = self.char_to_byte(sel.start);
            let eb = self.char_to_byte(sel.end);
            cx.write_to_clipboard(ClipboardItem::new_string(self.query[sb..eb].to_string()));
        }
    }

    fn cut(&mut self, _: &Cut, _window: &mut Window, cx: &mut Context<Self>) {
        cx.stop_propagation();
        let sel = self.selection_range();
        if !sel.is_empty() {
            let sb = self.char_to_byte(sel.start);
            let eb = self.char_to_byte(sel.end);
            cx.write_to_clipboard(ClipboardItem::new_string(self.query[sb..eb].to_string()));
            self.replace_char_range(sel, "", cx);
        }
    }

    fn paste(&mut self, _: &Paste, _window: &mut Window, cx: &mut Context<Self>) {
        cx.stop_propagation();
        let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) else {
            return;
        };
        let single_line = text.replace(['\r', '\n'], " ");
        if !single_line.is_empty() {
            let sel = self.selection_range();
            self.replace_char_range(sel, &single_line, cx);
        }
    }

    fn find_next(&mut self, _: &FindNext, window: &mut Window, cx: &mut Context<Self>) {
        let _ = self.editor_view.update(cx, |view, cx| {
            view.find_next(&FindNext, window, cx);
        });
    }

    fn find_prev(&mut self, _: &FindPrevious, window: &mut Window, cx: &mut Context<Self>) {
        let _ = self.editor_view.update(cx, |view, cx| {
            view.find_prev(&FindPrevious, window, cx);
        });
    }

    fn close(&mut self, _: &CloseSearch, window: &mut Window, cx: &mut Context<Self>) {
        self.stop_blink();
        let _ = self.editor_view.update(cx, |view, cx| {
            view.close_search(&CloseSearch, window, cx);
        });
    }

    fn refocus(&mut self, _: &Find, window: &mut Window, cx: &mut Context<Self>) {
        self.anchor = 0;
        self.head = self.query.chars().count();
        self.reset_blink(cx);
        window.focus(&self.focus_handle, cx);
        cx.notify();
    }
}

impl EntityInputHandler for SearchBar {
    fn text_for_range(
        &mut self,
        range_utf16: Range<usize>,
        adjusted_range: &mut Option<Range<usize>>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<String> {
        let start = self.utf16_to_char(range_utf16.start);
        let end = self.utf16_to_char(range_utf16.end).max(start);
        *adjusted_range = Some(self.char_to_utf16(start)..self.char_to_utf16(end));
        let sb = self.char_to_byte(start);
        let eb = self.char_to_byte(end);
        Some(self.query[sb..eb].to_string())
    }

    fn selected_text_range(
        &mut self,
        _ignore_disabled_input: bool,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        let sel = self.selection_range();
        Some(UTF16Selection {
            reversed: self.head < self.anchor,
            range: self.char_to_utf16(sel.start)..self.char_to_utf16(sel.end),
        })
    }

    fn marked_text_range(
        &self,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Range<usize>> {
        None
    }

    fn unmark_text(&mut self, _window: &mut Window, _cx: &mut Context<Self>) {}

    fn replace_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        text: &str,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let range = match range_utf16 {
            Some(r) => self.utf16_to_char(r.start)..self.utf16_to_char(r.end),
            None => self.selection_range(),
        };
        let clean = text.replace(['\r', '\n'], "");
        self.replace_char_range(range, &clean, cx);
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        _new_selected_range_utf16: Option<Range<usize>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.replace_text_in_range(range_utf16, new_text, window, cx);
    }

    fn bounds_for_range(
        &mut self,
        _range_utf16: Range<usize>,
        element_bounds: Bounds<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        Some(element_bounds)
    }

    fn character_index_for_point(
        &mut self,
        _point: Point<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<usize> {
        Some(self.char_to_utf16(self.head))
    }

    fn accepts_text_input(&self, _window: &mut Window, _cx: &mut Context<Self>) -> bool {
        true
    }
}

impl Render for SearchBar {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let search = self.state.read(cx).search();
        let count_label = if self.query.is_empty() {
            String::new()
        } else if search.matches.is_empty() {
            "No results".to_string()
        } else {
            let cur = search.current.map(|i| i + 1).unwrap_or(0);
            format!("{cur} / {}", search.matches.len())
        };

        let focused = self.focus_handle.is_focused(window);
        let input_focus = self.focus_handle.clone();
        let input_entity = cx.entity();

        let sel = self.selection_range();
        let before_sel: String = self.query.chars().take(sel.start).collect();
        let in_sel: String = self
            .query
            .chars()
            .skip(sel.start)
            .take(sel.end - sel.start)
            .collect();
        let after_sel: String = self.query.chars().skip(sel.end).collect();

        div()
            .id("editor-search-bar")
            .absolute()
            .top_2()
            .right_6()
            .flex()
            .flex_row()
            .items_center()
            .gap_1()
            .px_1p5()
            .py_1()
            .rounded_md()
            .bg(rgb(0x141416))
            .border_1()
            .border_color(rgba(0xffffff1f))
            .shadow_md()
            .text_xs()
            .text_color(rgb(0xe4e4e7))
            .track_focus(&self.focus_handle)
            .key_context("EditorSearch")
            .on_action(cx.listener(Self::find_next))
            .on_action(cx.listener(Self::find_prev))
            .on_action(cx.listener(Self::close))
            .on_action(cx.listener(Self::refocus))
            .on_action(cx.listener(Self::select_all))
            .on_action(cx.listener(Self::copy))
            .on_action(cx.listener(Self::cut))
            .on_action(cx.listener(Self::paste))
            .on_key_down(cx.listener(Self::key_down))
            .on_mouse_down(MouseButton::Left, |_, _, cx| {
                cx.stop_propagation();
            })
            .child(
                div()
                    .id("search-input-box")
                    .relative()
                    .w(px(240.0))
                    .h(px(24.0))
                    .px_2()
                    .rounded_sm()
                    .bg(rgb(0x0c0c0e))
                    .border_1()
                    .border_color(if focused {
                        rgba(0xffffff40)
                    } else {
                        rgba(0xffffff14)
                    })
                    .flex()
                    .flex_row()
                    .items_center()
                    .justify_between()
                    .gap_1p5()
                    .child(
                        canvas(
                            |_, _, _| {},
                            move |bounds, _, window, cx| {
                                window.handle_input(
                                    &input_focus,
                                    ElementInputHandler::new(bounds, input_entity),
                                    cx,
                                );
                            },
                        )
                        .absolute()
                        .size_full(),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_row()
                            .items_center()
                            .overflow_x_hidden()
                            .when(self.query.is_empty(), |el| {
                                el.when(focused && self.blink.is_visible(), |el| {
                                    el.child(div().w(px(1.5)).h(px(14.0)).bg(rgb(0xe4e4e7)))
                                })
                                .child(
                                    div()
                                        .text_color(rgb(0x71717a))
                                        .child("Find in file…"),
                                )
                            })
                            .when(!self.query.is_empty(), |el| {
                                el.child(before_sel)
                                    .when(!sel.is_empty(), |el| {
                                        el.child(div().bg(rgba(0x3b82f680)).child(in_sel))
                                    })
                                    .when(sel.is_empty() && focused && self.blink.is_visible(), |el| {
                                        el.child(div().w(px(1.5)).h(px(14.0)).bg(rgb(0xe4e4e7)))
                                    })
                                    .child(after_sel)
                            }),
                    )
                    .when(!count_label.is_empty(), |el| {
                        el.child(
                            div()
                                .flex_shrink_0()
                                .whitespace_nowrap()
                                .text_color(if search.matches.is_empty() {
                                    rgb(0xf87171)
                                } else {
                                    rgb(0x71717a)
                                })
                                .child(count_label),
                        )
                    }),
            )
            .child(div().w(px(1.0)).h(px(16.0)).bg(rgba(0xffffff1a)))
            .child(
                div()
                    .id("search-prev")
                    .w(px(24.0))
                    .h(px(24.0))
                    .rounded_sm()
                    .flex()
                    .items_center()
                    .justify_center()
                    .cursor_pointer()
                    .text_color(rgb(0xa1a1aa))
                    .hover(|s| s.bg(rgba(0xffffff14)).text_color(rgb(0xffffff)))
                    .child("↑")
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.find_prev(&FindPrevious, window, cx);
                    })),
            )
            .child(
                div()
                    .id("search-next")
                    .w(px(24.0))
                    .h(px(24.0))
                    .rounded_sm()
                    .flex()
                    .items_center()
                    .justify_center()
                    .cursor_pointer()
                    .text_color(rgb(0xa1a1aa))
                    .hover(|s| s.bg(rgba(0xffffff14)).text_color(rgb(0xffffff)))
                    .child("↓")
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.find_next(&FindNext, window, cx);
                    })),
            )
            .child(
                div()
                    .id("search-close")
                    .w(px(24.0))
                    .h(px(24.0))
                    .rounded_sm()
                    .flex()
                    .items_center()
                    .justify_center()
                    .cursor_pointer()
                    .text_color(rgb(0xa1a1aa))
                    .hover(|s| s.bg(rgba(0xffffff14)).text_color(rgb(0xffffff)))
                    .child("×")
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.close(&CloseSearch, window, cx);
                    })),
            )
    }
}
