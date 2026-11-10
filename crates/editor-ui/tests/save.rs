//! Saving: dirty tracking, the default atomic on-disk write, the host save
//! handler, and the cmd/ctrl-s binding.

use editor_ui::{EditorEvent, EditorState, EditorView, SaveRequest, SaveStatus};
use gpui::{AppContext as _, Entity, TestAppContext, VisualTestContext};
use std::cell::RefCell;
use std::fs;
use std::path::PathBuf;
use std::rc::Rc;

/// A fresh, empty directory under the system temp dir, removed on drop.
struct TempDir(PathBuf);

impl TempDir {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "editor-ui-save-{name}-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("create temp dir");
        Self(dir)
    }

    fn file(&self, name: &str, content: &str) -> PathBuf {
        let path = self.0.join(name);
        fs::write(&path, content).expect("seed file");
        path
    }

    fn entries(&self) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(&self.0)
            .expect("read temp dir")
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// Collect every `EditorEvent` `state` emits.
fn events(cx: &mut TestAppContext, state: &Entity<EditorState>) -> Rc<RefCell<Vec<EditorEvent>>> {
    let log = Rc::new(RefCell::new(Vec::new()));
    let sink = log.clone();
    cx.update(|cx| {
        cx.subscribe(state, move |_, event: &EditorEvent, _| {
            sink.borrow_mut().push(event.clone())
        })
        .detach()
    });
    log
}

fn save(cx: &mut TestAppContext, state: &Entity<EditorState>) -> bool {
    let handled = state.update(cx, |s, cx| s.save(cx));
    cx.run_until_parked();
    handled
}

// -- dirty tracking -----------------------------------------------------------

#[gpui::test]
fn edits_dirty_the_buffer_and_undo_cleans_it(cx: &mut TestAppContext) {
    let state = cx.update(|cx| cx.new(|_| EditorState::editable("abc", None)));
    state.read_with(cx, |s, _| assert!(!s.is_dirty()));
    state.update(cx, |s, cx| s.type_text(3..3, "d", cx));
    state.read_with(cx, |s, _| assert!(s.is_dirty()));
    state.update(cx, |s, cx| s.undo(cx));
    state.read_with(cx, |s, _| assert!(!s.is_dirty()));
    state.update(cx, |s, cx| s.redo(cx));
    state.read_with(cx, |s, _| assert!(s.is_dirty()));
}

/// After a save, the saved text is the clean point: undoing past it is
/// dirty, redoing back to it is clean.
#[gpui::test]
fn the_clean_point_moves_to_the_saved_text(cx: &mut TestAppContext) {
    let dir = TempDir::new("clean-point");
    let path = dir.file("a.txt", "abc");
    let state = cx.update(|cx| cx.new(|_| EditorState::open(&path, None).unwrap()));
    state.update(cx, |s, cx| s.replace_range(3..3, "d", cx));
    assert!(save(cx, &state));
    state.read_with(cx, |s, _| assert!(!s.is_dirty()));
    state.update(cx, |s, cx| s.undo(cx));
    state.read_with(cx, |s, _| assert!(s.is_dirty()));
    state.update(cx, |s, cx| s.redo(cx));
    state.read_with(cx, |s, _| assert!(!s.is_dirty()));
}

#[gpui::test]
fn set_text_loads_a_clean_buffer(cx: &mut TestAppContext) {
    let state = cx.update(|cx| cx.new(|_| EditorState::editable("abc", None)));
    state.update(cx, |s, cx| {
        s.type_text(3..3, "d", cx);
        s.set_text("fresh", cx);
    });
    state.read_with(cx, |s, _| assert!(!s.is_dirty()));
}

// -- default on-disk save -----------------------------------------------------

#[gpui::test]
fn open_edit_save_round_trips_through_the_file(cx: &mut TestAppContext) {
    let dir = TempDir::new("round-trip");
    let path = dir.file("a.txt", "héllo\n");
    let state = cx.update(|cx| cx.new(|_| EditorState::open(&path, None).unwrap()));
    let log = events(cx, &state);
    state.read_with(cx, |s, _| {
        assert_eq!(s.text(), "héllo\n");
        assert_eq!(s.path(), Some(path.as_path()));
        assert!(!s.is_dirty());
    });

    state.update(cx, |s, cx| s.type_text(5..5, " wörld 🎉", cx));
    assert!(save(cx, &state));
    assert_eq!(fs::read_to_string(&path).unwrap(), "héllo wörld 🎉\n");
    state.read_with(cx, |s, _| assert!(!s.is_dirty()));
    assert_eq!(*log.borrow(), vec![EditorEvent::Saved]);
    // The temporary sibling is gone: only the saved file remains.
    assert_eq!(dir.entries(), vec!["a.txt".to_string()]);
}

/// Line endings are written back exactly as loaded.
#[gpui::test]
fn crlf_line_endings_survive_a_save(cx: &mut TestAppContext) {
    let dir = TempDir::new("crlf");
    let path = dir.file("a.txt", "one\r\ntwo\r\n");
    let state = cx.update(|cx| cx.new(|_| EditorState::open(&path, None).unwrap()));
    state.update(cx, |s, cx| s.type_text(0..0, "x", cx));
    assert!(save(cx, &state));
    assert_eq!(fs::read(&path).unwrap(), b"xone\r\ntwo\r\n");
}

#[gpui::test]
fn save_creates_a_file_that_does_not_exist_yet(cx: &mut TestAppContext) {
    let dir = TempDir::new("create");
    let path = dir.0.join("new.txt");
    let state =
        cx.update(|cx| cx.new(|_| EditorState::editable("brand new", None).with_path(&path)));
    assert!(save(cx, &state));
    assert_eq!(fs::read_to_string(&path).unwrap(), "brand new");
}

#[cfg(unix)]
#[gpui::test]
fn save_keeps_permissions_and_follows_symlinks(cx: &mut TestAppContext) {
    use std::os::unix::fs::PermissionsExt;
    let dir = TempDir::new("unix");
    let real = dir.file("real.sh", "old");
    fs::set_permissions(&real, fs::Permissions::from_mode(0o750)).unwrap();
    let link = dir.0.join("link.sh");
    std::os::unix::fs::symlink(&real, &link).unwrap();

    let state = cx.update(|cx| cx.new(|_| EditorState::open(&link, None).unwrap()));
    state.update(cx, |s, cx| s.replace_range(0..3, "new", cx));
    assert!(save(cx, &state));

    assert_eq!(fs::read_to_string(&real).unwrap(), "new");
    assert!(fs::symlink_metadata(&link).unwrap().file_type().is_symlink());
    assert_eq!(fs::metadata(&real).unwrap().permissions().mode() & 0o777, 0o750);
}

/// A failed write reports the error, leaves the buffer dirty, and leaves
/// no temporary file behind.
#[gpui::test]
fn a_failed_save_emits_an_error_and_stays_dirty(cx: &mut TestAppContext) {
    let dir = TempDir::new("failure");
    let path = dir.0.join("missing-dir").join("a.txt");
    let state = cx.update(|cx| cx.new(|_| EditorState::editable("abc", None).with_path(&path)));
    let log = events(cx, &state);
    state.update(cx, |s, cx| s.type_text(3..3, "d", cx));
    assert!(save(cx, &state));
    state.read_with(cx, |s, _| assert!(s.is_dirty()));
    assert!(matches!(log.borrow().as_slice(), [EditorEvent::SaveFailed(_)]));
    assert!(dir.entries().is_empty());
}

// -- nothing to save with -----------------------------------------------------

#[gpui::test]
fn save_without_a_path_or_handler_does_nothing(cx: &mut TestAppContext) {
    let state = cx.update(|cx| cx.new(|_| EditorState::editable("abc", None)));
    let log = events(cx, &state);
    state.update(cx, |s, cx| s.type_text(3..3, "d", cx));
    assert!(!save(cx, &state));
    state.read_with(cx, |s, _| assert!(s.is_dirty()));
    assert!(log.borrow().is_empty());
}

#[gpui::test]
fn readonly_never_saves(cx: &mut TestAppContext) {
    let dir = TempDir::new("readonly");
    let path = dir.file("a.txt", "on disk");
    let state =
        cx.update(|cx| cx.new(|_| EditorState::readonly("in memory", None).with_path(&path)));
    assert!(!save(cx, &state));
    assert_eq!(fs::read_to_string(&path).unwrap(), "on disk");
}

// -- save handler -------------------------------------------------------------

/// A handler replaces the default write entirely and receives the text,
/// path, and revision.
#[gpui::test]
fn a_save_handler_takes_over_from_the_default_write(cx: &mut TestAppContext) {
    let dir = TempDir::new("handler");
    let path = dir.file("a.txt", "on disk");
    let state = cx.update(|cx| cx.new(|_| EditorState::open(&path, None).unwrap()));
    let requests: Rc<RefCell<Vec<SaveRequest>>> = Rc::default();
    let sink = requests.clone();
    state.update(cx, |s, cx| {
        s.set_save_handler(Some(Box::new(move |request| {
            sink.borrow_mut().push(request);
            Ok(SaveStatus::Saved)
        })));
        s.replace_range(0..7, "edited", cx);
    });
    assert!(save(cx, &state));

    let requests = requests.borrow();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].text, "edited");
    assert_eq!(requests[0].path.as_deref(), Some(path.as_path()));
    assert_eq!(requests[0].revision, state.read_with(cx, |s, _| s.revision()));
    state.read_with(cx, |s, _| assert!(!s.is_dirty()));
    // The editor itself never touched the file.
    assert_eq!(fs::read_to_string(&path).unwrap(), "on disk");
}

/// A deferred save stays dirty until the host reports the revision saved;
/// edits made while it was in flight keep the buffer dirty.
#[gpui::test]
fn a_deferred_save_is_completed_by_mark_saved(cx: &mut TestAppContext) {
    let state = cx.update(|cx| cx.new(|_| EditorState::editable("abc", None)));
    let log = events(cx, &state);
    let pending: Rc<RefCell<Option<u64>>> = Rc::default();
    let sink = pending.clone();
    state.update(cx, |s, cx| {
        s.set_save_handler(Some(Box::new(move |request| {
            *sink.borrow_mut() = Some(request.revision);
            Ok(SaveStatus::Deferred)
        })));
        s.replace_range(3..3, "d", cx);
    });
    assert!(save(cx, &state));
    state.read_with(cx, |s, _| assert!(s.is_dirty()));
    assert!(log.borrow().is_empty());

    // An edit lands while the save is still in flight.
    state.update(cx, |s, cx| s.replace_range(4..4, "e", cx));
    let revision = pending.borrow().expect("handler ran");
    state.update(cx, |s, cx| s.mark_saved(revision, cx));
    cx.run_until_parked();
    assert_eq!(*log.borrow(), vec![EditorEvent::Saved]);
    state.read_with(cx, |s, _| assert!(s.is_dirty()));

    // Undoing that edit returns to exactly what was saved.
    state.update(cx, |s, cx| s.undo(cx));
    state.read_with(cx, |s, _| assert!(!s.is_dirty()));
}

#[gpui::test]
fn a_failing_save_handler_reports_its_error(cx: &mut TestAppContext) {
    let state = cx.update(|cx| cx.new(|_| EditorState::editable("abc", None)));
    let log = events(cx, &state);
    state.update(cx, |s, cx| {
        s.set_save_handler(Some(Box::new(|_| Err(std::io::Error::other("disk on fire")))));
        s.replace_range(3..3, "d", cx);
    });
    assert!(save(cx, &state));
    assert_eq!(
        *log.borrow(),
        vec![EditorEvent::SaveFailed("disk on fire".to_string())]
    );
    state.read_with(cx, |s, _| assert!(s.is_dirty()));
}

// -- key binding --------------------------------------------------------------

fn open_view<'a>(
    cx: &'a mut TestAppContext,
    state: EditorState,
) -> (Entity<EditorState>, &'a mut VisualTestContext) {
    cx.update(editor_ui::init);
    let state = cx.update(|cx| cx.new(|_| state));
    let view_state = state.clone();
    let (view, cx) = cx.add_window_view(move |_window, cx| EditorView::new(&view_state, cx));
    let handle = view.read_with(cx, |v, _| v.focus_handle());
    cx.update(|window, cx| window.focus(&handle, cx));
    cx.run_until_parked();
    (state, cx)
}

#[gpui::test]
fn cmd_s_and_ctrl_s_save_the_focused_editor(cx: &mut TestAppContext) {
    let dir = TempDir::new("keys");
    let path = dir.file("a.txt", "");
    let (state, cx) = open_view(cx, EditorState::open(&path, None).unwrap());
    cx.simulate_input("one");
    cx.simulate_keystrokes("cmd-s");
    assert_eq!(fs::read_to_string(&path).unwrap(), "one");
    state.read_with(cx, |s, _| assert!(!s.is_dirty()));

    cx.simulate_input("two");
    state.read_with(cx, |s, _| assert!(s.is_dirty()));
    cx.simulate_keystrokes("ctrl-s");
    assert_eq!(fs::read_to_string(&path).unwrap(), "onetwo");
    state.read_with(cx, |s, _| assert!(!s.is_dirty()));
}
