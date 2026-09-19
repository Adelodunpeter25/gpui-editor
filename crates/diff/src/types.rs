//! Data types for `diff`: line-level diff output between two texts.

use similar::{capture_diff_slices_deadline, Algorithm, DiffTag};
use std::time::{Duration, Instant};

/// Worst-case time budget for one diff computation. Myers is O(ND) where D
/// is the number of differences — near-instant for the common case (small
/// changes to a large file), but two large, near-totally-different texts
/// can make D approach N. This bounds that instead of letting a pathological
/// pair of inputs block the caller indefinitely; `similar` returns its best
/// partial result so far once the deadline hits rather than erroring.
const DIFF_DEADLINE: Duration = Duration::from_millis(500);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiffLineKind {
    Context,
    Added,
    Removed,
}

/// One rendered row of a diff view: file-order (not unified-hunk-grouped),
/// which is what a scrollable UI wants — no hunk headers to skip over.
#[derive(Debug, Clone)]
pub struct DiffLine {
    pub kind: DiffLineKind,
    pub text: String,
    /// 1-based line number in `old`, present for `Context`/`Removed`.
    pub old_line: Option<u32>,
    /// 1-based line number in `new`, present for `Context`/`Added`.
    pub new_line: Option<u32>,
}

#[derive(Debug, Clone, Default)]
pub struct DiffResult {
    pub lines: Vec<DiffLine>,
    pub added: usize,
    pub removed: usize,
}

impl DiffResult {
    pub fn is_empty(&self) -> bool {
        self.lines.is_empty()
    }
}

/// Compute a line-level diff between `old` and `new` using the Myers
/// algorithm (O(ND) where D is the number of differences — the same engine
/// used by gitoxide and rustc; fast for large files with small changes,
/// the common case for reviewing an edit).
///
/// Bounded by `DIFF_DEADLINE` so a pathological pair of large, mostly-
/// distinct texts can't block the caller indefinitely.
pub fn diff_lines(old: &str, new: &str) -> DiffResult {
    let old_lines: Vec<&str> = if old.is_empty() {
        Vec::new()
    } else {
        old.split('\n').collect()
    };
    let new_lines: Vec<&str> = if new.is_empty() {
        Vec::new()
    } else {
        new.split('\n').collect()
    };

    let deadline = Instant::now() + DIFF_DEADLINE;
    let ops = capture_diff_slices_deadline(Algorithm::Myers, &old_lines, &new_lines, Some(deadline));

    let mut lines: Vec<DiffLine> = Vec::new();
    let mut added = 0usize;
    let mut removed = 0usize;

    for op in ops {
        let old_range = op.old_range();
        let new_range = op.new_range();
        match op.tag() {
            DiffTag::Equal => {
                for offset in 0..old_range.len() {
                    lines.push(DiffLine {
                        kind: DiffLineKind::Context,
                        text: old_lines[old_range.start + offset].to_owned(),
                        old_line: Some((old_range.start + offset + 1) as u32),
                        new_line: Some((new_range.start + offset + 1) as u32),
                    });
                }
            }
            DiffTag::Delete => {
                for offset in 0..old_range.len() {
                    lines.push(DiffLine {
                        kind: DiffLineKind::Removed,
                        text: old_lines[old_range.start + offset].to_owned(),
                        old_line: Some((old_range.start + offset + 1) as u32),
                        new_line: None,
                    });
                    removed += 1;
                }
            }
            DiffTag::Insert => {
                for offset in 0..new_range.len() {
                    lines.push(DiffLine {
                        kind: DiffLineKind::Added,
                        text: new_lines[new_range.start + offset].to_owned(),
                        old_line: None,
                        new_line: Some((new_range.start + offset + 1) as u32),
                    });
                    added += 1;
                }
            }
            DiffTag::Replace => {
                for offset in 0..old_range.len() {
                    lines.push(DiffLine {
                        kind: DiffLineKind::Removed,
                        text: old_lines[old_range.start + offset].to_owned(),
                        old_line: Some((old_range.start + offset + 1) as u32),
                        new_line: None,
                    });
                    removed += 1;
                }
                for offset in 0..new_range.len() {
                    lines.push(DiffLine {
                        kind: DiffLineKind::Added,
                        text: new_lines[new_range.start + offset].to_owned(),
                        old_line: None,
                        new_line: Some((new_range.start + offset + 1) as u32),
                    });
                    added += 1;
                }
            }
        }
    }

    DiffResult {
        lines,
        added,
        removed,
    }
}
