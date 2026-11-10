use edit_buffer::{Buffer, Edit};

#[test]
fn edit_bumps_version_and_reads_lines() {
    let mut b = Buffer::from_text("hello\nworld\n");
    assert_eq!(b.line_count(), 3);
    assert_eq!(b.line_text(0), "hello");
    let v = b.edit(&[Edit {
        range: 0..5,
        text: "hi".into(),
    }]);
    assert_eq!(v, 1);
    assert_eq!(b.line_text(0), "hi");
}

#[test]
fn undo_redo_roundtrip() {
    let mut b = Buffer::from_text("abc");
    b.edit(&[Edit {
        range: 0..1,
        text: "z".into(),
    }]);
    assert_eq!(b.text().to_string(), "zbc");
    b.undo();
    assert_eq!(b.text().to_string(), "abc");
    b.redo();
    assert_eq!(b.text().to_string(), "zbc");
}

#[test]
fn readonly_has_no_history() {
    let mut b = Buffer::from_text_readonly("abc");
    b.edit(&[Edit {
        range: 0..1,
        text: "z".into(),
    }]);
    assert!(!b.can_undo());
}

#[test]
fn search_caps_results() {
    let b = Buffer::from_text("foo Foo foo");
    assert_eq!(b.search("foo", true, 10).len(), 2);
    assert_eq!(b.search("foo", false, 10).len(), 3);
    assert_eq!(b.search("foo", false, 1).len(), 1);
}

#[test]
fn slice_text_returns_the_given_range() {
    let b = Buffer::from_text("hello world");
    assert_eq!(b.slice_text(0..5), "hello");
    assert_eq!(b.slice_text(6..11), "world");
}

#[test]
fn slice_text_clamps_out_of_bounds_ranges() {
    let b = Buffer::from_text("hi");
    assert_eq!(b.slice_text(0..100), "hi");
    assert_eq!(b.slice_text(50..100), "");
}

#[test]
fn same_group_edits_coalesce_into_one_undo() {
    let mut b = Buffer::from_text("ab");
    b.edit_grouped(
        &[Edit {
            range: 2..2,
            text: "c".into(),
        }],
        Some(1),
    );
    b.edit_grouped(
        &[Edit {
            range: 3..3,
            text: "d".into(),
        }],
        Some(1),
    );
    assert_eq!(b.text().to_string(), "abcd");
    // One undo reverts the whole run, not just the last keystroke.
    b.undo();
    assert_eq!(b.text().to_string(), "ab");
    assert!(b.can_redo());
    b.redo();
    assert_eq!(b.text().to_string(), "abcd");
}

#[test]
fn different_groups_undo_separately() {
    let mut b = Buffer::from_text("");
    b.edit_grouped(
        &[Edit {
            range: 0..0,
            text: "a".into(),
        }],
        Some(1),
    );
    b.edit_grouped(
        &[Edit {
            range: 1..1,
            text: "b".into(),
        }],
        Some(2),
    );
    b.undo();
    assert_eq!(b.text().to_string(), "a");
    b.undo();
    assert_eq!(b.text().to_string(), "");
}

#[test]
fn a_new_run_clears_redo() {
    let mut b = Buffer::from_text("x");
    b.edit(&[Edit {
        range: 1..1,
        text: "a".into(),
    }]);
    b.undo();
    assert!(b.can_redo());
    b.edit_grouped(
        &[Edit {
            range: 1..1,
            text: "b".into(),
        }],
        Some(7),
    );
    b.edit_grouped(
        &[Edit {
            range: 2..2,
            text: "c".into(),
        }],
        Some(7),
    );
    assert!(!b.can_redo());
}

#[test]
fn backspace_run_coalesces() {
    let mut b = Buffer::from_text("abcd");
    b.edit_grouped(
        &[Edit {
            range: 3..4,
            text: "".into(),
        }],
        Some(1),
    );
    b.edit_grouped(
        &[Edit {
            range: 2..3,
            text: "".into(),
        }],
        Some(1),
    );
    assert_eq!(b.text().to_string(), "ab");
    b.undo();
    assert_eq!(b.text().to_string(), "abcd");
}

fn grouped(b: &mut Buffer, range: std::ops::Range<usize>, text: &str, group: u64) {
    b.edit_grouped(
        &[Edit {
            range,
            text: text.into(),
        }],
        Some(group),
    );
}

/// A run mixing inserts and deletes must replay in order: sorting its steps
/// by offset (the old behavior) left "abd" behind here.
#[test]
fn mixed_typing_run_undoes_and_redoes_in_order() {
    let mut b = Buffer::from_text("ab");
    grouped(&mut b, 2..2, "c", 1);
    grouped(&mut b, 3..3, "d", 1);
    grouped(&mut b, 3..4, "", 1);
    grouped(&mut b, 2..3, "", 1);
    assert_eq!(b.text().to_string(), "ab");
    b.undo();
    assert_eq!(b.text().to_string(), "ab");
    b.redo();
    assert_eq!(b.text().to_string(), "ab");
}

/// A run whose keystrokes land at different places (caret moved mid-run).
#[test]
fn run_with_non_adjacent_edits_round_trips() {
    let mut b = Buffer::from_text("hello world");
    grouped(&mut b, 11..11, "!", 1);
    grouped(&mut b, 0..0, ">", 1);
    grouped(&mut b, 6..7, "", 1);
    let after = b.text().to_string();
    assert_eq!(after, ">helloworld!");
    b.undo();
    assert_eq!(b.text().to_string(), "hello world");
    b.redo();
    assert_eq!(b.text().to_string(), after);
}

/// Several edits in one call: inverse ranges must account for the shift
/// earlier edits cause.
#[test]
fn multi_edit_call_round_trips() {
    let mut b = Buffer::from_text("one two three");
    b.edit(&[
        Edit {
            range: 8..13,
            text: "3".into(),
        },
        Edit {
            range: 0..3,
            text: "first".into(),
        },
    ]);
    assert_eq!(b.text().to_string(), "first two 3");
    b.undo();
    assert_eq!(b.text().to_string(), "one two three");
    b.redo();
    assert_eq!(b.text().to_string(), "first two 3");
}

#[test]
fn clear_history_drops_undo_and_redo() {
    let mut b = Buffer::from_text("a");
    b.edit(&[Edit {
        range: 1..1,
        text: "b".into(),
    }]);
    b.edit(&[Edit {
        range: 2..2,
        text: "c".into(),
    }]);
    b.undo();
    assert!(b.can_undo() && b.can_redo());
    b.clear_history();
    assert!(!b.can_undo() && !b.can_redo());
    assert_eq!(b.text().to_string(), "ab");
}

/// "😀" is one char but two UTF-16 code units.
#[test]
fn utf16_offsets_convert_both_ways_and_clamp() {
    let b = Buffer::from_text("a😀b");
    assert_eq!(b.len_chars(), 3);
    assert_eq!(b.len_utf16(), 4);
    assert_eq!(b.char_to_utf16(0), 0);
    assert_eq!(b.char_to_utf16(1), 1);
    assert_eq!(b.char_to_utf16(2), 3);
    assert_eq!(b.char_to_utf16(3), 4);
    assert_eq!(b.char_to_utf16(99), 4);
    assert_eq!(b.utf16_to_char(0), 0);
    assert_eq!(b.utf16_to_char(1), 1);
    assert_eq!(b.utf16_to_char(2), 1); // inside the surrogate pair
    assert_eq!(b.utf16_to_char(3), 2);
    assert_eq!(b.utf16_to_char(4), 3);
    assert_eq!(b.utf16_to_char(99), 3);
}
