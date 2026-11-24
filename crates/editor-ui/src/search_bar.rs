//! Floating find bar overlay (`Cmd+F` / `Ctrl+F`) for `EditorView`.
//!
//! Owns its own `FocusHandle` and `EntityInputHandler` so typing a search
//! query never routes into the editor buffer.

use gpui::prelude::FluentBuilder;
use gpui::*;
use std::ops::Range;

use crate::state::EditorState;
use crate::view::EditorView;
use crate::{CloseSearch, Find, FindNext, FindPrevious, Paste, ToggleCaseSensitive};

pub(crate) struct SearchBar {
    state: Entity<EditorState>,
    editor_view: WeakEntity<EditorView>,
    focus_handle: FocusHandle,
    query: String,
    /// Caret position in `query` as a char offset.
    cursor: usize,
    case_sensitive: bool,
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
            cursor: 0,
            case_sensitive: false,
        }
    }

    pub(crate) fn focus_handle(&self) -> &FocusHandle {
        &self.focus_handle
    }

    pub(crate) fn query(&self) -> &str {
        &self.query
    }

    pub(crate) fn set_query(&mut self, query: String, cx: &mut Context<Self>) {
        self.cursor = query.chars().count();
        self.query = query;
        let case_sensitive = self.case_sensitive;
        let q = self.query.clone();
        self.state.update(cx, |state, cx| {
            state.update_search(&q, case_sensitive, cx);
            state.select_current_match();
        });
        cx.notify();
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
        let case_sensitive = self.case_sensitive;
        self.state.update(cx, |state, cx| {
            state.update_search(&query, case_sensitive, cx);
            state.select_current_match();
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
        self.cursor = start + text.chars().count();
        self.run_search(cx);
    }

    fn key_down(&mut self, event: &KeyDownEvent, _window: &mut Window, cx: &mut Context<Self>) {
        if !self.focus_handle.is_focused(_window) {
            return;
        }
        let key = event.keystroke.key.as_str();
        let len = self.query.chars().count();
        match key {
            "backspace" => {
                cx.stop_propagation();
                if self.cursor > 0 {
                    let c = self.cursor;
                    self.replace_char_range(c - 1..c, "", cx);
                }
            }
            "delete" => {
                cx.stop_propagation();
                if self.cursor < len {
                    let c = self.cursor;
                    self.replace_char_range(c..c + 1, "", cx);
                }
            }
            "left" => {
                cx.stop_propagation();
                self.cursor = self.cursor.saturating_sub(1);
                cx.notify();
            }
            "right" => {
                cx.stop_propagation();
                self.cursor = (self.cursor + 1).min(len);
                cx.notify();
            }
            "home" => {
                cx.stop_propagation();
                self.cursor = 0;
                cx.notify();
            }
            "end" => {
                cx.stop_propagation();
                self.cursor = len;
                cx.notify();
            }
            _ => {}
        }
    }

    fn paste(&mut self, _: &Paste, _window: &mut Window, cx: &mut Context<Self>) {
        let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) else {
            return;
        };
        let single_line = text.replace(['\r', '\n'], " ");
        if !single_line.is_empty() {
            let c = self.cursor;
            self.replace_char_range(c..c, &single_line, cx);
        }
    }

    fn toggle_case(&mut self, _: &ToggleCaseSensitive, _window: &mut Window, cx: &mut Context<Self>) {
        self.case_sensitive = !self.case_sensitive;
        self.run_search(cx);
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
        let _ = self.editor_view.update(cx, |view, cx| {
            view.close_search(&CloseSearch, window, cx);
        });
    }

    fn refocus(&mut self, _: &Find, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus_handle, cx);
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
        let u = self.char_to_utf16(self.cursor);
        Some(UTF16Selection {
            reversed: false,
            range: u..u,
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
            None => self.cursor..self.cursor,
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
        Some(self.char_to_utf16(self.cursor))
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
        let case_sensitive = self.case_sensitive;

        let before_cursor: String = self.query.chars().take(self.cursor).collect();
        let after_cursor: String = self.query.chars().skip(self.cursor).collect();

        div()
            .id("editor-search-bar")
            .absolute()
            .top_2()
            .right_4()
            .flex()
            .flex_row()
            .items_center()
            .gap_1p5()
            .px_2p5()
            .py_1p5()
            .rounded_md()
            .bg(rgb(0x1e1e2e))
            .border_1()
            .border_color(rgb(0x45475a))
            .shadow_md()
            .text_xs()
            .text_color(rgb(0xcdd6f4))
            .track_focus(&self.focus_handle)
            .key_context("EditorSearch")
            .on_action(cx.listener(Self::find_next))
            .on_action(cx.listener(Self::find_prev))
            .on_action(cx.listener(Self::close))
            .on_action(cx.listener(Self::toggle_case))
            .on_action(cx.listener(Self::refocus))
            .on_action(cx.listener(Self::paste))
            .on_key_down(cx.listener(Self::key_down))
            .on_mouse_down(MouseButton::Left, |_, _, cx| {
                cx.stop_propagation();
            })
            .child(
                div()
                    .id("search-input-box")
                    .relative()
                    .w(px(180.0))
                    .h(px(24.0))
                    .px_2()
                    .rounded_sm()
                    .bg(rgb(0x11111b))
                    .border_1()
                    .border_color(if focused {
                        rgb(0x89b4fa)
                    } else {
                        rgb(0x313244)
                    })
                    .flex()
                    .flex_row()
                    .items_center()
                    .overflow_x_hidden()
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
                    .when(self.query.is_empty(), |el| {
                        el.child(
                            div()
                                .text_color(rgb(0x6c7086))
                                .child("Find in file…"),
                        )
                    })
                    .when(!self.query.is_empty(), |el| {
                        el.child(before_cursor)
                            .when(focused, |el| {
                                el.child(div().w(px(1.5)).h(px(14.0)).bg(rgb(0xcdd6f4)))
                            })
                            .child(after_cursor)
                    }),
            )
            .when(!count_label.is_empty(), |el| {
                el.child(
                    div()
                        .px_1()
                        .text_color(if search.matches.is_empty() {
                            rgb(0xf38ba8)
                        } else {
                            rgb(0xa6adc8)
                        })
                        .child(count_label),
                )
            })
            .child(
                div()
                    .id("search-case-toggle")
                    .px_1p5()
                    .py_0p5()
                    .rounded_sm()
                    .cursor_pointer()
                    .bg(if case_sensitive {
                        rgb(0x89b4fa)
                    } else {
                        rgb(0x313244)
                    })
                    .text_color(if case_sensitive {
                        rgb(0x1e1e2e)
                    } else {
                        rgb(0xcdd6f4)
                    })
                    .child("Aa")
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.toggle_case(&ToggleCaseSensitive, window, cx);
                    })),
            )
            .child(
                div()
                    .id("search-prev")
                    .px_1p5()
                    .py_0p5()
                    .rounded_sm()
                    .cursor_pointer()
                    .bg(rgb(0x313244))
                    .hover(|s| s.bg(rgb(0x45475a)))
                    .child("↑")
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.find_prev(&FindPrevious, window, cx);
                    })),
            )
            .child(
                div()
                    .id("search-next")
                    .px_1p5()
                    .py_0p5()
                    .rounded_sm()
                    .cursor_pointer()
                    .bg(rgb(0x313244))
                    .hover(|s| s.bg(rgb(0x45475a)))
                    .child("↓")
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.find_next(&FindNext, window, cx);
                    })),
            )
            .child(
                div()
                    .id("search-close")
                    .px_1p5()
                    .py_0p5()
                    .rounded_sm()
                    .cursor_pointer()
                    .bg(rgb(0x313244))
                    .hover(|s| s.bg(rgb(0x45475a)))
                    .child("×")
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.close(&CloseSearch, window, cx);
                    })),
            )
    }
}
