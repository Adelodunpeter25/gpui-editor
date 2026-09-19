//! `edit-buffer`: single-owner text storage for gpui-editor.
//!
//! No GPUI dependency. Text lives once in a `ropey::Rope`.
//! All offsets are **char offsets** (not bytes) to stay O(1)-ish with ropey.
//! Byte conversion happens at the syntax/render boundary.

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

/// Bounded undo entry: inverse of one `edit()` call.
#[derive(Debug, Clone)]
struct UndoEntry {
    /// Edits as applied (forward).
    forward: Vec<Edit>,
    /// Inverse edits to restore prior text.
    inverse: Vec<Edit>,
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
        if edits.is_empty() {
            return self.version;
        }
        // Build inverse by reading current text at each range (apply back-to-front
        // so earlier offsets stay valid while collecting).
        let mut sorted: Vec<&Edit> = edits.iter().collect();
        sorted.sort_by_key(|e| e.range.start);

        let mut inverse: Vec<Edit> = Vec::with_capacity(sorted.len());
        for e in sorted.iter().rev() {
            let start = e.range.start.min(self.len_chars());
            let end = e.range.end.min(self.len_chars());
            let old: String = self.rope.slice(start..end).to_string();
            inverse.push(Edit {
                range: start..start + e.text.chars().count(),
                text: old,
            });
        }
        // Apply forward back-to-front so ranges don't shift.
        for e in sorted.iter().rev() {
            let start = e.range.start.min(self.rope.len_chars());
            let end = e.range.end.min(self.rope.len_chars());
            self.rope.remove(start..end);
            if !e.text.is_empty() {
                self.rope.insert(start, &e.text);
            }
        }

        self.version += 1;
        if self.max_history > 0 {
            // Recompute forward ranges for redo: forward ranges as given, but clamped
            // at apply time on redo. Store owned copy.
            let forward: Vec<Edit> = edits.to_vec();
            // Inverse currently reversed (back-to-front order); store in apply order.
            inverse.reverse();
            self.undo.push(UndoEntry { forward, inverse });
            if self.undo.len() > self.max_history {
                self.undo.remove(0);
            }
            self.redo.clear();
        }
        self.version
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
        // Apply inverse back-to-front.
        let mut sorted = entry.inverse.clone();
        sorted.sort_by_key(|e| e.range.start);
        for e in sorted.iter().rev() {
            let start = e.range.start.min(self.rope.len_chars());
            let end = e.range.end.min(self.rope.len_chars());
            self.rope.remove(start..end);
            if !e.text.is_empty() {
                self.rope.insert(start, &e.text);
            }
        }
        self.version += 1;
        self.redo.push(entry);
        Some(self.version)
    }

    /// Redo last undone group. Returns new version or None.
    pub fn redo(&mut self) -> Option<u64> {
        let entry = self.redo.pop()?;
        let mut sorted = entry.forward.clone();
        sorted.sort_by_key(|e| e.range.start);
        for e in sorted.iter().rev() {
            let start = e.range.start.min(self.rope.len_chars());
            let end = e.range.end.min(self.rope.len_chars());
            self.rope.remove(start..end);
            if !e.text.is_empty() {
                self.rope.insert(start, &e.text);
            }
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
