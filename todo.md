# todo.md

Editor roadmap, ordered by what to build first. Vim (3) needs edit mode (2);
gutter annotate (1) is independent viewer work — smallest win first, no
editing required. The goal stays: work in code inside the desktop app
without leaving for an external editor. The viewer (what ships today) keeps
working exactly as-is throughout.

## 1. Gutter annotate — `+` comment for the AI

GitHub's code-review pattern applied to "send this code to the AI with a
note": hover the gutter, get a `+`, pick a range, type the comment. Pure
viewer work — annotation never touches the buffer, so it neither blocks nor
waits on items 2–3, and readonly stays readonly.

- Hover a row's line-number gutter → a `+` chip appears in it. It rides the
  row, so it scrolls away with h-scroll exactly like the line numbers do —
  fine for v1, keyboard alternative later
- Click `+` with a text selection → annotate that selection (the selection
  *is* the range; show a `LINES 3–20`-style label). No selection → current
  line
- Highlight the range full-width while composing — same paint path as the
  existing selection and diff row backgrounds
- Composer is a floating overlay anchored under the range's last line, NOT
  an inserted row: `uniform_list` requires uniform row heights, so pushing
  lines down the way GitHub does would break virtualization. Anchor to the
  row's on-screen position; handle scroll-away/unmount while it's open
- Esc cancels, Enter submits (`↵` hint on the button)
- After posting, leave a persistent marker on the range (GitHub's line
  badge equivalent)
- Ownership split: hover/`+`/range-highlight/anchor+focus mechanics live in
  editor-ui as generic primitives — the editor crate must not know "AI"
  exists; the composer content (textarea, send-to-AI, persistence) lives in
  console, which already has gpui-component input and the AI plumbing
- Deliberately deferred: reply threads, several composers per file,
  range-adjust arrows, a keyboard "+ at cursor" shortcut

## 2. Edit mode

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

## 3. Vim mode — minimal Normal/Insert

Only after edit mode: vim mode needs a buffer that can actually take input.
Exactly the toggle, nothing else:

- Files open in Normal mode
- `i` → Insert: keys go to the buffer
- `Esc` → Normal: keys stop reaching the buffer
- Current mode visible (status area in the desktop app)
- Every other Normal-mode key is ignored for now

Later, once the toggle is solid: motions (`h j k l`, `w`/`b`, `0`/`$`),
visual mode, counts, and operators (`d`/`c`/`y`).

## 4. Ship to the desktop app

- Each feature ships the same way: bump the `editor-ui` pin in `console` to
  the rev containing it
- The diff view stays readonly — editing diffs is explicitly out of scope
  until real diff-editing exists
