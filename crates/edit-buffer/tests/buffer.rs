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
