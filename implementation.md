# gpui-editor — Implementation Plan

Goal: fast, low-RAM / low-CPU code viewer + syntax package for embedding in another app. GPUI-native.

## 0. Goals / Non-Goals

Goals:
- Read-only viewer first, optional edit mode behind flag.
- Lowest RAM/CPU: virtualize render, cache shaped lines, parse off-thread, incremental everything.
- Embeddable: `Entity<EditorState>` + `EditorView`, themable via `cx.theme()`.
- Features: readonly, optional edit, selection, search, bracket match, auto-indent.

Non-Goals (v1):
- No multi-cursor, no minimap, no LSP, no inlay hints, no collaborative editing.
- No full IDE (tabs, sidebar, statusbar owned by host app).

Performance budgets (measure on M-series, release):
- Scroll 10k-line file at 60fps+, frame build < 8ms.
- Open 1MB file < 100ms to first paint, incremental highlight < 16ms per keystroke.
- RAM: text stored once (rope) + shaped cache for visible lines only (~100 lines). No per-char GPUI element.
- CPU idle: 0. No timers except cursor blink when focused. Parse debounced 50ms, cancelled on new edit.

## 1. Architecture — 3 crates, 1 dependency

Depend only on `gpui-kit` in UI crate. Core crates have zero GPUI deps.

```
gpui-editor/
  Cargo.toml (workspace)
  crates/
    edit-buffer/   # rope, history, line map — no gpui
    syntax/        # tree-sitter registry, highlight — no gpui
    editor-ui/     # Entity + Element, depends on gpui-kit + above two
  languages/       # *.scm queries
  implementation.md
```

Why: host app can use `edit-buffer`/`syntax` headless for search/indexing without opening a window. UI stays thin.

## 2. Crate 1: `edit-buffer`

Responsibility: single source of truth for text.

- Storage: `ropey::Rope` (or `sum-tree` if we want Zed-style). Store once. `RopeSlice` for reads, no `String` clone per line.
- Line map: `Vec<usize>` byte offset per line, rebuilt incrementally on edit. `line(n) -> RopeSlice`, `offset_to_point`, `point_to_offset`.
- Large files: `memmap2` + chunked load. Cap: files > 5MB open in readonly + no highlight beyond 5k visible window (configurable). Never load whole file into `String`.
- Edit model:
  ```rust
  pub struct Edit { range: Range<usize>, text: String }
  pub struct BufferSnapshot { version: u64, len: usize }
  impl Buffer {
    pub fn text(&self) -> &Rope;
    pub fn line(&self, row: u32) -> RopeSlice;
    pub fn line_count(&self) -> u32;
    pub fn edit(&mut self, edits: &[Edit]) -> u64; // returns new version
    pub fn snapshot(&self) -> BufferSnapshot;
  }
  ```
- Undo/redo (edit mode only): bounded stack (100 steps), char-grouped by word/time. Disabled in readonly to save RAM.
- Line endings: normalize to LF internally, preserve on save.

Perf rules: all edits O(log n). `snapshot()` is cheap clone (Arc<Rope>). No regex in hot path.

## 3. Crate 2: `syntax`

Responsibility: bytes -> `HighlightSpan`, off main thread.

- Engine: `tree-sitter` + `tree-sitter-highlight`. One `Parser` per language per worker (not shared across threads).
- Registry:
  ```rust
  pub struct Language { name: &'static str, parser: fn() -> Parser, highlights_query: &'static str }
  pub struct LanguageRegistry { map: HashMap<&'static str, Arc<Language>> }
  ```
  Ship `rust, toml, markdown, json, ts` first. Queries in `languages/{lang}.scm`.
- Incremental: keep `Tree` from last parse. On `Buffer::edit`, call `tree.edit(&InputEdit)` then `parser.parse()`. Only re-highlight changed byte ranges + 5 lines slop.
- Output:
  ```rust
  pub struct HighlightSpan { pub start: usize, pub end: usize, pub capture: Capture }
  pub struct HighlightedVersion { pub buffer_version: u64, pub spans: Vec<HighlightSpan> }
  ```
- Threading: `cx.background_spawn` worker. Debounce 50ms. Cancel prior task on new version (version check, drop stale results). Fallback: plain-text (single style) if parse > 50ms.
- Theme map: `Capture -> Hsla` via `cx.theme()` tokens, e.g. `keyword -> primary`, `string -> green`, `comment -> muted`. Do lookup once per highlight pass, store `Hsla` in render cache, not per frame.
- RAM: store spans as `Vec<u32>` packed (start, len, style_idx), not `String`. Drop spans for lines far outside viewport after 30s (LRU).

## 4. Crate 3: `editor-ui` (GPUI)

State ownership: `Entity<EditorState>` holds everything. View holds `Entity` handle only.

```rust
pub struct EditorState {
  buffer: Buffer,
  language: Option<Arc<Language>>,
  mode: Mode, // ReadOnly | Editable
  highlight: Option<HighlightedVersion>,
  selection: Selection, // single range only
  search: SearchState,
  scroll_handle: ScrollHandle,
  line_cache: HashMap<u32, CachedLine>, // row -> shaped
}
pub struct EditorView { state: Entity<EditorState>, focus: FocusHandle }
```

- Check `gpui_kit::component::input::{Editor, EditorState}` first. If its `tree-sitter` feature covers readonly + search, wrap it instead of custom `Element`. Only go custom if measured frame cost too high.
- Custom `Element` (if needed): `EditorElement { state: Entity<EditorState> }`.
  - `request_layout`: measure `line_height * visible_count`, fixed `line_height` from `TextSystem`.
  - `prepaint/paint`: for `row in viewport_rows`: get-or-shape `CachedLine`, paint runs with `window.paint_text()`.
  - `CachedLine { version: u64, shaped: ShapedLine, styles: Vec<StyleRun> }`. Key `(row, buffer_version, theme_id)`. Evict beyond viewport+100.
- IDs: `ElementId::Name(format!("editor-line-{row}").into())`. Never list index as id.
- Focus: `FocusHandle` on view, `key_context "Editor"`, `actions!` for copy/select-all/search. Blink cursor via `cx.spawn` timer only when focused + editable.
- Scroll: native `ScrollHandle`, `overflow-y: scroll`. No custom scrollbar v1, use `gpui_kit::component::scroll::Scrollbar` if needed.
- Overlays: search bar as child `div`, not separate window. Diagnostics none v1.

## 5. Feature Spec (v1 only)

1. **Readonly (M1):** `EditorState::readonly(text, lang)`. No cursor, no input handler. Select + copy allowed. Fast scroll.
2. **Optional edit (M2):** `mode: Editable` flag. Single cursor, insert/delete/newline, undo/redo (`cmd-z`). IME via GPUI default. No autocomplete.
3. **Selection (M2):** single `Range<Point>`. Mouse drag, shift+arrows, `cmd-a`. Render as `paint_quad` behind text. Copy copies selected slice only.
4. **Search (M3):** `cmd-f` overlay. Literal + case-insensitive first, regex behind flag. `editor-ui` holds `matches: Vec<Range<usize>>`, current index. Highlight all matches (yellow wash), current (orange). Enter/next, wrap. Search runs on `Rope` with `memchr`, not regex per keystroke. Debounced 100ms, max 1000 matches.
5. **Bracket match (M3):** on cursor move or click adjacent to `()[]{}`, find mate by scanning with depth counter, limit 5000 chars each dir to bound CPU. Highlight both with border quad. No tree-sitter dependency — pure text scan so it works in plain-text fallback.
6. **Indent (M3):** keep-indent + bracket extra indent on newline. Language-aware only via 2 rules: if line ends with `{`/`[`/`(` increase by 1 tab, if next char is `}` decrease. Config `tab_width: 2|4`, `hard_tabs: bool`. No full formatter.

Out: multi-cursor, word wrap (v1 uses h-scroll, wrap later — wrap kills perf), folding, minimap.

## 6. Threading / Data Flow

```
key/mouse -> EditorView -> EditorState::apply_edit()
  -> buffer.edit() bumps version, drops CachedLine for dirty rows
  -> cx.notify() for immediate repaint with stale highlight (fast)
  -> background_spawn parse(version): tree-sitter -> HighlightedVersion
  -> cx.update(|cx| state.highlight = v; evict + notify) if version still current
```

Never block `render` on parse. Never allocate in `paint` except shaped-line miss (then shape once).

## 7. Milestones

- **M0 scaffold:** workspace, `gpui-kit` dep, `cargo run` opens window with `Root` + empty `EditorView`. `#[gpui_kit::test]` smoke test.
- **M1 readonly viewer:** load file, virtualized scroll 10k lines, sync highlight for Rust/MD. Bench: scroll fps, open time.
- **M2 edit + selection:** editable flag, undo, single selection, copy. Bench: keystroke latency <16ms p95.
- **M3 search + brackets + indent:** search overlay, bracket highlight, auto-indent. Bench: search 1MB <50ms.
- **M4 harden:** large-file guard, theme support, docs, publish as lib for host app. `cargo clippy`, UI integration tests for each feature.

Each milestone ends with `cargo test -p editor-ui` + manual 10k-line scroll check.

## 8. Public API (embed in host app)

```rust
use editor_ui::{EditorState, EditorView};

let state = cx.new(|cx| EditorState::readonly("fn main(){}", Some("rust"), window, cx));
let state2 = cx.new(|cx| EditorState::editable("", None, window, cx));
let view = EditorView::new(&state);
// in render: EditorView::new(&self.state)
// actions: state.update(cx, |s, cx| s.set_search("fn", window, cx));
```

Builders only, no `pub` fields. `new()` takes `&Window, &mut Context`. Setters `with_language()`, `with_mode()`.

## 9. Testing

- Unit (`edit-buffer`, `syntax`): edit/undo, line map, incremental parse returns same spans for untouched lines.
- UI integration (`#[gpui_kit::test]`): type char -> buffer version bump, search -> match count, bracket -> highlight quad present, readonly ignores input.
- Perf: `criterion` bench for `buffer.edit` + `highlight` on 1MB Rust file. Fail CI if p95 > budget.

## 10. Risks

- Shaped text cost — mitigate with line cache + fixed line height.
- tree-sitter query maintenance — pin versions, snapshot-test highlight output.
- GPUI API drift — single `gpui-kit` dep, search source before inventing API, never translate React/CSS patterns.
