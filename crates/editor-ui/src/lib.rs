//! `editor-ui`: embeddable GPUI code view (raw `gpui 0.2.2`, no component kit).
//!
//! M0: readonly viewer + optional single-cursor edit stub.
//! State lives in `Entity<EditorState>`; view holds the handle only.
//! `edit-buffer` and `syntax` stay GPUI-free so the host app can reuse them
//! headless.

use edit_buffer::{Buffer, Edit};
use gpui::*;
use std::ops::Range;
use syntax::{Capture, HighlightSpan, HighlightedVersion, Language};

// ---------------------------------------------------------------------------
// Model (pure, testable without a Window)
// ---------------------------------------------------------------------------

/// Edit capability flag. Readonly disables undo history + input handling.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Mode {
    #[default]
    ReadOnly,
    Editable,
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
        let version = buffer.version();
        let text = buffer.text().to_string();
        let highlight = syntax::highlight(&text, language, version);
        let mut this = Self {
            buffer,
            language,
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
        self.mode == Mode::ReadOnly
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
        self.highlight = syntax::highlight(&text, self.language, version);
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
                    for j in ix..end {
                        if chars[j] == open {
                            depth += 1;
                        } else if chars[j] == close {
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

pub struct EditorView {
    state: Entity<EditorState>,
}

impl EditorView {
    pub fn new(state: &Entity<EditorState>) -> Self {
        Self {
            state: state.clone(),
        }
    }
}

impl Render for EditorView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let state = self.state.read(cx);
        let line_count = state.line_count();

        // Capture per-line plain text + styled runs up front so render only
        // reads owned data (no borrow across frames).
        let lines: Vec<(String, Vec<(Range<usize>, Hsla)>)> = (0..line_count)
            .map(|row| {
                let text = state.line_text(row);
                let runs = state
                    .row_spans
                    .get(row as usize)
                    .map(|spans| {
                        spans
                            .iter()
                            .map(|s| (s.range.clone(), color_for(s.capture)))
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default();
                (text, runs)
            })
            .collect();

        div()
            .id("gpui-editor")
            .size_full()
            .bg(Hsla::black())
            .text_color(Hsla::white())
            .overflow_y_hidden()
            .child(
                div()
                    .flex()
                    .flex_col()
                    .text_sm()
                    .py_2()
                    .children(lines.into_iter().enumerate().map(|(ix, (text, runs))| {
                        render_line(ix as u32, text, runs)
                    })),
            )
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

fn render_line(row: u32, text: String, runs: Vec<(Range<usize>, Hsla)>) -> impl IntoElement {
    let line_no = format!("{:>4}", row + 1);
    div()
        .id(("editor-line", row as usize))
        .flex()
        .flex_row()
        .items_center()
        .px_3()
        .h(px(22.0))
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
                .flex()
                .flex_row()
                .flex_1()
                .overflow_x_hidden()
                .child(render_spans(text, runs)),
        )
}

fn render_spans(text: String, runs: Vec<(Range<usize>, Hsla)>) -> impl IntoElement {
    let chars: Vec<char> = text.chars().collect();
    if runs.is_empty() {
        let display_text = if text.is_empty() { " ".to_string() } else { text };
        return div().child(display_text).into_any_element();
    }

    let mut children: Vec<AnyElement> = Vec::new();
    let mut cursor = 0usize;
    for (range, color) in runs {
        let lo = range.start.min(chars.len());
        let hi = range.end.min(chars.len());
        if lo > cursor {
            let gap: String = chars[cursor..lo].iter().collect();
            if !gap.is_empty() {
                children.push(div().text_color(rgb(0xcdd6f4)).child(gap).into_any_element());
            }
        }
        if lo < hi {
            let word: String = chars[lo..hi].iter().collect();
            if !word.is_empty() {
                children.push(div().text_color(color).child(word).into_any_element());
            }
        }
        cursor = cursor.max(hi);
    }
    if cursor < chars.len() {
        let tail: String = chars[cursor..].iter().collect();
        if !tail.is_empty() {
            children.push(div().text_color(rgb(0xcdd6f4)).child(tail).into_any_element());
        }
    }
    if children.is_empty() {
        children.push(div().child(" ".to_string()).into_any_element());
    }

    div().flex().flex_row().children(children).into_any_element()
}

// Re-export for host apps.
pub use syntax::LanguageRegistry as Registry;
