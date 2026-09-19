//! Data types for `editor-ui`: the pure `EditorState` model (buffer +
//! selection + search + highlight cache) and the `EditorView` handle.
//! Rendering logic itself (the `Render` impl and its paint helpers) stays in
//! `lib.rs`, next to the free functions it shares with them.

use edit_buffer::{Buffer, Edit, Point};
use gpui::*;
use std::cell::Cell;
use std::ops::Range;
use std::rc::Rc;
use syntax::{Capture, HighlightSpan, HighlightedVersion, Language, ThemePreset};

use crate::{Copy, SelectAll};

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
    // `pub(crate)`: lib.rs's render path reads this directly per visible row.
    pub(crate) row_spans: Vec<Vec<StyledSpan>>,
}

#[derive(Debug, Clone)]
pub(crate) struct StyledSpan {
    /// Char range *within the row*.
    pub(crate) range: Range<usize>,
    pub(crate) capture: Capture,
    /// Theme's actual foreground color for this scope, from `lumis`.
    pub(crate) color: Option<(u8, u8, u8)>,
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

pub struct EditorView {
    pub(crate) state: Entity<EditorState>,
    pub(crate) scroll_handle: UniformListScrollHandle,
    pub(crate) focus_handle: FocusHandle,
    /// Last measured pixel height of the editor viewport, captured by a
    /// `canvas` each frame. Used to snap the visible list height to a whole
    /// multiple of `LINE_HEIGHT` so scrolling never shows a half-clipped row.
    pub(crate) viewport_height: Rc<Cell<Pixels>>,
    /// Monospace glyph advance width, measured lazily on first mouse
    /// interaction and cached (same font/size every row, so one shape call
    /// suffices for pixel -> column hit testing).
    pub(crate) char_width: Rc<Cell<Option<Pixels>>>,
    /// True while a left-mouse selection drag is in progress.
    pub(crate) selecting: Rc<Cell<bool>>,
    /// Fixed end of the in-progress drag; the other end follows the mouse.
    pub(crate) drag_anchor: Rc<Cell<Option<usize>>>,
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

    pub(crate) fn copy(&mut self, _: &Copy, _window: &mut Window, cx: &mut Context<Self>) {
        let text = self.state.read(cx).selected_text();
        if !text.is_empty() {
            cx.write_to_clipboard(ClipboardItem::new_string(text));
        }
    }

    pub(crate) fn select_all(&mut self, _: &SelectAll, _window: &mut Window, cx: &mut Context<Self>) {
        self.state.update(cx, |state, _| state.select_all());
        cx.notify();
    }
}

/// Shared, cheaply-clonable handles a rendered row's mouse closures need to
/// read/update selection state. Lives on `EditorView`; cloned once per row.
#[derive(Clone)]
pub(crate) struct RowInteraction {
    pub(crate) state: Entity<EditorState>,
    pub(crate) char_width: Rc<Cell<Option<Pixels>>>,
    pub(crate) selecting: Rc<Cell<bool>>,
    pub(crate) drag_anchor: Rc<Cell<Option<usize>>>,
}
