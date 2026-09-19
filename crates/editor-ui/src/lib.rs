//! `editor-ui`: embeddable GPUI code view (raw `gpui 0.2.2`, no component kit).
//!
//! M0: readonly viewer + optional single-cursor edit stub.
//! State lives in `Entity<EditorState>`; view holds the handle only.
//! `edit-buffer` and `syntax` stay GPUI-free so the host app can reuse them
//! headless.

use edit_buffer::{Buffer, Edit, Point};
use gpui::*;
use std::cell::Cell;
use std::ops::Range;
use std::rc::Rc;
use syntax::{Capture, HighlightSpan, HighlightedVersion, Language, ThemePreset};

/// Fixed row height every editor line renders at. Virtualized scroll math in
/// `EditorView::render` depends on this staying constant per `render_line`.
const LINE_HEIGHT: Pixels = px(22.0);

// ---------------------------------------------------------------------------
// Model (pure, testable without a Window)
// ---------------------------------------------------------------------------

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

pub struct EditorState {
    buffer: Buffer,
    language: Option<&'static Language>,
    theme: ThemePreset,
    highlight: HighlightedVersion,
    mode: Mode,
    selection: Selection,
    search: SearchState,
    indent: IndentOptions,
    // Row -> highlight spans clipped to that row (rebuilt on edit/highlight).
    // M1 moves this to a shaped-line cache keyed by (row, version).
    row_spans: Vec<Vec<StyledSpan>>,
}

#[derive(Debug, Clone)]
struct StyledSpan {
    /// Char range *within the row*.
    range: Range<usize>,
    capture: Capture,
    /// Theme's actual foreground color for this scope, from `lumis`.
    color: Option<(u8, u8, u8)>,
}

impl EditorState {
    pub fn readonly(text: &str, language: Option<&'static Language>) -> Self {
        let buffer = Buffer::from_text_readonly(text);
        Self::from_buffer(buffer, language, Mode::ReadOnly)
    }

    pub fn editable(text: &str, language: Option<&'static Language>) -> Self {
        let buffer = Buffer::from_text(text);
        Self::from_buffer(buffer, language, Mode::Editable)
    }

    fn from_buffer(buffer: Buffer, language: Option<&'static Language>, mode: Mode) -> Self {
        let theme = ThemePreset::GitHubDark;
        let version = buffer.version();
        let text = buffer.text().to_string();
        let highlight = syntax::highlight_themed(&text, language, version, Some(theme));
        let mut this = Self {
            buffer,
            language,
            theme,
            highlight,
            mode,
            selection: Selection::default(),
            search: SearchState::default(),
            indent: IndentOptions::default(),
            row_spans: Vec::new(),
        };
        this.rebuild_row_spans();
        this
    }

    // -- readers (plain nouns) -------------------------------------------------

    pub fn mode(&self) -> Mode {
        self.mode
    }

    pub fn is_readonly(&self) -> bool {
        self.mode.is_readonly()
    }

    pub fn line_count(&self) -> u32 {
        self.buffer.line_count()
    }

    pub fn version(&self) -> u64 {
        self.buffer.version()
    }

    pub fn selection(&self) -> &Selection {
        &self.selection
    }

    pub fn search(&self) -> &SearchState {
        &self.search
    }

    pub fn language(&self) -> Option<&'static Language> {
        self.language
    }

    pub fn theme(&self) -> ThemePreset {
        self.theme
    }

    pub fn line_text(&self, row: u32) -> String {
        self.buffer.line_text(row)
    }

    // -- builders / setters --------------------------------------------------

    pub fn with_mode(mut self, mode: Mode) -> Self {
        self.mode = mode;
        self
    }

    pub fn with_indent(mut self, indent: IndentOptions) -> Self {
        self.indent = indent;
        self
    }

    pub fn set_language(&mut self, language: Option<&'static Language>) {
        self.language = language;
        self.rehighlight();
    }

    pub fn set_theme(&mut self, theme: ThemePreset) {
        self.theme = theme;
        self.rehighlight();
    }

    /// Replace whole text (e.g. file open). Resets selection/search.
    pub fn set_text(&mut self, text: &str) {
        let len = self.buffer.len_chars();
        self.buffer.edit(&[Edit {
            range: 0..len,
            text: text.to_string(),
        }]);
        self.selection = Selection::default();
        self.rehighlight();
        self.rerun_search();
    }

    /// M2 path: insert at char offset. No-op in readonly.
    pub fn insert(&mut self, offset: usize, text: &str) {
        if self.is_readonly() {
            return;
        }
        self.buffer.edit(&[Edit {
            range: offset..offset,
            text: text.to_string(),
        }]);
        self.rehighlight();
        self.rerun_search();
    }

    pub fn set_selection(&mut self, range: Range<usize>) {
        self.selection = Selection { range };
    }

    pub fn select_all(&mut self) {
        let len = self.buffer.len_chars();
        self.selection = Selection { range: 0..len };
    }

    /// Copyable text for the current selection (empty string if none).
    pub fn selected_text(&self) -> String {
        self.buffer.slice_text(self.selection.range.clone())
    }

    /// Char offset of the first column of `row`.
    pub fn row_start_offset(&self, row: u32) -> usize {
        self.buffer.point_to_offset(Point { row, col: 0 })
    }

    /// Char offset just past the end of `row` (including its newline, if any).
    pub fn row_end_offset(&self, row: u32) -> usize {
        if row + 1 < self.line_count() {
            self.row_start_offset(row + 1)
        } else {
            self.buffer.len_chars()
        }
    }

    /// Char offset for a (row, col) point, clamped by the buffer.
    pub fn point_to_offset(&self, point: Point) -> usize {
        self.buffer.point_to_offset(point)
    }

    // -- search (M3, sync literal; debounced overlay lands in view) ----------

    pub fn set_search(&mut self, query: &str, case_sensitive: bool) {
        self.search.query = query.to_string();
        self.search.case_sensitive = case_sensitive;
        self.rerun_search();
    }

    fn rerun_search(&mut self) {
        if self.search.query.is_empty() {
            self.search.matches.clear();
            self.search.current = None;
            return;
        }
        self.search.matches =
            self.buffer
                .search(&self.search.query, self.search.case_sensitive, 1000);
        self.search.current = if self.search.matches.is_empty() {
            None
        } else {
            Some(0)
        };
    }

    pub fn next_match(&mut self) {
        if self.search.matches.is_empty() {
            return;
        }
        let next = self.search.current.map(|i| i + 1).unwrap_or(0) % self.search.matches.len();
        self.search.current = Some(next);
    }

    // -- highlight -----------------------------------------------------------

    fn rehighlight(&mut self) {
        let version = self.buffer.version();
        let text = self.buffer.text().to_string();
        self.highlight = syntax::highlight_themed(&text, self.language, version, Some(self.theme));
        self.rebuild_row_spans();
    }

    /// Split flat char-offset spans into per-row spans for O(visible) render.
    fn rebuild_row_spans(&mut self) {
        let rows = self.buffer.line_count() as usize;
        self.row_spans.clear();
        self.row_spans.resize_with(rows, Vec::new);
        // Row start char offsets.
        let mut row_starts: Vec<usize> = Vec::with_capacity(rows + 1);
        let mut acc = 0usize;
        for row in 0..rows {
            row_starts.push(acc);
            acc += self.buffer.line_slice(row as u32).len_chars();
        }
        row_starts.push(acc);

        for span in &self.highlight.spans {
            let s: &HighlightSpan = span;
            if s.start >= s.end {
                continue;
            }
            let start_row = match row_starts.binary_search(&s.start) {
                Ok(r) => r,
                Err(r) => r.saturating_sub(1),
            };
            let end_row = match row_starts.binary_search(&s.end) {
                Ok(r) => r,
                Err(r) => r.saturating_sub(1),
            };
            for row in start_row..=end_row.min(rows.saturating_sub(1)) {
                let rs = row_starts[row];
                let re = row_starts[row + 1];
                let lo = s.start.max(rs).saturating_sub(rs);
                let hi = s.end.min(re).saturating_sub(rs);
                if lo < hi {
                    self.row_spans[row].push(StyledSpan {
                        range: lo..hi,
                        capture: s.capture,
                        color: s.color,
                    });
                }
            }
        }
    }

    // -- bracket match (pure text scan, works without tree-sitter) -----------

    /// If `offset` is adjacent to a bracket, return (open_offset, close_offset).
    /// Scans max 5000 chars each direction to bound CPU.
    pub fn bracket_match(&self, offset: usize) -> Option<(usize, usize)> {
        let text = self.buffer.text().to_string();
        let chars: Vec<char> = text.chars().collect();
        if chars.is_empty() {
            return None;
        }
        const LIMIT: usize = 5000;
        let at = offset.min(chars.len());
        // Candidate bracket at cursor or just before it.
        let candidates = [at, at.saturating_sub(1)];
        for &ix in &candidates {
            if ix >= chars.len() {
                continue;
            }
            let c = chars[ix];
            if let Some((open, close, fwd)) = bracket_pair(c) {
                if fwd {
                    // scan forward for mate
                    let mut depth = 0usize;
                    let end = (ix + LIMIT + 1).min(chars.len());
                    for (j, &ch) in chars.iter().enumerate().take(end).skip(ix) {
                        if ch == open {
                            depth += 1;
                        } else if ch == close {
                            depth -= 1;
                            if depth == 0 {
                                return Some((ix, j));
                            }
                        }
                    }
                } else {
                    // scan backward
                    let mut depth = 0usize;
                    let start = ix.saturating_sub(LIMIT);
                    for j in (start..=ix).rev() {
                        if chars[j] == close {
                            depth += 1;
                        } else if chars[j] == open {
                            depth -= 1;
                            if depth == 0 {
                                return Some((j, ix));
                            }
                        }
                    }
                }
            }
        }
        None
    }

    /// Indent text for a newline inserted after `row`: keep-indent + one extra
    /// level if the line ends with an opener.
    pub fn indent_for_newline(&self, row: u32) -> String {
        let line = self.buffer.line_text(row);
        let base: String = line.chars().take_while(|c| *c == ' ' || *c == '\t').collect();
        let trimmed = line.trim_end();
        let extra = if trimmed.ends_with('{') || trimmed.ends_with('[') || trimmed.ends_with('(') {
            if self.indent.hard_tabs {
                "\t".to_string()
            } else {
                " ".repeat(self.indent.tab_width as usize)
            }
        } else {
            String::new()
        };
        format!("{base}{extra}")
    }
}

fn bracket_pair(c: char) -> Option<(char, char, bool)> {
    match c {
        '(' => Some(('(', ')', true)),
        '[' => Some(('[', ']', true)),
        '{' => Some(('{', '}', true)),
        ')' => Some(('(', ')', false)),
        ']' => Some(('[', ']', false)),
        '}' => Some(('{', '}', false)),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// View (retained Entity seam for the host app)
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

pub struct EditorView {
    state: Entity<EditorState>,
    scroll_handle: UniformListScrollHandle,
    focus_handle: FocusHandle,
    /// Last measured pixel height of the editor viewport, captured by a
    /// `canvas` each frame. Used to snap the visible list height to a whole
    /// multiple of `LINE_HEIGHT` so scrolling never shows a half-clipped row.
    viewport_height: Rc<Cell<Pixels>>,
    /// Monospace glyph advance width, measured lazily on first mouse
    /// interaction and cached (same font/size every row, so one shape call
    /// suffices for pixel -> column hit testing).
    char_width: Rc<Cell<Option<Pixels>>>,
    /// True while a left-mouse selection drag is in progress.
    selecting: Rc<Cell<bool>>,
    /// Fixed end of the in-progress drag; the other end follows the mouse.
    drag_anchor: Rc<Cell<Option<usize>>>,
}

impl EditorView {
    pub fn new(state: &Entity<EditorState>, cx: &mut Context<Self>) -> Self {
        Self {
            state: state.clone(),
            scroll_handle: UniformListScrollHandle::new(),
            focus_handle: cx.focus_handle(),
            viewport_height: Rc::new(Cell::new(px(0.))),
            char_width: Rc::new(Cell::new(None)),
            selecting: Rc::new(Cell::new(false)),
            drag_anchor: Rc::new(Cell::new(None)),
        }
    }

    fn copy(&mut self, _: &Copy, _window: &mut Window, cx: &mut Context<Self>) {
        let text = self.state.read(cx).selected_text();
        if !text.is_empty() {
            cx.write_to_clipboard(ClipboardItem::new_string(text));
        }
    }

    fn select_all(&mut self, _: &SelectAll, _window: &mut Window, cx: &mut Context<Self>) {
        self.state.update(cx, |state, _| state.select_all());
        cx.notify();
    }
}

/// Shared, cheaply-clonable handles a rendered row's mouse closures need to
/// read/update selection state. Lives on `EditorView`; cloned once per row.
#[derive(Clone)]
struct RowInteraction {
    state: Entity<EditorState>,
    char_width: Rc<Cell<Option<Pixels>>>,
    selecting: Rc<Cell<bool>>,
    drag_anchor: Rc<Cell<Option<usize>>>,
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
            .on_action(cx.listener(Self::copy))
            .on_action(cx.listener(Self::select_all))
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
                                let row_selection = row_local_selection(
                                    &selection, row_start, row_end,
                                );
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
                    down.state.update(cx, |state, _| state.set_selection(offset..offset));
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
                    let range = if anchor <= head { anchor..head } else { head..anchor };
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
