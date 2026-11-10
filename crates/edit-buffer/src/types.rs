//! Data types for `edit-buffer`: `Buffer` (rope-backed text storage) and its
//! supporting value types.

use std::ops::Range;

use ropey::{Rope, RopeSlice};

/// Row/col point. `row` is 0-based line index, `col` is char offset within line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Point {
    pub row: u32,
    pub col: u32,
}

/// A single replacement: replace `range` (char offsets) with `text`.
#[derive(Debug, Clone)]
pub struct Edit {
    pub range: Range<usize>,
    pub text: String,
}

/// Cheap snapshot handle: version + length for stale-task rejection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BufferSnapshot {
    pub version: u64,
    pub len_chars: usize,
}

/// One `edit()` call's worth of history: the edits as applied, and the
/// edits that revert them.
#[derive(Debug, Clone)]
struct UndoStep {
    /// Edits as applied, in pre-edit coordinates.
    forward: Vec<Edit>,
    /// Edits restoring the prior text, in post-edit coordinates.
    inverse: Vec<Edit>,
}

/// Bounded undo entry: one `edit()` call, or the run of several same-group
/// calls (see `edit_grouped`). Steps are kept in the order they were
/// applied; each one's offsets are only valid against the text as it stood
/// at that point, so undo replays inverses newest-first and redo replays
/// forwards oldest-first.
#[derive(Debug, Clone)]
struct UndoEntry {
    steps: Vec<UndoStep>,
    /// Calls sharing this id merge into this entry (typing-run grouping).
    group: Option<u64>,
}

#[derive(Debug)]
pub struct Buffer {
    rope: Rope,
    version: u64,
    undo: Vec<UndoEntry>,
    redo: Vec<UndoEntry>,
    /// Max undo depth. 0 disables history (readonly low-RAM mode).
    max_history: usize,
}

impl Buffer {
    pub fn from_text(text: &str) -> Self {
        Self {
            rope: Rope::from_str(text),
            version: 0,
            undo: Vec::new(),
            redo: Vec::new(),
            max_history: 100,
        }
    }

    /// Readonly constructor: disables undo history to save RAM.
    pub fn from_text_readonly(text: &str) -> Self {
        Self {
            rope: Rope::from_str(text),
            version: 0,
            undo: Vec::new(),
            redo: Vec::new(),
            max_history: 0,
        }
    }

    pub fn version(&self) -> u64 {
        self.version
    }

    pub fn len_chars(&self) -> usize {
        self.rope.len_chars()
    }

    pub fn line_count(&self) -> u32 {
        // ropey counts a trailing newline as an extra empty line; clamp empty doc to 1.
        self.rope.len_lines() as u32
    }

    pub fn snapshot(&self) -> BufferSnapshot {
        BufferSnapshot {
            version: self.version,
            len_chars: self.len_chars(),
        }
    }

    /// Borrow a line without cloning. Caller must not hold across `edit`.
    pub fn line_slice(&self, row: u32) -> RopeSlice<'_> {
        self.rope.line(row as usize)
    }

    /// Owned line string (strips trailing \n for rendering).
    /// Prefer `line_slice` in hot paths; this allocates.
    pub fn line_text(&self, row: u32) -> String {
        let s = self.line_slice(row).to_string();
        s.strip_suffix('\n').unwrap_or(&s).to_string()
    }

    pub fn text(&self) -> &Rope {
        &self.rope
    }

    /// Owned text for a char-offset range, clamped to buffer bounds.
    pub fn slice_text(&self, range: Range<usize>) -> String {
        let start = range.start.min(self.len_chars());
        let end = range.end.min(self.len_chars()).max(start);
        self.rope.slice(start..end).to_string()
    }

    /// Buffer length in UTF-16 code units (what platform text-input APIs
    /// count in).
    pub fn len_utf16(&self) -> usize {
        self.rope.len_utf16_cu()
    }

    /// Char offset -> UTF-16 code-unit offset (clamped).
    pub fn char_to_utf16(&self, offset: usize) -> usize {
        self.rope.char_to_utf16_cu(offset.min(self.len_chars()))
    }

    /// UTF-16 code-unit offset -> char offset (clamped). An offset landing
    /// inside a surrogate pair resolves to the char that pair encodes.
    pub fn utf16_to_char(&self, offset: usize) -> usize {
        self.rope.utf16_cu_to_char(offset.min(self.len_utf16()))
    }

    /// Char offset -> Point.
    pub fn offset_to_point(&self, offset: usize) -> Point {
        let line = self.rope.char_to_line(offset.min(self.len_chars()));
        let line_start = self.rope.line_to_char(line);
        Point {
            row: line as u32,
            col: (offset - line_start) as u32,
        }
    }

    /// Point -> char offset (clamped).
    pub fn point_to_offset(&self, point: Point) -> usize {
        let row = (point.row as usize).min(self.rope.len_lines().saturating_sub(1));
        let line_start = self.rope.line_to_char(row);
        let line_len = self.rope.line(row).len_chars();
        // Exclude trailing newline from col clamp.
        let max_col = line_len.saturating_sub(1).max(line_len.min(1));
        let col = (point.col as usize).min(line_len);
        let _ = max_col;
        line_start + col
    }

    /// Apply edits, bump version, push inverse onto undo stack.
    /// Returns new version.
    pub fn edit(&mut self, edits: &[Edit]) -> u64 {
        self.edit_grouped(edits, None)
    }

    /// Apply edits, bump version, and record history. Consecutive calls with
    /// the same `Some(group)` coalesce into ONE undo entry — the seam typing
    /// runs (chars, backspaces) use so undo works per run, not per keystroke.
    /// `None` (or a changed id) always starts a fresh entry. Returns new version.
    pub fn edit_grouped(&mut self, edits: &[Edit], group: Option<u64>) -> u64 {
        if edits.is_empty() {
            return self.version;
        }
        let step = self.apply(edits);
        self.version += 1;
        if self.max_history > 0 {
            self.redo.clear();
            if let (Some(g), Some(last)) = (group, self.undo.last_mut()) {
                if last.group == Some(g) {
                    last.steps.push(step);
                    return self.version;
                }
            }
            self.undo.push(UndoEntry {
                steps: vec![step],
                group,
            });
            if self.undo.len() > self.max_history {
                self.undo.remove(0);
            }
        }
        self.version
    }

    /// Apply `edits` (ranges in current-text coordinates, non-overlapping)
    /// to the rope and return the step that records them.
    fn apply(&mut self, edits: &[Edit]) -> UndoStep {
        let len = self.len_chars();
        let mut forward: Vec<Edit> = edits
            .iter()
            .map(|e| {
                let start = e.range.start.min(len);
                let end = e.range.end.min(len).max(start);
                Edit {
                    range: start..end,
                    text: e.text.clone(),
                }
            })
            .collect();
        forward.sort_by_key(|e| e.range.start);

        // Each inverse range sits where the inserted text lands once every
        // earlier edit in this call has shifted it.
        let mut inverse: Vec<Edit> = Vec::with_capacity(forward.len());
        let mut shift: isize = 0;
        for e in &forward {
            let inserted = e.text.chars().count();
            let start = (e.range.start as isize + shift) as usize;
            inverse.push(Edit {
                range: start..start + inserted,
                text: self.rope.slice(e.range.clone()).to_string(),
            });
            shift += inserted as isize - (e.range.end - e.range.start) as isize;
        }
        self.replace_all(&forward);
        UndoStep { forward, inverse }
    }

    /// Replace each range with its text. `edits` must be sorted by start and
    /// non-overlapping; applied back-to-front so earlier offsets stay valid.
    fn replace_all(&mut self, edits: &[Edit]) {
        for e in edits.iter().rev() {
            let start = e.range.start.min(self.rope.len_chars());
            let end = e.range.end.min(self.rope.len_chars()).max(start);
            self.rope.remove(start..end);
            if !e.text.is_empty() {
                self.rope.insert(start, &e.text);
            }
        }
    }

    /// Drop all undo/redo history (e.g. after loading a different file).
    pub fn clear_history(&mut self) {
        self.undo.clear();
        self.redo.clear();
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    /// Undo last edit group. Returns new version or None.
    pub fn undo(&mut self) -> Option<u64> {
        let entry = self.undo.pop()?;
        for step in entry.steps.iter().rev() {
            self.replace_all(&step.inverse);
        }
        self.version += 1;
        self.redo.push(entry);
        Some(self.version)
    }

    /// Redo last undone group. Returns new version or None.
    pub fn redo(&mut self) -> Option<u64> {
        let entry = self.redo.pop()?;
        for step in &entry.steps {
            self.replace_all(&step.forward);
        }
        self.version += 1;
        self.undo.push(entry);
        if self.undo.len() > self.max_history.max(1) {
            self.undo.remove(0);
        }
        Some(self.version)
    }

    /// Literal search over rope chars. Returns char-offset ranges, capped at `limit`.
    /// Case-insensitive folds via `char::to_lowercase` (no regex, no alloc per line).
    pub fn search(&self, needle: &str, case_sensitive: bool, limit: usize) -> Vec<Range<usize>> {
        if needle.is_empty() || limit == 0 {
            return Vec::new();
        }
        // Materialize once — M3 will replace with chunked memchr scan for >1MB.
        // M0 files are small demo texts; single String keeps CPU low vs per-line allocs.
        let hay = self.rope.to_string();
        if case_sensitive {
            hay.match_indices(needle)
                .take(limit)
                .map(|(byte_ix, _)| {
                    let start = hay[..byte_ix].chars().count();
                    start..start + needle.chars().count()
                })
                .collect()
        } else {
            let hay_l = hay.to_lowercase();
            let needle_l = needle.to_lowercase();
            hay_l
                .match_indices(&needle_l)
                .take(limit)
                .map(|(byte_ix, _)| {
                    let start = hay_l[..byte_ix].chars().count();
                    start..start + needle_l.chars().count()
                })
                .collect()
        }
    }
}
