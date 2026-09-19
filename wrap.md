# Word wrap — implementation plan (not started)

No horizontal scrolling is wanted. Today long lines are silently clipped
(`overflow_x_hidden()` in `render_line`, `crates/editor-ui/src/lib.rs`) —
there's no h-scroll to remove, just a clip to replace with a real wrap.

## Why this isn't a small change

The whole render/virtualization/selection stack (built in earlier sessions:
`crates/editor-ui/src/lib.rs` + `types.rs`) assumes **1 buffer line == 1
fixed-height visual row**:

- `uniform_list("gpui-editor-lines", line_count, ...)` — `line_count` is
  `EditorState::line_count()`, i.e. buffer rows.
- `LINE_HEIGHT` (`px(22.0)`) is a per-row constant the scrollbar, the
  viewport-snapping math, and `uniform_list`'s own visible-range math all
  depend on.
- Selection/hit-testing (`row_start_offset`, `row_end_offset`,
  `point_to_offset`, the mouse handlers in `render_line`) all key off
  **buffer row index**, assuming one row = one line of text at a fixed y.

Word wrap breaks the 1:1 assumption: one buffer line becomes N visual rows,
N varies per line, and N changes whenever the viewport resizes or that
line's text is edited. Every one of the systems above needs a visual-row
layer added on top of the buffer-row layer they use today.

implementation.md already flagged this cost explicitly (§5, out-of-scope
for v1): *"word wrap (v1 uses h-scroll, wrap later — wrap kills perf)"*.

## The good news: gpui already has the hard part

Checked the vendored `gpui` crate
(`crates/gpui/src/text_system/line_wrapper.rs` at the pinned rev). It ships
`LineWrapper::wrap_line(fragments, wrap_width) -> impl Iterator<Item =
Boundary>` — proper greedy word-wrap (breaks at word boundaries, falls back
to a hard char break for tokens wider than `wrap_width`, handles CJK) with
**per-char width caching built in** (`cached_ascii_char_widths`,
`cached_other_char_widths`). This is the same wrapper Zed's own editor uses.
We do not need to hand-roll wrap-point math — this directly answers
implementation.md's "wrap kills perf" worry, as long as we cache its output
(see below) rather than re-wrapping every frame.

Obtained via `text_system.line_wrapper(font_id, font_size)` (pooled per
`TextSystem`'s `wrapper_pool`, so repeated calls for the same font/size are
cheap).

## Proposed design

### 1. A visual-row layer in `EditorState`

Add a `WrapCache` alongside `row_spans`:

```rust
struct WrapCache {
    wrap_width: Option<Pixels>,   // None = wrap disabled (today's behavior)
    buffer_version: u64,
    /// One entry per buffer row: char-offset boundaries within that row
    /// where a visual line break occurs (from LineWrapper::wrap_line).
    row_breaks: Vec<Vec<usize>>,
    /// Prefix sums so "visual row N" -> (buffer_row, sub_line_index) is a
    /// binary search, not a scan. Rebuilt alongside `row_breaks`.
    visual_row_starts: Vec<usize>,
}
```

- Invalidated (rebuilt) whenever `buffer_version` changes (same trigger as
  `rebuild_row_spans`) **or** `wrap_width` changes (viewport resize).
- Rebuild cost: one `LineWrapper::wrap_line` call per buffer row that
  changed — for a full-buffer edit or resize that's every row once, same
  order of cost as `rebuild_row_spans` already pays today, not a new
  category of cost.
- `EditorState::visual_row_count() -> usize` replaces `line_count()` as
  `uniform_list`'s item count when wrap is on.
- `EditorState::visual_row(ix) -> (buffer_row: u32, char_range: Range<usize>)`
  is the new per-row lookup `render()`'s `uniform_list` closure calls
  instead of `line_text`/`row_spans.get(row)` directly — it resolves to a
  buffer row + a char sub-range within that row, then slices `line_text`
  and `row_spans` the same way as today.

### 2. Rendering (`crates/editor-ui/src/lib.rs`)

- `uniform_list` keeps its uniform-height contract (gpui's `uniform_list`
  requires this — see `crates/gpui/src/elements/uniform_list.rs`'s own doc
  comment: "provides lazy rendering for a set of items that are of uniform
  height"). Each **visual** row is still exactly `LINE_HEIGHT` tall — wrap
  only changes *what maps to a row index*, not the row-height virtualization
  scheme itself. This means the scrollbar and viewport-snapping code added
  earlier this session need no changes — they already work purely in terms
  of "how many uniform rows, how tall is the track," which stays valid.
- `render_line` gets an extra `is_continuation: bool` (or
  `sub_line_index: u32`) parameter: continuation rows of a wrapped buffer
  line render an **empty** gutter cell instead of repeating the line number
  (matches how every mainstream editor shows wrapped lines).
- Word-wrapping must re-measure on resize. Reuse the `canvas`-based
  viewport-width measurement pattern already in `render()` (mirrors the
  existing `viewport_height` canvas) to get `wrap_width`, store it in
  `EditorState` via a new `set_wrap_width` setter, and trigger a
  `WrapCache` rebuild only when it actually changes (not every frame).

### 3. Selection / hit-testing (`crates/editor-ui/src/types.rs` + `lib.rs`)

This is the fiddliest part, because every selection helper added this
session is buffer-row-keyed:

- `row_start_offset`/`row_end_offset` stay as buffer-row helpers (used for
  syntax-span row-slicing, unaffected).
- The mouse-down/move handlers in `render_line` currently do
  `Point { row, col }` → `state.point_to_offset(...)` where `row` is a
  buffer row. With wrap on, the row a mouse event fires in is a **visual**
  row; it must resolve to `(buffer_row, char_range)` via
  `EditorState::visual_row(ix)` first, then the existing column math (char
  width × local x) still applies **within that sub-range**, just offset by
  `char_range.start` instead of `0`.
- Drag-selection across a wrap boundary (mouse moves from the tail of one
  visual row to the head of the next, same buffer line) must resolve to a
  single continuous buffer-offset range — this falls out naturally once
  selection is computed in buffer-offset space (which it already is;
  `EditorState::selection` never knew about rows to begin with) as long as
  the visual→buffer row resolution above is correct.

### 4. Public API

- `EditorState::with_wrap(bool)` builder + `set_wrap_enabled(bool)` setter,
  defaulting to **off** (matches today's behavior; host apps opt in).
- No change to `EditorView`'s public surface — wrap is a render-time
  concern driven by `EditorState`.

## Milestones

1. **Wrap computation + cache**: `WrapCache`, `visual_row_count`,
   `visual_row(ix)` in `types.rs`. Unit-testable without a `Window` (feed it
   a fixed `wrap_width` and fake `LineWrapper` output, or — simpler — test
   the boundary-merging/prefix-sum logic directly against hand-computed
   `row_breaks`, independent of gpui's wrapper).
2. **Render wiring**: `uniform_list` switches to `visual_row_count`/
   `visual_row`; continuation-row gutter blanking; resize-driven
   `set_wrap_width` via the canvas pattern.
3. **Selection fix-up**: mouse handlers resolve visual row → buffer
   row + sub-range before hitting existing column math.
4. **Toggle wiring in `editor-demo`**: a menu item or keybinding to flip
   `set_wrap_enabled`, so it's actually exercisable end-to-end.

Each milestone should end with `cargo test --workspace` (per AGENTS.md) plus
a manual scroll/resize/select check on a file with some very long lines,
per this session's precedent (the uniform_list virtualization and scrollbar
work were both verified this way).

## Explicitly out of scope for v1 wrap

- Per-line wrap toggle (whole-buffer only, like every other v1 feature flag
  in this codebase).
- Indent-aware wrap (continuation lines aligning under the original line's
  indent) — cosmetic nice-to-have, not required for correctness.
- Soft-wrap-aware bracket matching / search highlighting — both already
  operate in buffer-offset space today and should keep working unmodified
  once selection's visual→buffer resolution (step 3 above) is correct,
  since they never rendered per-row markers to begin with. Verify this
  rather than assuming it once wrap lands.
