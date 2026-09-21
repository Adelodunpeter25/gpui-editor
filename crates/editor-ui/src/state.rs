//! `EditorState` — the model: buffer + selection + search + highlight cache,
//! plus the pure text helpers that read them (bracket match, newline indent,
//! widest-row scan). Testable without a `Window`; the only GPUI it touches is
//! `Context`, for spawning a background rehighlight.
//!
//! Plain vocabulary types it's built from (`Mode`, `Selection`,
//! `FontConfig`, …) live in `types.rs`, the retained view handle in
//! `view.rs`, and the `Render` impl plus its paint helpers in `lib.rs`.

use edit_buffer::{Buffer, Edit, Point};
use gpui::{px, AppContext, Context, Pixels};
use std::ops::Range;
use syntax::{HighlightSpan, Language, ThemePreset};

use crate::chars_display_width;
use crate::types::{FontConfig, IndentOptions, Mode, SearchState, Selection, StyledSpan};

/// Below this size, rehighlight runs synchronously inline rather than via
/// `cx.background_spawn` — see `EditorState::rehighlight_from`.
const SYNC_HIGHLIGHT_THRESHOLD: usize = 64_000;

pub struct EditorState {
    buffer: Buffer,
    language: Option<&'static Language>,
    theme: ThemePreset,
    mode: Mode,
    selection: Selection,
    search: SearchState,
    indent: IndentOptions,
    /// Whole-buffer word-wrap toggle. Off by default (matches today's
    /// clip-long-lines behavior). Actual wrap-point computation is a
    /// render-time concern owned by `EditorView`'s `WrapCache` (needs a
    /// `Window`/text system), not this GPUI-free model.
    wrap_enabled: bool,
    font: FontConfig,
    // Row -> highlight spans clipped to that row (rebuilt on edit/highlight).
    // M1 moves this to a shaped-line cache keyed by (row, version).
    // `pub(crate)`: lib.rs's render path reads this directly per visible row.
    pub(crate) row_spans: Vec<Vec<StyledSpan>>,
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
        let text = buffer.text().to_string();
        let mut this = Self {
            buffer,
            language,
            theme,
            mode,
            selection: Selection::default(),
            search: SearchState::default(),
            indent: IndentOptions::default(),
            wrap_enabled: false,
            font: FontConfig::default(),
            row_spans: Vec::new(),
        };
        // Construction has no `Context<Self>` yet (these constructors are
        // plain functions, not `cx.new(|cx| ...)` closures themselves) — a
        // synchronous first highlight is the only option here regardless of
        // file size; later mutation via `set_text`/etc. is what gets the
        // background-parse treatment.
        this.apply_highlight(&text);
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

    pub fn wrap_enabled(&self) -> bool {
        self.wrap_enabled
    }

    pub fn tab_width(&self) -> u32 {
        self.indent.tab_width
    }

    pub fn font(&self) -> &FontConfig {
        &self.font
    }

    pub fn line_text(&self, row: u32) -> String {
        self.buffer.line_text(row)
    }

    /// Full buffer text, owned. Allocates — for occasional whole-buffer use
    /// (e.g. feeding a `diff::DiffState`), not the per-frame render path.
    pub fn text(&self) -> String {
        self.buffer.text().to_string()
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

    pub fn with_wrap(mut self, enabled: bool) -> Self {
        self.wrap_enabled = enabled;
        self
    }

    pub fn set_wrap_enabled(&mut self, enabled: bool) {
        self.wrap_enabled = enabled;
    }

    pub fn with_font(mut self, font: FontConfig) -> Self {
        self.font = font;
        self
    }

    pub fn set_font(&mut self, font: FontConfig) {
        self.font = font;
    }

    pub fn set_language(&mut self, language: Option<&'static Language>, cx: &mut Context<Self>) {
        self.language = language;
        self.rehighlight(cx);
    }

    pub fn set_theme(&mut self, theme: ThemePreset, cx: &mut Context<Self>) {
        self.theme = theme;
        self.rehighlight(cx);
    }

    /// Replace whole text (e.g. file open). Resets selection/search.
    ///
    /// Rehighlights directly from `text` (the caller's own `&str`) rather
    /// than round-tripping through `self.buffer.text().to_string()` — for
    /// the file-open path this avoids a full extra copy of the file on top
    /// of the ones already needed to read it and build the rope.
    pub fn set_text(&mut self, text: &str, cx: &mut Context<Self>) {
        let len = self.buffer.len_chars();
        self.buffer.edit(&[Edit {
            range: 0..len,
            text: text.to_string(),
        }]);
        self.selection = Selection::default();
        self.rehighlight_from(text, cx);
        self.rerun_search();
    }

    /// M2 path: insert at char offset. No-op in readonly.
    pub fn insert(&mut self, offset: usize, text: &str, cx: &mut Context<Self>) {
        if self.is_readonly() {
            return;
        }
        self.buffer.edit(&[Edit {
            range: offset..offset,
            text: text.to_string(),
        }]);
        self.rehighlight(cx);
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

    /// Widest buffer row in pixels (tab stops honored, capped at the same
    /// `MAX_ROW_RENDER_CHARS` the view paints) — drives horizontal-scroll
    /// content width so the scroll range matches what's actually rendered.
    /// Zero-copy: scans rope slices, no per-line allocation.
    pub fn max_line_width(&self, char_width: Pixels) -> Pixels {
        let mut max = px(0.);
        for row in 0..self.line_count() {
            let w = chars_display_width(
                self.buffer.line_slice(row).chars(),
                char_width,
                self.indent.tab_width,
            );
            if w > max {
                max = w;
            }
        }
        max
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

    /// Rehighlight from the buffer's current text (an extra full-buffer
    /// clone). Used by callers that don't already hold the text as a
    /// `&str` (`set_language`/`set_theme`/`insert`). `set_text` skips this
    /// and calls `rehighlight_from` directly with its own parameter.
    fn rehighlight(&mut self, cx: &mut Context<Self>) {
        let text = self.buffer.text().to_string();
        self.rehighlight_from(&text, cx);
    }

    /// Highlight `text` (must match the buffer's current content) and
    /// rebuild `row_spans` from it — synchronously below
    /// `SYNC_HIGHLIGHT_THRESHOLD`, off the main thread above it (see
    /// `rehighlight_from`).
    fn apply_highlight(&mut self, text: &str) {
        let version = self.buffer.version();
        let highlight = syntax::highlight_themed(text, self.language, version, Some(self.theme));
        self.rebuild_row_spans(&highlight.spans);
    }

    /// Highlight `text` (must match the buffer's current content). Small
    /// files highlight synchronously inline — a `cx.background_spawn`
    /// round-trip (task scheduling + a second `cx.update` dispatch) costs
    /// more than it saves when `highlight_themed` already finishes in far
    /// under a millisecond. Above `SYNC_HIGHLIGHT_THRESHOLD`, the parse
    /// moves to a background thread so it can't block repaint, and the
    /// result is discarded on arrival if a newer edit/open already changed
    /// the buffer's version by then (an old file's highlight can't flash
    /// onto whatever's open now).
    fn rehighlight_from(&mut self, text: &str, cx: &mut Context<Self>) {
        if text.len() <= SYNC_HIGHLIGHT_THRESHOLD {
            self.apply_highlight(text);
            return;
        }

        let version = self.buffer.version();
        let owned_text = text.to_string();
        let language = self.language;
        let theme = self.theme;
        let task = cx.background_spawn(async move {
            syntax::highlight_themed(&owned_text, language, version, Some(theme))
        });
        cx.spawn(async move |this, cx| {
            let highlight = task.await;
            let _ = this.update(cx, |state, cx| {
                if state.buffer.version() != highlight.buffer_version {
                    // A newer edit/open landed before this parse finished;
                    // it no longer describes the buffer's current content.
                    return;
                }
                state.rebuild_row_spans(&highlight.spans);
                cx.notify();
            });
        })
        .detach();
    }

    /// Split flat char-offset spans into per-row spans for O(visible) render.
    fn rebuild_row_spans(&mut self, spans: &[HighlightSpan]) {
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

        for span in spans {
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
        let base: String = line
            .chars()
            .take_while(|c| *c == ' ' || *c == '\t')
            .collect();
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
