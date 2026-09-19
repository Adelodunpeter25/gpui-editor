//! Small, hand-verified cases — not real files or `.patch` fixtures.
//! `similar`'s own test suite already covers Myers-algorithm correctness;
//! these tests cover *our* translation of its ops into `DiffLine`s, which
//! is easiest to verify by eye on inputs small enough to check by hand.

use diff::{diff_lines, DiffLineKind};

#[test]
fn identical_text_is_all_context_no_changes() {
    let result = diff_lines("a\nb\nc", "a\nb\nc");
    assert_eq!(result.added, 0);
    assert_eq!(result.removed, 0);
    assert!(result.lines.iter().all(|l| l.kind == DiffLineKind::Context));
}

#[test]
fn empty_to_text_is_all_added() {
    let result = diff_lines("", "a\nb");
    assert_eq!(result.added, 2);
    assert_eq!(result.removed, 0);
    assert!(result.lines.iter().all(|l| l.kind == DiffLineKind::Added));
}

#[test]
fn text_to_empty_is_all_removed() {
    let result = diff_lines("a\nb", "");
    assert_eq!(result.added, 0);
    assert_eq!(result.removed, 2);
    assert!(result.lines.iter().all(|l| l.kind == DiffLineKind::Removed));
}

#[test]
fn single_line_replacement_reports_remove_then_add() {
    let result = diff_lines("a\nb\nc", "a\nX\nc");
    assert_eq!(result.added, 1);
    assert_eq!(result.removed, 1);

    let kinds: Vec<DiffLineKind> = result.lines.iter().map(|l| l.kind).collect();
    assert_eq!(
        kinds,
        vec![
            DiffLineKind::Context,
            DiffLineKind::Removed,
            DiffLineKind::Added,
            DiffLineKind::Context,
        ]
    );

    let removed = &result.lines[1];
    assert_eq!(removed.text, "b");
    assert_eq!(removed.old_line, Some(2));
    assert_eq!(removed.new_line, None);

    let added = &result.lines[2];
    assert_eq!(added.text, "X");
    assert_eq!(added.old_line, None);
    assert_eq!(added.new_line, Some(2));
}

#[test]
fn context_lines_carry_both_old_and_new_line_numbers() {
    let result = diff_lines("a\nb", "a\nb");
    assert_eq!(result.lines[0].old_line, Some(1));
    assert_eq!(result.lines[0].new_line, Some(1));
    assert_eq!(result.lines[1].old_line, Some(2));
    assert_eq!(result.lines[1].new_line, Some(2));
}

#[test]
fn appended_line_is_reported_as_addition_after_context() {
    let result = diff_lines("a\nb", "a\nb\nc");
    assert_eq!(result.added, 1);
    assert_eq!(result.removed, 0);
    let last = result.lines.last().unwrap();
    assert_eq!(last.kind, DiffLineKind::Added);
    assert_eq!(last.text, "c");
    assert_eq!(last.new_line, Some(3));
}
