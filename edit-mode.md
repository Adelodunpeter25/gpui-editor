# Edit mode — spec (todo.md item 2)

The editor is a readonly viewer today. This milestone makes `Mode::Editable`
real: typing, a visible caret, cursor motion, cut/paste, and wired undo/redo —
with readonly behavior staying byte-for-byte identical.

Verified current state (what the spec builds on):

- `Mode::Editable` and `EditorState::editable()` exist as reserved variants —
  nothing implements them. Only selection, copy, select-all are wired.
- `EditorState::insert(offset, text, cx)` exists (gated on `!is_readonly()`),
  rehighlights, reruns search — nothing calls it.
- `Buffer` has bounded undo/redo (100 entries, one entry per `edit()` call;
  `from_text_readonly` disables history). `EditorState` does not expose it.
- `EditorState::indent_for_newline(row)` exists — unused. Click-to-place
  already computes the char offset — there is just no caret to show.
- The view handles keys via GPUI actions + `on_key_down` (escape only) and
  renders rows through a virtualized `uniform_list`; content-width and wrap
  caches key off buffer version.

## 1. Goals

1. Typing works in `Mode::Editable`: printable characters, Enter,
   Backspace/Delete, Tab. Readonly tabs: zero behavior change.
2. Caret rendered at the selection head (empty selection = a visible caret);
   arrow/Home/End/Up/Down motion; click-to-place caret already works via the
   existing selection hit-testing.
3. Undo/redo wired end to end: `cmd-z` / `cmd-shift-z`, bounded history the
   buffer already keeps.
4. Cut (`cmd-x`) and paste (`cmd-v`) join copy in the clipboard actions.
5. Editing a buffer stays correct with word wrap, horizontal scroll, search
   highlights, and background rehighlighting for large files.

## 2. Non-goals (explicitly out)

- **Diff views stay readonly.** `EditableDiff` stays a reserved variant;
  editing diffs waits for real diff-editing (todo.md §4).
- **No vim mode** — that is item 3 and consumes this milestone.
- **No multi-cursor, no auto-pairs/bracket-close, no IME composition** —
  deferred; noted below where the design leaves room.
- **No file save / dirty tracking** — the host app owns persistence. The
  editor exposes enough state (`version()`, `text()`) for a host to diff.
- **No search overlay UI** — model methods exist; wiring the overlay is its
  own milestone (M3 per state.rs notes).

## 3. Design decisions

### 3.1 Input path: keystroke actions, not `InputHandler` (for v1)

GPUI's `InputHandler` trait is the IME/replacement-text path; v1 uses plain
bound actions + one `on_key_down` for printable input, matching how
Copy/SelectAll/CancelComment already work:

- New actions: `Input` (carries the char), `Backspace`, `Delete`, `Enter`,
  `Tab`, `Undo`, `Redo`, `Cut`, `Paste`, and movement actions
  (`MoveLeft/Right/Up/Down`, `MoveToLineStart/End`).
- Printable keys: `on_key_down` on the focused editor forwards
  `keystroke.key`/`keystroke.ime` to the buffer when the key is a single
  printable char and mode is editable; otherwise ignores it (so existing
  bindings like escape/comment-drag keep first claim).
- `init(cx)` gains the new `bind_keys` entries; `editor_ui::init` stays the
  only setup call hosts make.

IME (`replace_text_in_range` etc. via `InputHandler`) is a follow-up; the
edit primitive this milestone adds (§3.3) is the same one IME would call, so
that work layers on without rework.

### 3.2 Edit primitive: `replace_range` on `EditorState`

`insert` covers only insertion and bypasses selection handling. Add one
canonical primitive and make everything else call it:

```rust
pub fn replace_range(&mut self, range: Range<usize>, text: &str, cx: &mut Context<Self>)
```

- No-op in readonly (same guard `insert` has).
- One `buffer.edit` → one version bump → one undo entry → rehighlight →
  rerun search → move selection to the end of inserted text (collapse to
  caret). `insert` becomes `replace_range(offset..offset, …)`.
- Callers that want grouped undo (paste, auto-indent runs) pass their edits
  in one call; see §3.4.

### 3.3 Keystroke semantics

| Key | Behavior (editable mode only) |
|---|---|
| printable char | `replace_range(head..head, c)` — replaces selection if non-empty |
| Enter | `replace_range(sel, "\n" + indent_for_newline(row))` — uses the existing helper |
| Backspace | delete char before caret, or replace selection with `""` |
| Delete | delete char at caret, or replace selection with `""` |
| Tab | insert `IndentOptions`-correct text (spaces/hard tabs); no focus change |
| arrows/Home/End | move caret (§3.5), extend selection with Shift |
| `cmd-z` / `cmd-shift-z` | undo / redo (§3.4) |
| `cmd-x` / `cmd-v` | cut: copy selection then delete; paste: `replace_range(sel, clipboard)` |

Up/Down keep a goal column (the x position the caret last aimed at) so
moving across a short line and back doesn't drift — stored on `EditorView`,
not in the model.

### 3.4 Undo grouping

`Buffer` pushes one undo entry per `edit()`; naive typing gives one entry per
keystroke, so undo becomes per-character. Add grouping at the `EditorState`
layer: consecutive single-char inserts/deletes within a short window (~500ms
or until a non-typing edit) coalesce into one undo unit. Paste/Enter/cut
always close the current group. If `Buffer` needs a
`edit_grouped(edits, group_id)` seam for this, that stays inside edit-buffer
with integration tests in `crates/edit-buffer/tests/`.

### 3.5 Caret rendering

- Empty selection ⇒ render a 2px vertical caret at the head offset's pixel x,
  clipped to the row; non-empty selection keeps today's highlight only.
- The caret lives in the row paint path in `lib.rs` (only visible rows paint;
  a caret on a scrolled-away row simply isn't painted).
- Wrap-aware: caret x derives from the same wrap-cache mapping clicks use.
- No blink in v1 (GPUI timers + focus tracking for it is its own follow-up);
  the caret is always-on while focused. Escape/blur hides it.

### 3.6 Cache invalidation — verify, don't assume

`content_width_cache` and `WrapCache` key off buffer version, so a keystroke
bumping the version should invalidate both. Verification is part of the
milestone, not an assumption: the horizontal-scroll integration tests gain an
editable-mode case (type a longer line than any existing one, assert the
content width and scroll range grow; undo, assert they shrink back).

### 3.7 Rehighlight cost per keystroke

`insert` already rehighlights on every call: synchronous under 64KB, background
with a version-guard above it. Editable mode keeps that exact path for v1.
Known tradeoff, stated here on purpose: a large file rehighlights the whole
buffer per keystroke (backgrounded, stale results discarded, but still O(file)
per key). If that's noticeable in the demo app, the follow-up is
incremental/spans-only-reparse — out of scope here.

## 4. Files touched

- `crates/editor-ui/src/types.rs` — no new vocabulary types expected;
  movement actions are GPUI action structs in `lib.rs` alongside Copy.
- `crates/editor-ui/src/state.rs` — `replace_range`, undo/redo exposure
  (`can_undo/can_redo/undo/redo`), caret-goal-column helper on the model.
- `crates/editor-ui/src/view.rs` — key handling, new actions, goal column.
- `crates/editor-ui/src/lib.rs` — caret paint, `init` key bindings,
  printable-key forward.
- `crates/edit-buffer/src/types.rs` — only if undo grouping needs a seam (§3.4).
- `crates/editor-demo` — smoke path: open a file editable, type, undo.

## 5. Tests (each crate's `tests/` only, per AGENTS.md)

- `crates/editor-ui/tests/editor_state.rs` — `replace_range` semantics:
  selection replacement, collapse, readonly no-op, undo/redo round-trip,
  undo grouping, Enter auto-indent, version/cache invalidation.
- `crates/edit-buffer/tests/buffer.rs` — grouping seam (if added).
- `crates/editor-ui/tests/horizontal_scroll.rs` — editable-mode width
  invalidation (§3.6).
- Existing tests must stay green unchanged — readonly is not allowed to move.

## 6. Milestone checklist

1. [ ] `replace_range` + undo/redo exposure on `EditorState` (model-only, tests)
2. [ ] Key bindings + actions; typing/Enter/Backspace/Delete/Tab reachable
3. [ ] Caret render + arrow/Home/End/Up/Down motion with goal column
4. [ ] Cut/paste; undo/redo wired to keys
5. [ ] Scroll/wrap cache verification tests (§3.6)
6. [ ] Demo app editable smoke; cut a release rev (desktop app consumes by pinned rev)

## 7. Open questions (answer before/while building)

1. Undo grouping window: time-based (~500ms) is simplest — good enough, or
   does the desktop app need manual grouping (e.g. group per sentence)?
2. Caret blink: acceptable to defer, or required for the desktop app?
3. Should `EditorState::editable()` gain a way to flip an existing state from
   readonly→editable (`set_mode`), or is mode fixed at construction?
