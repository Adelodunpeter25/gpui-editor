//! Saving: the vocabulary a host sees (`SaveRequest`, `SaveStatus`,
//! `EditorEvent`) and the default on-disk writer. `EditorState::save` in
//! `state.rs` decides *whether* and *who* saves; this file is the how.

use edit_buffer::LineEnding;
use ropey::Rope;
use std::fs::{self, File};
use std::io::{self, BufWriter, Write};
use std::path::{Path, PathBuf};

/// What a save handler is asked to persist.
#[derive(Debug, Clone)]
pub struct SaveRequest {
    /// Full buffer text at the time of the save, with the document's line
    /// ending applied — the exact bytes to persist.
    pub text: String,
    /// The file the editor is bound to, if any.
    pub path: Option<PathBuf>,
    /// Content revision being saved. A handler that finishes later reports
    /// back with `EditorState::mark_saved(revision)`, so edits made in the
    /// meantime still read as unsaved.
    pub revision: u64,
}

/// What a save handler did with a `SaveRequest`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SaveStatus {
    /// The text is persisted; the editor marks itself clean now.
    Saved,
    /// The handler will finish later (remote write, save-as dialog) and
    /// call `EditorState::mark_saved` itself. The editor stays dirty.
    Deferred,
}

/// Custom save hook; see `EditorState::set_save_handler`.
pub type SaveHandler = Box<dyn Fn(SaveRequest) -> io::Result<SaveStatus>>;

/// Events `EditorState` emits; subscribe with `cx.subscribe(&state, …)`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EditorEvent {
    /// The buffer was saved and is now clean.
    Saved,
    /// A save was attempted and failed; the buffer is still dirty.
    SaveFailed(String),
}

/// Write `text` (holding `\n` breaks) to `path` with `ending` applied,
/// without ever leaving a half-written file behind:
/// the bytes go to a temporary sibling first, which then replaces the
/// target in one rename. Symlinks are followed (the link survives, its
/// target is rewritten) and an existing file keeps its permissions.
pub(crate) fn write_atomic(path: &Path, text: &Rope, ending: LineEnding) -> io::Result<()> {
    let target = fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let file_name = target
        .file_name()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "path has no file name"))?;
    let mut temp_name = std::ffi::OsString::from(".");
    temp_name.push(file_name);
    temp_name.push(format!(".{}.tmp", std::process::id()));
    let temp = target.with_file_name(temp_name);

    let written = (|| {
        let mut writer = BufWriter::new(File::create(&temp)?);
        for chunk in text.chunks() {
            writer.write_all(ending.apply(chunk).as_bytes())?;
        }
        let file = writer.into_inner().map_err(|e| e.into_error())?;
        file.sync_all()?;
        if let Ok(existing) = fs::metadata(&target) {
            fs::set_permissions(&temp, existing.permissions())?;
        }
        fs::rename(&temp, &target)
    })();
    if written.is_err() {
        let _ = fs::remove_file(&temp);
    }
    written
}
