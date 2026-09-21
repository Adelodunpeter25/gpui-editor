# todo.md

Editor roadmap in dependency order — each item needs the one above it.
The goal: edit code inside the desktop app without leaving for an external
editor. The viewer (what ships today) keeps working exactly as-is throughout.

## 1. Edit mode

The editor is a readonly viewer today: `Mode::Editable` exists as a reserved
variant but nothing implements it — no key ever reaches the buffer (only
selection, copy, and select-all are wired), and there is no visible caret.
`EditorState::insert` exists as the edit primitive but nothing calls it.

- Typing: printable characters, Enter, Backspace/Delete — gated strictly on
  `Mode::Editable`, so readonly tabs keep today's behavior byte for byte
- Caret: render it at the selection head (empty selection = a visible line),
  with arrow-key / Home / End motion; click-to-place already computes the
  offset, it just has nothing to show
- Surface undo/redo: `edit-buffer` already has bounded undo/redo (readonly
  constructors disable it) — `EditorState` just doesn't expose it yet;
  wire it to cmd-z / cmd-shift-z
- Verify horizontal scroll stays healthy while editing: the content-width
  and wrap caches key off buffer version, so they should invalidate per
  keystroke — confirm it, don't assume it
- Cut a release rev when green (the desktop app consumes the editor by
  pinned rev, not path)

## 2. Vim mode — minimal Normal/Insert

Only after edit mode: vim mode needs a buffer that can actually take input.
Exactly the toggle, nothing else:

- Files open in Normal mode
- `i` → Insert: keys go to the buffer
- `Esc` → Normal: keys stop reaching the buffer
- Current mode visible (status area in the desktop app)
- Every other Normal-mode key is ignored for now

Later, once the toggle is solid: motions (`h j k l`, `w`/`b`, `0`/`$`),
visual mode, counts, and operators (`d`/`c`/`y`).

## 3. Ship to the desktop app

- Bump the `editor-ui` pin in `console` to the rev containing 1–2
- The diff view stays readonly — editing diffs is explicitly out of scope
  until real diff-editing exists
