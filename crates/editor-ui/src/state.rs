//! `EditorState` — the model: buffer + selection + search + highlight cache,
//! plus the pure text helpers that read them (bracket match, newline indent,
//! widest-row scan). Testable without a `Window`; the only GPUI it touches is
//! `Context`, for spawning a background rehighlight.
//!
//! Plain vocabulary types it's built from (`Mode`, `Selection`,
//! `FontConfig`, …) live in `types.rs`, the retained view handle in
//! `view.rs`, and the `Render` impl plus its paint helpers in `lib.rs`.

use edit_buffer::{Buffer, Edit, LineEnding, Point};
use gpui::{px, AppContext, Context, EventEmitter, Pixels};
use std::cell::RefCell;
use std::io;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use syntax::{HighlightSpan, Language, ThemePreset};

use crate::chars_display_width;
use crate::save::{write_atomic, EditorEvent, SaveHandler, SaveRequest, SaveStatus};
use crate::types::{CommentTarget, FontConfig, IndentOptions, Mode, SearchState, Selection, StyledSpan};

/// Below this size, rehighlight runs synchronously inline rather than via
/// `cx.background_spawn` — see `EditorState::rehighlight_from`.
const SYNC_HIGHLIGHT_THRESHOLD: usize = 64_000;

/// Consecutive typing edits (`type_text`) within this window coalesce into
/// one undo entry, so undo works per typing run instead of per keystroke.
/// Any structural edit (`replace_range`, `set_text`, undo/redo) closes the
/// run.
const TYPE_GROUP_WINDOW: Duration = Duration::from_millis(500);

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
    comment_gutter: bool,
    comment_target_handler: Option<Box<dyn Fn(CommentTarget)>>,
    /// Line ranges the host has marked as comment targets. They stay
    /// highlighted after the drag ends and are cleared only when the host
    /// removes the comment.
    comment_selections: Vec<CommentTarget>,
    font: FontConfig,
    /// Current typing-run undo group (`type_text` only). `None` after any
    /// structural edit, so the next keystroke starts a fresh undo entry.
    typing_group: Option<u64>,
    /// Allocator for typing-group ids.
    next_group: u64,
    /// When the last typing edit landed — decides run continuation.
    last_typing: Option<Instant>,
    /// Line break the document uses on disk. The buffer itself holds `\n`
    /// only; this is re-applied on save.
    line_ending: LineEnding,
    /// File this buffer is bound to — where `save` writes by default.
    path: Option<PathBuf>,
    /// Host override for `save` (remote files, save-as, format-on-save).
    save_handler: Option<SaveHandler>,
    /// Buffer revision at the last save/load; differing from the buffer's
    /// current revision is what "dirty" means.
    saved_revision: u64,
    /// In-progress IME composition (dead keys, CJK input): the char range
    /// of provisional text the input method may still replace. `None` when
    /// not composing.
    marked: Option<Range<usize>>,
    /// Undo group shared by every edit of one composition, so a committed
    /// composition undoes as a single entry however many updates built it.
    composition_group: Option<u64>,
    // Row -> highlight spans clipped to that row (rebuilt on edit/highlight).
    // M1 moves this to a shaped-line cache keyed by (row, version).
    // `pub(crate)`: lib.rs's render path reads this directly per visible row.
    pub(crate) row_spans: Vec<Vec<StyledSpan>>,
    /// Per-row display widths for `max_line_width` (h-scroll sizing),
    /// spliced incrementally per edit so typing costs O(rows) instead of a
    /// full-buffer char scan. `None` = needs a full scan (first read, font
    /// or tab-width change, or after undo/redo where the affected rows
    /// aren't tracked). `RefCell` because reads (`max_line_width`) take
    /// `&self` while filling the cache.
    row_widths: RefCell<Option<RowWidthCache>>,
}

impl EventEmitter<EditorEvent> for EditorState {}

/// How an edit is recorded in undo history.
#[derive(Clone, Copy, PartialEq)]
enum EditKind {
    /// Its own undo entry (paste, cut, Enter, programmatic edits).
    Structural,
    /// Coalesces with neighbouring typing inside `TYPE_GROUP_WINDOW`.
    Typing,
    /// One update of an IME composition; all of a composition's updates,
    /// and the edit that commits it, share one entry.
    Composing,
}

/// Identity of a `RowWidthCache`: a different char width (font change) or
/// tab width changes every row's display width, voiding the cache.
#[derive(Clone, Copy, PartialEq)]
struct RowWidthKey {
    char_width: f32,
    tab_width: u32,
}

struct RowWidthCache {
    key: RowWidthKey,
    /// Display width of each buffer row, `widths[row]`.
    widths: Vec<Pixels>,
}

impl EditorState {
    pub fn readonly(text: &str, language: Option<&'static Language>) -> Self {
        let buffer = Buffer::from_text_readonly(&LineEnding::normalize(text));
        Self::from_buffer(buffer, language, Mode::ReadOnly).with_line_ending(LineEnding::detect(text))
    }

    /// Editable buffer over `text`. Line breaks are normalized to `\n` in
    /// the buffer (so `text()` and every offset are `\n`-based); the
    /// ending `text` used is remembered and written back by `save`.
    pub fn editable(text: &str, language: Option<&'static Language>) -> Self {
        let buffer = Buffer::from_text(&LineEnding::normalize(text));
        Self::from_buffer(buffer, language, Mode::Editable).with_line_ending(LineEnding::detect(text))
    }

    /// Open `path` for editing: reads it (UTF-8) and binds the editor to
    /// it, so `save` writes back to the same file.
    pub fn open(path: impl Into<PathBuf>, language: Option<&'static Language>) -> io::Result<Self> {
        let path = path.into();
        let text = std::fs::read_to_string(&path)?;
        Ok(Self::editable(&text, language).with_path(path))
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
            comment_gutter: false,
            comment_target_handler: None,
            comment_selections: Vec::new(),
            font: FontConfig::default(),
            typing_group: None,
            next_group: 0,
            last_typing: None,
            line_ending: LineEnding::default(),
            path: None,
            save_handler: None,
            saved_revision: 0,
            marked: None,
            composition_group: None,
            row_spans: Vec::new(),
            row_widths: RefCell::new(None),
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

    /// Indent options (tab width + hard-tabs flag), for Tab insertion.
    pub fn indent_options(&self) -> IndentOptions {
        self.indent
    }

    /// Total buffer length in chars (selection clamping, caret bounds).
    pub fn len_chars(&self) -> usize {
        self.buffer.len_chars()
    }

    pub fn font(&self) -> &FontConfig {
        &self.font
    }

    pub fn line_text(&self, row: u32) -> String {
        self.buffer.line_text(row)
    }

    /// Full buffer text, owned, with `\n` line breaks whatever the file
    /// uses on disk (see `line_ending`). Allocates — for occasional
    /// whole-buffer use (e.g. feeding a `diff::DiffState`), not the
    /// per-frame render path.
    pub fn text(&self) -> String {
        self.buffer.text().to_string()
    }

    // -- builders / setters --------------------------------------------------

    pub fn with_mode(mut self, mode: Mode) -> Self {
        self.mode = mode;
        self
    }

    fn with_line_ending(mut self, ending: LineEnding) -> Self {
        self.line_ending = ending;
        self
    }

    /// Line break `save` writes. Detected from the loaded text (first break
    /// wins; a file mixing endings is saved with that one throughout).
    pub fn line_ending(&self) -> LineEnding {
        self.line_ending
    }

    /// Convert the document to a different line break on its next save.
    /// Doesn't change the buffer or the dirty state.
    pub fn set_line_ending(&mut self, ending: LineEnding) {
        self.line_ending = ending;
    }

    /// Bind the editor to a file path (see `save`).
    pub fn with_path(mut self, path: impl Into<PathBuf>) -> Self {
        self.path = Some(path.into());
        self
    }

    /// Change or clear the file this editor saves to. Doesn't touch the
    /// text or the dirty state.
    pub fn set_path(&mut self, path: Option<PathBuf>) {
        self.path = path;
    }

    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    /// Take over saving. With a handler set, `save` hands it a
    /// `SaveRequest` instead of writing to `path` itself — for remote
    /// files, a save-as dialog, format-on-save, and the like. `None`
    /// restores the default.
    pub fn set_save_handler(&mut self, handler: Option<SaveHandler>) {
        self.save_handler = handler;
    }

    /// True when the text differs from what was last saved or loaded.
    /// Undoing back to the saved text reads as clean again.
    pub fn is_dirty(&self) -> bool {
        self.buffer.revision() != self.saved_revision
    }

    /// Content revision of the current text (see `SaveRequest::revision`).
    pub fn revision(&self) -> u64 {
        self.buffer.revision()
    }

    /// Record that `revision` is what's persisted. For save handlers that
    /// returned `SaveStatus::Deferred`: call this once the write lands.
    pub fn mark_saved(&mut self, revision: u64, cx: &mut Context<Self>) {
        self.saved_revision = revision;
        cx.emit(EditorEvent::Saved);
        cx.notify();
    }

    /// Save the buffer: through the save handler if one is set, otherwise
    /// straight to `path` (atomically — see `save.rs`). Emits
    /// `EditorEvent::Saved` / `SaveFailed`. Returns `false` without doing
    /// anything when there's nothing to save with: the editor is readonly,
    /// or has neither a handler nor a path.
    pub fn save(&mut self, cx: &mut Context<Self>) -> bool {
        if self.is_readonly() {
            return false;
        }
        let revision = self.buffer.revision();
        let result = if let Some(handler) = &self.save_handler {
            handler(SaveRequest {
                text: self
                    .line_ending
                    .apply(&self.buffer.text().to_string())
                    .into_owned(),
                path: self.path.clone(),
                revision,
            })
        } else if let Some(path) = &self.path {
            write_atomic(path, self.buffer.text(), self.line_ending).map(|()| SaveStatus::Saved)
        } else {
            return false;
        };
        match result {
            Ok(SaveStatus::Saved) => self.mark_saved(revision, cx),
            Ok(SaveStatus::Deferred) => {}
            Err(error) => {
                cx.emit(EditorEvent::SaveFailed(error.to_string()));
                cx.notify();
            }
        }
        true
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

    pub fn with_comment_gutter(mut self, enabled: bool) -> Self {
        self.comment_gutter = enabled;
        self
    }

    pub fn set_comment_gutter(&mut self, enabled: bool, _cx: &mut Context<Self>) {
        self.comment_gutter = enabled;
    }

    pub fn set_comment_target_handler(&mut self, handler: Option<Box<dyn Fn(CommentTarget)>>) {
        self.comment_target_handler = handler;
    }

    pub fn comment_gutter(&self) -> bool {
        self.comment_gutter
    }

    /// Replace the persisted comment ranges (the host owns the comment list).
    pub fn set_comment_selections(&mut self, selections: Vec<CommentTarget>) {
        self.comment_selections = selections;
    }

    /// Add one range if it is not already highlighted.
    pub fn add_comment_selection(&mut self, target: CommentTarget) {
        if !self.comment_selections.contains(&target) {
            self.comment_selections.push(target);
        }
    }

    pub fn comment_selections(&self) -> &[CommentTarget] {
        &self.comment_selections
    }

    pub(crate) fn report_comment_target(&self, target: CommentTarget) {
        if let Some(handler) = &self.comment_target_handler {
            handler(target);
        }
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

    /// Replace whole text (e.g. file open). Resets selection/search, drops
    /// undo history, and marks the buffer clean: this is a different
    /// document, not an edit to the previous one, so undo must not be able
    /// to bring the old text back.
    ///
    /// Rehighlights directly from `text` (the caller's own `&str`) rather
    /// than round-tripping through `self.buffer.text().to_string()` — for
    /// the file-open path this avoids a full extra copy of the file on top
    /// of the ones already needed to read it and build the rope.
    pub fn set_text(&mut self, text: &str, cx: &mut Context<Self>) {
        self.line_ending = LineEnding::detect(text);
        let text = &*LineEnding::normalize(text);
        let len = self.buffer.len_chars();
        self.buffer.edit(&[Edit {
            range: 0..len,
            text: text.to_string(),
        }]);
        self.buffer.clear_history();
        self.saved_revision = self.buffer.revision();
        self.end_typing_run();
        self.marked = None;
        self.composition_group = None;
        *self.row_widths.borrow_mut() = None;
        self.selection = Selection::default();
        self.rehighlight_from(text, cx);
        self.rerun_search();
        cx.notify();
    }

    /// M2 path: insert at char offset. No-op in readonly.
    pub fn insert(&mut self, offset: usize, text: &str, cx: &mut Context<Self>) {
        self.replace_range(offset..offset, text, cx);
    }

    /// Structural edit: replace `range` with `text` as its own undo entry
    /// (paste, cut, programmatic edits). Collapses the selection to the end
    /// of the inserted text. No-op in readonly.
    pub fn replace_range(&mut self, range: Range<usize>, text: &str, cx: &mut Context<Self>) {
        self.replace_impl(range, text, EditKind::Structural, cx);
    }

    /// Typing edit: same as `replace_range`, but consecutive calls within
    /// `TYPE_GROUP_WINDOW` coalesce into one undo entry. Text input,
    /// Backspace, and Delete route here.
    pub fn type_text(&mut self, range: Range<usize>, text: &str, cx: &mut Context<Self>) {
        self.replace_impl(range, text, EditKind::Typing, cx);
    }

    /// IME composition update: replace `range` with provisional `text` and
    /// mark it as still being composed. `selected` is the caret/selection
    /// the input method wants inside the new text, in chars relative to its
    /// start (`None` = caret at its end). Empty `text` cancels the
    /// composition. The next edit commits it; see `marked_range`.
    pub fn set_marked_text(
        &mut self,
        range: Range<usize>,
        text: &str,
        selected: Option<Range<usize>>,
        cx: &mut Context<Self>,
    ) {
        if self.is_readonly() {
            return;
        }
        let text = &*LineEnding::normalize(text);
        let start = range.start.min(self.buffer.len_chars());
        self.replace_impl(range, text, EditKind::Composing, cx);
        let len = text.chars().count();
        if len == 0 {
            self.composition_group = None;
            return;
        }
        self.marked = Some(start..start + len);
        if let Some(selected) = selected {
            let lo = selected.start.min(selected.end).min(len);
            let hi = selected.end.max(selected.start).min(len);
            self.selection = Selection {
                range: start + lo..start + hi,
            };
        }
    }

    /// Char range of the in-progress IME composition, if any.
    pub fn marked_range(&self) -> Option<Range<usize>> {
        self.marked.clone()
    }

    /// End the composition, keeping its text as ordinary buffer content.
    pub fn unmark_text(&mut self, cx: &mut Context<Self>) {
        self.composition_group = None;
        if self.marked.take().is_some() {
            cx.notify();
        }
    }

    /// Pick the undo group an edit of `kind` belongs to, updating the
    /// typing-run and composition bookkeeping.
    fn undo_group_for(&mut self, kind: EditKind) -> Option<u64> {
        if kind == EditKind::Composing {
            self.end_typing_run();
            if self.composition_group.is_none() {
                self.composition_group = Some(self.next_group);
                self.next_group += 1;
            }
            return self.composition_group;
        }
        // The first edit after a composition commits it: same undo entry.
        if let Some(group) = self.composition_group.take() {
            self.end_typing_run();
            return Some(group);
        }
        if kind == EditKind::Structural {
            self.end_typing_run();
            return None;
        }
        let now = Instant::now();
        let continues = match (self.typing_group, self.last_typing) {
            (Some(_), Some(last)) => now.duration_since(last) < TYPE_GROUP_WINDOW,
            _ => false,
        };
        if !continues {
            self.typing_group = Some(self.next_group);
            self.next_group += 1;
        }
        self.last_typing = Some(now);
        self.typing_group
    }

    fn replace_impl(
        &mut self,
        range: Range<usize>,
        text: &str,
        kind: EditKind,
        cx: &mut Context<Self>,
    ) {
        if self.is_readonly() {
            return;
        }
        // Pasted or typed text may carry `\r\n`; the buffer holds `\n` only.
        let text = &*LineEnding::normalize(text);
        let group = self.undo_group_for(kind);
        let clamped_start = range.start.min(self.buffer.len_chars());
        let clamped_end = range.end.min(self.buffer.len_chars());
        let start_row = self.buffer.offset_to_point(clamped_start).row;
        let end_row_old = self.buffer.offset_to_point(clamped_end).row;
        let inserted = text.chars().count();
        let caret = clamped_start + inserted;
        self.buffer
            .edit_grouped(&[Edit { range, text: text.to_string() }], group);
        let end_row_new = self.buffer.offset_to_point(caret).row;
        self.update_row_widths(start_row, end_row_old, end_row_new);
        // Collapse onto the caret just past the inserted text.
        self.selection = Selection {
            range: caret..caret,
        };
        self.marked = None;
        self.rehighlight(cx);
        self.rerun_search();
        // Every buffer mutation repaints its views: callers (key handlers,
        // hosts) must not have to remember to notify.
        cx.notify();
    }

    pub fn can_undo(&self) -> bool {
        self.buffer.can_undo()
    }

    pub fn can_redo(&self) -> bool {
        self.buffer.can_redo()
    }

    /// Undo one edit group (typing run / structural edit). No-op in readonly
    /// (history is disabled there anyway); leaves the caret at the old
    /// selection start, clamped to the restored buffer.
    pub fn undo(&mut self, cx: &mut Context<Self>) {
        if self.buffer.undo().is_some() {
            self.after_history_move(cx);
        }
    }

    /// Redo the most recently undone edit group.
    pub fn redo(&mut self, cx: &mut Context<Self>) {
        if self.buffer.redo().is_some() {
            self.after_history_move(cx);
        }
    }

    fn after_history_move(&mut self, cx: &mut Context<Self>) {
        self.end_typing_run();
        self.marked = None;
        self.composition_group = None;
        // Undo/redo can shrink or shift any rows; the affected range isn't
        // tracked here, so let the next read do a full width scan.
        *self.row_widths.borrow_mut() = None;
        let head = self.selection.range.start.min(self.buffer.len_chars());
        self.selection = Selection { range: head..head };
        self.rehighlight(cx);
        self.rerun_search();
        // Every buffer mutation repaints its views: callers (key handlers,
        // hosts) must not have to remember to notify.
        cx.notify();
    }

    /// Set the selection (clicks, drags, caret motion). Moving the caret
    /// ends the current typing run, so the next keystroke starts its own
    /// undo entry, and commits any IME composition in progress.
    pub fn set_selection(&mut self, range: Range<usize>) {
        self.end_typing_run();
        // Moving the caret away abandons a composition: its text stays as
        // typed, and the next input starts fresh at the new caret.
        self.marked = None;
        self.composition_group = None;
        self.selection = Selection { range };
    }

    fn end_typing_run(&mut self) {
        self.typing_group = None;
        self.last_typing = None;
    }

    /// Select the text of a 1-based line number, clamping to the buffer.
    pub fn select_line(&mut self, line: u32) {
        let line_count = self.line_count();
        if line_count == 0 {
            return;
        }
        let row = line.saturating_sub(1).min(line_count.saturating_sub(1));
        self.end_typing_run();
        self.selection = Selection {
            range: self.row_start_offset(row)..self.line_end_offset(row),
        };
    }

    pub fn select_all(&mut self) {
        let len = self.buffer.len_chars();
        self.end_typing_run();
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

    /// Char offset of the end of `row`'s text, excluding its trailing newline.
    pub fn line_end_offset(&self, row: u32) -> usize {
        if row + 1 < self.line_count() {
            self.row_end_offset(row) - 1
        } else {
            self.buffer.len_chars()
        }
    }

    /// The edit Enter performs with the caret at `head`: an insertion at
    /// the caret of `"\n"` + auto-indent. The indent is derived from the
    /// text *before* the caret only, so splitting a line mid-way never
    /// indents for an opener that ends up on the new line. Pure helper,
    /// exposed for tests.
    pub fn newline_edit(&self, head: usize) -> (Range<usize>, String) {
        let head = head.min(self.buffer.len_chars());
        let point = self.offset_to_point(head);
        let before: String = self
            .buffer
            .line_text(point.row)
            .chars()
            .take(point.col as usize)
            .collect();
        let indent = self.indent_after(&before);
        (head..head, format!("\n{indent}"))
    }

    /// Char offset for a (row, col) point, clamped by the buffer.
    pub fn point_to_offset(&self, point: Point) -> usize {
        self.buffer.point_to_offset(point)
    }

    /// (row, col) point for a char offset.
    pub fn offset_to_point(&self, offset: usize) -> Point {
        self.buffer.offset_to_point(offset)
    }

    /// Buffer length in UTF-16 code units — the unit platform text-input
    /// (IME) APIs address text in.
    pub fn len_utf16(&self) -> usize {
        self.buffer.len_utf16()
    }

    /// Char offset -> UTF-16 code-unit offset (clamped).
    pub fn offset_to_utf16(&self, offset: usize) -> usize {
        self.buffer.char_to_utf16(offset)
    }

    /// UTF-16 code-unit offset -> char offset (clamped).
    pub fn utf16_to_offset(&self, offset: usize) -> usize {
        self.buffer.utf16_to_char(offset)
    }

    /// Owned text for a char-offset range, clamped to the buffer.
    pub fn text_in_range(&self, range: Range<usize>) -> String {
        self.buffer.slice_text(range)
    }

    /// Widest row's display width (h-scroll sizing). Served from the
    /// per-row width cache — O(rows) max-scan — once built; edits splice
    /// their affected rows in `replace_impl` instead of rescanning the
    /// whole buffer per keystroke.
    pub fn max_line_width(&self, char_width: Pixels) -> Pixels {
        let key = RowWidthKey {
            char_width: char_width.into(),
            tab_width: self.indent.tab_width,
        };
        {
            let cache = self.row_widths.borrow();
            if let Some(c) = cache.as_ref() {
                if c.key == key {
                    return c.widths.iter().copied().fold(px(0.), |a, w| w.max(a));
                }
            }
        }
        let widths: Vec<Pixels> = (0..self.line_count())
            .map(|row| self.row_width(row, key))
            .collect();
        let max = widths.iter().copied().fold(px(0.), |a, w| w.max(a));
        *self.row_widths.borrow_mut() = Some(RowWidthCache { key, widths });
        max
    }

    fn row_width(&self, row: u32, key: RowWidthKey) -> Pixels {
        chars_display_width(
            self.buffer.line_slice(row).chars(),
            px(key.char_width),
            key.tab_width,
        )
    }

    /// Re-measure only the rows an edit touched (`start_row..=end_row_old`
    /// became `start_row..=end_row_new`), shifting later rows in place.
    /// Drops a cache whose tab width no longer matches — the next
    /// `max_line_width` read does a full scan under the current key.
    fn update_row_widths(&self, start_row: u32, end_row_old: u32, end_row_new: u32) {
        let mut cache = self.row_widths.borrow_mut();
        match cache.as_mut() {
            Some(c) if c.key.tab_width == self.indent.tab_width => {
                let key = c.key;
                let old_len = c.widths.len();
                let lo = (start_row as usize).min(old_len);
                let hi = ((end_row_old as usize + 1).min(old_len)).max(lo);
                let replacement = (start_row..=end_row_new).map(|row| self.row_width(row, key));
                c.widths.splice(lo..hi, replacement);
            }
            _ => *cache = None,
        }
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
        self.indent_after(&self.buffer.line_text(row))
    }

    /// Indent for a new line following `line`: its leading whitespace, plus
    /// one level if it ends with an opener.
    fn indent_after(&self, line: &str) -> String {
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
