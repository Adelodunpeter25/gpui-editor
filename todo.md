# todo.md

Editor roadmap, ordered by what to build first. Vim (3) needs edit mode (2);
gutter annotate (1) is independent viewer work — smallest win first, no
editing required. The goal stays: work in code inside the desktop app
without leaving for an external editor. The viewer (what ships today) keeps
working exactly as-is throughout.

## 1. Gutter annotate — `+` comment for the AI (editor-ui primitives done)

GitHub's code-review pattern applied to "send this code to the AI with a
note": hover the gutter, get a `+`, pick a range, type the comment. Pure
viewer work — annotation never touches the buffer, so it neither blocks nor
waits on items 2–3, and readonly stays readonly.

Done in editor-ui: hover `+` chip riding the gutter, drag-to-select a
contiguous range (single line = `start_line == end_line`), range lit while
composing and after release, Esc cancels, one `CommentTarget` callback per
post, and a scroll-correct `CommentAnchor` for the host to place a
composer card under the range's last line (no inserted row —
`uniform_list` needs uniform row heights).

Still open:
- Composer content (textarea, send-to-AI, persistence) lives in console,
  which already has gpui-component input and the AI plumbing
- Ownership split: editor-ui stays generic — the editor crate must not
  know "AI" exists
- Deliberately deferred: reply threads, several composers per file,
  range-adjust arrows, a keyboard "+ at cursor" shortcut

## 2. Edit mode — DONE (except release rev)

Shipped: printable typing, Enter (auto-indent) / Backspace / Delete / Tab,
a visible caret at the selection head, arrow / Home / End motion with a
goal column, click-to-place, undo/redo on cmd/ctrl-z (+shift), and
cut/paste. Consecutive typing coalesces into one undo entry; every key is
gated on focus + `Mode::Editable`, so readonly tabs behave byte for byte
as before. The h-scroll/wrap caches key off buffer version, so they
invalidate per keystroke — covered by tests (`max_line_width` grows on
edit, shrinks on undo).

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
