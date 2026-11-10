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

Shipped: text entry through the platform input handler (plain typing,
dead keys, press-and-hold, emoji picker, IME composition with underlined
provisional text), Enter (auto-indent from the text before the caret) /
Backspace / Delete / Tab, a visible caret at the selection head that is
scrolled back into view after every keyboard edit or motion, arrow / Home
/ End motion with a goal column, click-to-place, undo/redo on cmd/ctrl-z
(+shift), cut/paste, and save on cmd/ctrl-s.

Model guarantees, all covered in `tests/`:
- Every mutation notifies observers, so views repaint without the caller
  remembering to.
- Undo entries replay in the order they were made; a typing run ends on
  caret motion, a structural edit, or 500ms idle; an IME composition is
  one entry.
- `set_text` is a document load: history cleared, buffer clean.
- The buffer holds `\n` only; the file's line ending is restored on save.
- Dirty state follows content, not edit count: undo back to the saved
  text is clean.
- Readonly ignores every edit path (keys, input handler, save).

Still open:
- Cut a release rev when green (the desktop app consumes the editor by
  pinned rev, not path)
- Check real IME on macOS by hand in the demo (accent via press-and-hold,
  a CJK input source, the emoji picker) — the tests drive the handler
  directly, not the OS
- Word/line motion and delete (alt/cmd + arrows, backspace), shift-click,
  double-click word select, PageUp/PageDown
- Block indent / outdent (Tab / Shift+Tab over a selection)
- Selection direction for mouse-made selections (shift+arrow after a
  backwards drag extends the wrong end)
- Incremental highlight and search instead of whole-buffer per keystroke
- Notice files changed on disk underneath the editor

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
