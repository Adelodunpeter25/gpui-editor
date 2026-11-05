//! Edit-mode input: the editor's actions, their default key bindings, and
//! the `on_key_down` router that turns keystrokes into buffer edits and
//! caret motion. The `Render` impl (`render.rs`) wires `handle_edit_key` in;
//! the model side of every edit lives in `EditorState`.

use gpui::*;

use std::cell::Cell;
use std::rc::Rc;

use edit_buffer::Point;

use crate::state::EditorState;

actions!(
    editor_ui,
    [Copy, SelectAll, CancelComment, Undo, Redo, Cut, Paste]
);

/// Bind default keys for `editor_ui`'s actions. Call once at app startup.
pub fn init(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("cmd-c", Copy, Some("Editor")),
        KeyBinding::new("ctrl-c", Copy, Some("Editor")),
        KeyBinding::new("cmd-a", SelectAll, Some("Editor")),
        KeyBinding::new("ctrl-a", SelectAll, Some("Editor")),
        KeyBinding::new("cmd-z", Undo, Some("Editor")),
        KeyBinding::new("shift-cmd-z", Redo, Some("Editor")),
        KeyBinding::new("ctrl-z", Undo, Some("Editor")),
        KeyBinding::new("shift-ctrl-z", Redo, Some("Editor")),
        KeyBinding::new("cmd-x", Cut, Some("Editor")),
        KeyBinding::new("ctrl-x", Cut, Some("Editor")),
        KeyBinding::new("cmd-v", Paste, Some("Editor")),
        KeyBinding::new("ctrl-v", Paste, Some("Editor")),
    ]);
}

/// Route one key event to the buffer in editable mode. Editing keys are
/// handled on raw `on_key_down` rather than as bound actions: printable
/// input has no stable binding name, and motion needs no payload. The
/// `cmd`-combos (undo/redo/cut/paste/copy) stay bound actions. No-op when
/// the editor isn't focused or is readonly — readonly tabs keep the viewer
/// behavior byte for byte.
pub(crate) fn handle_edit_key(
    event: &KeyDownEvent,
    focus_handle: &FocusHandle,
    state: &Entity<EditorState>,
    key_anchor: &Rc<Cell<Option<usize>>>,
    goal_col: &Rc<Cell<Option<u32>>>,
    window: &mut Window,
    cx: &mut App,
) {
    if !focus_handle.is_focused(window) || state.read(cx).is_readonly() {
        return;
    }
    let key = event.keystroke.key.as_str();
    let shift = event.keystroke.modifiers.shift;
    match key {
        "left" | "right" | "up" | "down" | "home" | "end" => {
            cx.stop_propagation();
            window.prevent_default();
            move_caret(state, key_anchor, goal_col, key, shift, cx);
        }
        "backspace" | "delete" | "enter" | "tab" => {
            cx.stop_propagation();
            window.prevent_default();
            key_anchor.set(None);
            goal_col.set(None);
            state.update(cx, |s, cx| {
                let sel = s.selection().range.clone();
                match key {
                    "backspace" => {
                        if sel.is_empty() {
                            if sel.start > 0 {
                                s.type_text(sel.start - 1..sel.start, "", cx);
                            }
                        } else {
                            s.type_text(sel, "", cx);
                        }
                    }
                    "delete" => {
                        if sel.is_empty() {
                            let end = s.len_chars();
                            if sel.end < end {
                                s.type_text(sel.end..sel.end + 1, "", cx);
                            }
                        } else {
                            s.type_text(sel, "", cx);
                        }
                    }
                    "enter" => {
                        // Shift+Enter inserts a plain newline (no auto-indent).
                        if shift {
                            s.replace_range(sel, "\n", cx);
                        } else {
                            let (range, text) = if sel.is_empty() {
                                s.newline_edit(sel.start)
                            } else {
                                (sel, "\n".to_string())
                            };
                            s.replace_range(range, &text, cx);
                        }
                    }
                    "tab" => {
                        let indent = s.indent_options();
                        let text = if indent.hard_tabs {
                            "\t".to_string()
                        } else {
                            " ".repeat(indent.tab_width as usize)
                        };
                        s.replace_range(sel, &text, cx);
                    }
                    _ => unreachable!("matched above"),
                }
            });
        }
        _ => {
            // Printable input: `key_char` is the typed character (None for
            // modifier combos and control keys), already shifted-correct.
            if event.keystroke.modifiers.control
                || event.keystroke.modifiers.platform
                || event.keystroke.modifiers.function
            {
                return;
            }
            let Some(text) = event.keystroke.key_char.as_deref().filter(|s| !s.is_empty()) else {
                return;
            };
            let text = text.to_string();
            cx.stop_propagation();
            key_anchor.set(None);
            goal_col.set(None);
            state.update(cx, |s, cx| {
                let sel = s.selection().range.clone();
                s.type_text(sel, &text, cx);
            });
        }
    }
}

/// Move (or extend, with shift) the selection for one motion key. The
/// anchor/goal bookkeeping lives on the view (not the model): `key_anchor`
/// pins where a shift-drag started, `goal_col` remembers the column an
/// up/down is aiming at across short lines.
fn move_caret(
    state: &Entity<EditorState>,
    key_anchor: &Rc<Cell<Option<usize>>>,
    goal_col: &Rc<Cell<Option<u32>>>,
    key: &str,
    shift: bool,
    cx: &mut App,
) {
    state.update(cx, |s, cx| {
        let sel = s.selection().range.clone();
        let horizontal = matches!(key, "left" | "right" | "home" | "end");
        if horizontal && !shift {
            key_anchor.set(None);
            goal_col.set(None);
        }

        // Which end the keyboard is dragging (`head`) and which end stays
        // put (`opposite`). A stored anchor decides; without one (mouse-made
        // selection, or a caret) assume a forward selection, head at `end`.
        let (head, opposite) = if sel.is_empty() {
            (sel.start, sel.start)
        } else {
            match key_anchor.get() {
                Some(a) if a == sel.start => (sel.end, sel.start),
                Some(a) if a == sel.end => (sel.start, sel.end),
                _ => (sel.end, sel.start),
            }
        };

        let (new_head, new_anchor, new_goal) = match key {
            "left" => {
                if shift {
                    (head.saturating_sub(1), Some(opposite), goal_col.get())
                } else if sel.is_empty() {
                    (head.saturating_sub(1), None, None)
                } else {
                    (sel.start, None, None)
                }
            }
            "right" => {
                let len = s.len_chars();
                if shift {
                    ((head + 1).min(len), Some(opposite), goal_col.get())
                } else if sel.is_empty() {
                    ((head + 1).min(len), None, None)
                } else {
                    (sel.end, None, None)
                }
            }
            "home" | "end" => {
                let row = s.offset_to_point(head).row;
                let target = if key == "home" {
                    s.row_start_offset(row)
                } else {
                    s.line_end_offset(row)
                };
                if shift {
                    (target, Some(opposite), None)
                } else {
                    (target, None, None)
                }
            }
            "up" | "down" => {
                // Aim at the remembered column; remember it on the first
                // vertical move of a run.
                let goal = goal_col.get().unwrap_or(s.offset_to_point(head).col);
                let row = s.offset_to_point(head).row;
                let (target_row, at_edge) = if key == "up" {
                    (row.saturating_sub(1), row == 0)
                } else {
                    (row + 1, row + 1 >= s.line_count())
                };
                let head_new = if at_edge {
                    if key == "up" {
                        0
                    } else {
                        s.len_chars()
                    }
                } else {
                    let row_start = s.row_start_offset(target_row);
                    let max_col = (s.line_end_offset(target_row) - row_start) as u32;
                    s.point_to_offset(Point {
                        row: target_row,
                        col: goal.min(max_col),
                    })
                };
                if shift {
                    (head_new, Some(opposite), Some(goal))
                } else {
                    (head_new, None, Some(goal))
                }
            }
            _ => return,
        };

        let range = match new_anchor {
            Some(anchor) => new_head.min(anchor)..new_head.max(anchor),
            None => new_head..new_head,
        };
        s.set_selection(range);
        key_anchor.set(new_anchor);
        goal_col.set(new_goal);
        cx.notify();
    });
}

