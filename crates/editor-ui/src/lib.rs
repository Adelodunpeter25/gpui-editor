//! `editor-ui`: embeddable GPUI code view (raw `gpui 0.2.2`, no component kit).
//!
//! M0: readonly viewer + optional single-cursor edit stub.
//! State lives in `Entity<EditorState>`; view holds the handle only.
//! `edit-buffer` and `syntax` stay GPUI-free so the host app can reuse them
//! headless. Model/view types live in `types.rs`; this file holds the
//! `Render` impl and its paint helpers.

mod types;

use edit_buffer::Point;
use gpui::*;
use std::cell::Cell;
use std::ops::Range;
use std::rc::Rc;
use syntax::Capture;
use types::{RowInteraction, StyledSpan};

pub use types::{EditorState, EditorView, IndentOptions, Mode, SearchState, Selection};

/// Fixed row height every editor line renders at. Virtualized scroll math in
/// `EditorView::render` depends on this staying constant per `render_line`.
const LINE_HEIGHT: Pixels = px(22.0);

// ---------------------------------------------------------------------------
// Actions + view rendering
// ---------------------------------------------------------------------------

actions!(editor_ui, [Copy, SelectAll]);

/// Bind default keys for `editor_ui`'s actions. Call once at app startup.
pub fn init(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("cmd-c", Copy, Some("Editor")),
        KeyBinding::new("ctrl-c", Copy, Some("Editor")),
        KeyBinding::new("cmd-a", SelectAll, Some("Editor")),
        KeyBinding::new("ctrl-a", SelectAll, Some("Editor")),
    ]);
}

/// Measure (and cache) the monospace glyph advance width for pixel -> column
/// hit testing. Same font/size on every row, so one shape call suffices.
fn measure_char_width(cache: &Rc<Cell<Option<Pixels>>>, window: &mut Window) -> Pixels {
    if let Some(w) = cache.get() {
        return w;
    }
    let run = TextRun {
        len: 1,
        font: font("JetBrains Mono"),
        color: Hsla::white(),
        background_color: None,
        underline: None,
        strikethrough: None,
    };
    let width = window
        .text_system()
        .shape_line("M".into(), px(14.), &[run], None)
        .width();
    cache.set(Some(width));
    width
}

/// Pixel x (relative to the row content's left edge) -> char column, clamped
/// to `row_len`.
fn column_for_x(local_x: Pixels, char_width: Pixels, row_len: usize) -> u32 {
    let col = (local_x.max(px(0.)) / char_width).round().max(0.) as usize;
    col.min(row_len) as u32
}

fn resolve_color(span: &StyledSpan) -> Hsla {
    span.color
        .map(|(r, g, b)| rgb(((r as u32) << 16) | ((g as u32) << 8) | b as u32).into())
        .unwrap_or_else(|| color_for(span.capture))
}

impl Render for EditorView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let line_count = self.state.read(cx).line_count() as usize;

        let measured = self.viewport_height.get();
        let rows_that_fit = (measured / LINE_HEIGHT).floor();
        let list_height = if rows_that_fit > 0. {
            LINE_HEIGHT * rows_that_fit
        } else {
            measured
        };
        let viewport_height = self.viewport_height.clone();
        let interaction = RowInteraction {
            state: self.state.clone(),
            char_width: self.char_width.clone(),
            selecting: self.selecting.clone(),
            drag_anchor: self.drag_anchor.clone(),
        };

        div()
            .id("gpui-editor")
            .relative()
            .size_full()
            .bg(Hsla::black())
            .text_color(Hsla::white())
            .track_focus(&self.focus_handle)
            .key_context("Editor")
            .on_action(cx.listener(EditorView::copy))
            .on_action(cx.listener(EditorView::select_all))
            .on_mouse_up(MouseButton::Left, {
                let selecting = self.selecting.clone();
                move |_event, _window, _cx| selecting.set(false)
            })
            // Paint-less sibling purely to measure the container's pixel
            // height each frame, so the list below can snap to whole lines.
            .child(
                canvas(
                    move |bounds, _window, _cx| viewport_height.set(bounds.size.height),
                    |_, _, _, _| {},
                )
                .absolute()
                .size_full(),
            )
            .child(
                uniform_list(
                    "gpui-editor-lines",
                    line_count,
                    cx.processor(move |this, range: Range<usize>, _window, cx| {
                        let state = this.state.read(cx);
                        let selection = state.selection().range.clone();
                        range
                            .map(|row| {
                                let text = state.line_text(row as u32);
                                let runs = state
                                    .row_spans
                                    .get(row)
                                    .map(|spans| {
                                        spans
                                            .iter()
                                            .map(|s| (s.range.clone(), resolve_color(s)))
                                            .collect::<Vec<_>>()
                                    })
                                    .unwrap_or_default();
                                let row_start = state.row_start_offset(row as u32);
                                let row_end = state.row_end_offset(row as u32);
                                let row_selection =
                                    row_local_selection(&selection, row_start, row_end);
                                render_line(
                                    row as u32,
                                    text,
                                    runs,
                                    row_selection,
                                    interaction.clone(),
                                )
                            })
                            .collect()
                    }),
                )
                .track_scroll(&self.scroll_handle)
                .text_sm()
                .py_2()
                .w_full()
                .h(list_height),
            )
    }
}

/// Clip the whole-buffer selection range to `row_start..row_end`, in row-local
/// char coords. `None` if the row has no overlap with the selection.
fn row_local_selection(
    selection: &Range<usize>,
    row_start: usize,
    row_end: usize,
) -> Option<Range<usize>> {
    let lo = selection.start.max(row_start);
    let hi = selection.end.min(row_end);
    if lo < hi {
        Some(lo - row_start..hi - row_start)
    } else {
        None
    }
}

fn color_for(capture: Capture) -> Hsla {
    match capture {
        Capture::Keyword => rgb(0x89b4fa).into(),
        Capture::String => rgb(0xa6e3a1).into(),
        Capture::Comment => rgb(0x6c7086).into(),
        Capture::Number => rgb(0xfab387).into(),
        Capture::Function => rgb(0x89dceb).into(),
        Capture::Type => rgb(0xf9e2af).into(),
        Capture::Plain => rgb(0xcdd6f4).into(),
    }
}

/// Selection wash color: translucent blue behind selected glyphs.
const SELECTION_BG: u32 = 0x3b82f680;

fn render_line(
    row: u32,
    text: String,
    runs: Vec<(Range<usize>, Hsla)>,
    selection: Option<Range<usize>>,
    interaction: RowInteraction,
) -> impl IntoElement {
    let line_no = format!("{:>4}", row + 1);
    let row_len = text.chars().count();

    let content_x = Rc::new(Cell::new(px(0.)));
    let measure_x = content_x.clone();

    let down = interaction.clone();
    let down_content_x = content_x.clone();
    let move_ = interaction.clone();
    let move_content_x = content_x;

    div()
        .id(("editor-line", row as usize))
        .flex()
        .flex_row()
        .items_center()
        .px_3()
        .h(LINE_HEIGHT)
        .font_family("JetBrains Mono")
        .text_sm()
        .child(
            div()
                .w_12()
                .flex_shrink_0()
                .text_color(rgb(0x585b70))
                .child(line_no),
        )
        .child(
            div()
                .id(("editor-line-content", row as usize))
                .relative()
                .flex()
                .flex_row()
                .flex_1()
                .overflow_x_hidden()
                .child(
                    canvas(
                        move |bounds, _window, _cx| measure_x.set(bounds.origin.x),
                        |_, _, _, _| {},
                    )
                    .absolute()
                    .size_full(),
                )
                .child(render_spans(text, runs, selection))
                .on_mouse_down(MouseButton::Left, move |event, window, cx| {
                    let width = measure_char_width(&down.char_width, window);
                    let local_x = event.position.x - down_content_x.get();
                    let col = column_for_x(local_x, width, row_len);
                    let offset = down.state.read(cx).point_to_offset(Point { row, col });
                    down.drag_anchor.set(Some(offset));
                    down.selecting.set(true);
                    down.state
                        .update(cx, |state, _| state.set_selection(offset..offset));
                    window.refresh();
                })
                .on_mouse_move(move |event, window, cx| {
                    if !move_.selecting.get() {
                        return;
                    }
                    let Some(anchor) = move_.drag_anchor.get() else {
                        return;
                    };
                    let width = measure_char_width(&move_.char_width, window);
                    let local_x = event.position.x - move_content_x.get();
                    let col = column_for_x(local_x, width, row_len);
                    let head = move_.state.read(cx).point_to_offset(Point { row, col });
                    let range = if anchor <= head {
                        anchor..head
                    } else {
                        head..anchor
                    };
                    move_.state.update(cx, |state, _| state.set_selection(range));
                    window.refresh();
                }),
        )
}

fn render_spans(
    text: String,
    runs: Vec<(Range<usize>, Hsla)>,
    selection: Option<Range<usize>>,
) -> impl IntoElement {
    let chars: Vec<char> = text.chars().collect();
    let len = chars.len();
    if len == 0 {
        return div().child(" ".to_string()).into_any_element();
    }

    // Merge syntax-run and selection edges into one sorted breakpoint list so
    // every resulting segment has a single color + selected flag.
    let mut breaks: Vec<usize> = vec![0, len];
    for (range, _) in &runs {
        breaks.push(range.start.min(len));
        breaks.push(range.end.min(len));
    }
    if let Some(sel) = &selection {
        breaks.push(sel.start.min(len));
        breaks.push(sel.end.min(len));
    }
    breaks.sort_unstable();
    breaks.dedup();

    let mut children: Vec<AnyElement> = Vec::new();
    for pair in breaks.windows(2) {
        let (lo, hi) = (pair[0], pair[1]);
        if lo >= hi {
            continue;
        }
        let color = runs
            .iter()
            .find(|(range, _)| range.start <= lo && hi <= range.end)
            .map(|(_, color)| *color)
            .unwrap_or_else(|| rgb(0xcdd6f4).into());
        let selected = selection
            .as_ref()
            .is_some_and(|sel| sel.start <= lo && hi <= sel.end);
        let word: String = chars[lo..hi].iter().collect();
        let mut el = div().text_color(color);
        if selected {
            el = el.bg(rgba(SELECTION_BG));
        }
        children.push(el.child(word).into_any_element());
    }
    if children.is_empty() {
        children.push(div().child(" ".to_string()).into_any_element());
    }

    div().flex().flex_row().children(children).into_any_element()
}

// Re-export for host apps.
pub use syntax::LanguageRegistry as Registry;
