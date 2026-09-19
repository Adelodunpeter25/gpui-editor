//! `DemoApp`: the demo window's top-level state. Its `Render` impl and
//! `main()` stay in `main.rs`, next to the actions/menus/keybindings that
//! drive them.

use editor_ui::{DiffState, DiffView, EditorState, EditorView};
use gpui::*;
use std::fs;
use std::path::PathBuf;
use syntax::LanguageRegistry;

use crate::INITIAL_SAMPLE;

pub(crate) struct DemoApp {
    pub(crate) state: Entity<EditorState>,
    pub(crate) view: Entity<EditorView>,
    /// Diffs `INITIAL_SAMPLE` (old) against whatever's currently open (new)
    /// — refreshed just before `showing_diff` flips on, not live-tracked,
    /// since there's no edit mode yet to make "live" meaningful.
    pub(crate) diff_state: Entity<DiffState>,
    pub(crate) diff_view: Entity<DiffView>,
    pub(crate) showing_diff: bool,
    pub(crate) file_path: Option<PathBuf>,
    pub(crate) focus_handle: FocusHandle,
}

impl DemoApp {
    pub(crate) fn new(cx: &mut Context<Self>) -> Self {
        let registry = LanguageRegistry::builtin();
        let state = cx.new(|_| EditorState::readonly(INITIAL_SAMPLE, registry.for_name("rust")));
        let view = cx.new(|cx| EditorView::new(&state, cx));
        let diff_state =
            cx.new(|_| DiffState::new(INITIAL_SAMPLE, INITIAL_SAMPLE, registry.for_name("rust")));
        let diff_view = cx.new(|_| DiffView::new(&diff_state));
        let focus_handle = cx.focus_handle();

        Self {
            state,
            view,
            diff_state,
            diff_view,
            showing_diff: false,
            file_path: None,
            focus_handle,
        }
    }

    pub(crate) fn toggle_wrap(&mut self, cx: &mut Context<Self>) {
        let enabled = self.state.read(cx).wrap_enabled();
        self.state.update(cx, |editor, _cx| editor.set_wrap_enabled(!enabled));
        cx.notify();
    }

    pub(crate) fn toggle_diff(&mut self, cx: &mut Context<Self>) {
        if !self.showing_diff {
            let (new_text, language) = {
                let editor = self.state.read(cx);
                (editor.text(), editor.language())
            };
            self.diff_state.update(cx, |diff, _cx| {
                diff.set_texts(INITIAL_SAMPLE, &new_text);
                diff.set_language(language);
            });
        }
        self.showing_diff = !self.showing_diff;
        cx.notify();
    }

    pub(crate) fn open_file(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        if let Some(path) = rfd::FileDialog::new().pick_file() {
            if let Ok(content) = fs::read_to_string(&path) {
                let registry = LanguageRegistry::builtin();
                let lang = LanguageRegistry::for_path(&path).or_else(|| {
                    path.extension()
                        .and_then(|ext| ext.to_str())
                        .and_then(|ext| registry.for_extension(ext))
                });

                self.state.update(cx, |editor, _cx| {
                    editor.set_text(&content);
                    editor.set_language(lang);
                });

                self.file_path = Some(path);
                cx.notify();
            }
        }
    }
}
