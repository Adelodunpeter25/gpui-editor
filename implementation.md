# gpui-editor — Implementation Plan

Goal: fast, low-RAM / low-CPU code viewer + syntax package for embedding in another app. GPUI-native.

**Status at a glance:** M1 (readonly viewer) is done, including virtualized
scroll, a hand-rolled scrollbar, mouse selection + copy, real theme-driven
syntax colors, and word wrap — ahead of the original M1→M4 ordering below in
some ways (selection/wrap), behind in others (no edit mode yet). See §7 for
the accurate per-milestone status. For embedding this in a host app, see
`package.md`.

## 0. Goals / Non-Goals

Goals:
- Read-only viewer first, optional edit mode behind flag.
- Lowest RAM/CPU: virtualize render, cache shaped lines, parse off-thread, incremental everything.
- Embeddable: `Entity<EditorState>` + `EditorView`, themable via `syntax::ThemePreset`.
- Features: readonly, optional edit, selection, search, bracket match, auto-indent.

Non-Goals (v1):
- No multi-cursor, no minimap, no LSP, no inlay hints, no collaborative editing.
- No full IDE (tabs, sidebar, statusbar owned by host app).

Performance budgets (measure on M-series, release):
- Scroll 10k-line file at 60fps+, frame build < 8ms.
- Open 1MB file < 100ms to first paint, incremental highlight < 16ms per keystroke.
- RAM: text stored once (rope) + shaped cache for visible lines only (~100 lines). No per-char GPUI element.
- CPU idle: 0. No timers except cursor blink when focused. Parse debounced 50ms, cancelled on new edit.

**Status:** scroll/frame-build budget met via `uniform_list` virtualization
(only visible rows are built/painted). Open-time and incremental-highlight
budgets are **not yet measured** — no criterion benches exist (§9). Syntax
highlight today is synchronous on the caller's thread (`EditorState::rehighlight`),
not off-thread/debounced — acceptable at readonly-demo scale (confirmed
smooth scrolling an 8k-line file), a real risk once edit mode makes it fire
per keystroke. See the "known gaps" note at the end of §7.

## 1. Architecture — 4 crates

Raw `gpui` only in the UI crate (no component-kit dependency — this
diverged from the original "1 dependency" `gpui-kit` plan once `gpui-kit`
turned out not to be the right fit; raw `gpui 0.2.2`, pinned to a specific
git rev, was used instead). Core crates have zero GPUI deps.

```
gpui-editor/
  Cargo.toml (workspace, pins gpui + gpui_platform to one git rev)
  crates/
    edit-buffer/   # rope, history, line map — no gpui
                    #   src/types.rs: Point, Edit, BufferSnapshot, Buffer
    syntax/        # tree-sitter registry (via `lumis`) + highlight — no gpui
                    #   src/languages.rs: Language, LanguageRegistry
                    #   src/theme.rs: ThemePreset, get_theme (lumis themes)
                    #   src/types.rs: Capture, HighlightSpan, HighlightedVersion
    editor-ui/      # Entity + Render, depends on gpui + edit-buffer + syntax
                    #   src/types.rs: EditorState, EditorView, Mode, Selection, ...
                    #   src/wrap.rs: word-wrap (buffer-row <-> visual-row mapping)
                    #   src/lib.rs: Render impl + paint helpers
    editor-demo/    # example host app (window, file picker, toolbar, menus)
  diff.md           # research + future plan for a diff viewer (not built)
  wrap.md           # word-wrap design doc (implemented; kept for history)
  package.md        # embedding guide for host apps
  implementation.md
```

Why: host app can use `edit-buffer`/`syntax` headless for search/indexing without opening a window. UI stays thin.

## 2. Crate 1: `edit-buffer`

Responsibility: single source of truth for text.

- Storage: `ropey::Rope`. Store once. `RopeSlice` for reads, no `String` clone per line in the hot path (`line_text` does allocate — documented as the non-hot-path variant; `line_slice` is the zero-copy one).
- Point/offset conversion: `Point { row, col }`, `offset_to_point`, `point_to_offset` — implemented (not just planned).
- Edit model (implemented, matches original plan):
  ```rust
  pub struct Edit { pub range: Range<usize>, pub text: String }
  pub struct BufferSnapshot { pub version: u64, pub len_chars: usize }
  impl Buffer {
      pub fn text(&self) -> &Rope;
      pub fn line_slice(&self, row: u32) -> RopeSlice<'_>;
      pub fn line_text(&self, row: u32) -> String;
      pub fn slice_text(&self, range: Range<usize>) -> String; // for copy
      pub fn line_count(&self) -> u32;
      pub fn edit(&mut self, edits: &[Edit]) -> u64;
      pub fn snapshot(&self) -> BufferSnapshot;
      pub fn search(&self, needle: &str, case_sensitive: bool, limit: usize) -> Vec<Range<usize>>;
  }
  ```
- Undo/redo: bounded stack (100 steps), implemented. `from_text_readonly` disables history (`max_history: 0`) to save RAM in readonly mode.
- **Not implemented:** `memmap2`/chunked large-file loading, line-ending normalization, word/time-grouped undo batching. Whole file is loaded into the rope via `Rope::from_str` regardless of size.

Perf rules: all edits O(log n) via ropey. No regex in the edit hot path (search still materializes the whole rope to a `String` per call — fine for demo-sized files, not for 1MB+).

## 3. Crate 2: `syntax`

Responsibility: bytes -> `HighlightSpan`, using `lumis` (tree-sitter under the hood) instead of hand-rolled `tree-sitter-highlight`.

- Registry: `Language { name, lumis_lang, extensions }` + `LanguageRegistry`, in `languages.rs`. Ships far more than the originally planned `rust, toml, markdown, json, ts` — see `ALL_LANGUAGES` (Rust, Python, Go, C/C++, Java, Kotlin, Swift, PHP, Ruby, Bash, YAML, SQL, Zig, Lua, Dockerfile, Elixir, Erlang, Haskell, OCaml, Scala, Clojure, Dart, GLSL, Make, CMake, Nix, Solidity, GraphQL, Protobuf, Astro, Svelte, Vue, XML, INI, HTML, CSS, SCSS, TS/TSX/JS, Diff, and more).
- Theming: `ThemePreset` (Dracula, OneDark, GitHubDark/Light, CatppuccinMocha, Nord, TokyoNight, Solarized, MonokaiPro, VSCode Dark/Light) wraps `lumis::themes::get(name)`. `syntax::highlight_themed(text, language, buffer_version, theme)` calls `lumis::highlight::highlight_iter` and reads each scope's **actual theme color** (`style.fg`, hex-parsed to RGB) — not a hand-picked palette. `editor-ui`'s `color_for(Capture)` is only a fallback for scopes the theme leaves unstyled.
- Output (implemented, close to original plan):
  ```rust
  pub struct HighlightSpan { pub start: usize, pub end: usize, pub capture: Capture, pub color: Option<(u8,u8,u8)> }
  pub struct HighlightedVersion { pub buffer_version: u64, pub spans: Vec<HighlightSpan> }
  ```
- **Not implemented:** off-thread/background parsing, incremental re-parse (keeping a `Tree` across edits), debouncing, packed `Vec<u32>` span storage, LRU eviction of far-offscreen spans. Every `highlight_themed` call re-parses and re-highlights the *entire* text from scratch — acceptable today because it only runs on `set_text`/`set_language`/`set_theme`, never per-frame or per-keystroke (there's no edit-mode keystroke loop exercising it yet).

## 4. Crate 3: `editor-ui` (GPUI)

State ownership: `Entity<EditorState>` holds everything. View holds handles + transient UI interaction state only. Split across three files (not one, per this session's own refactor — kept types/wrap/render logic in separate files specifically so no single file grows into a "does everything" blob):

```rust
// types.rs
pub struct EditorState {
    buffer: Buffer,
    language: Option<&'static Language>,
    theme: ThemePreset,
    highlight: HighlightedVersion,
    mode: Mode, // ReadOnly | ReadOnlyDiff | Editable | EditableDiff
    selection: Selection,
    search: SearchState,
    indent: IndentOptions,
    wrap_enabled: bool,
    row_spans: Vec<Vec<StyledSpan>>, // row -> highlight spans clipped to that row
}

pub struct EditorView {
    state: Entity<EditorState>,
    scroll_handle: UniformListScrollHandle,
    focus_handle: FocusHandle,
    viewport_height: Rc<Cell<Pixels>>,
    viewport_width: Rc<Cell<Pixels>>,
    char_width: Rc<Cell<Option<Pixels>>>,   // for mouse hit-testing
    selecting: Rc<Cell<bool>>,
    drag_anchor: Rc<Cell<Option<usize>>>,
    thumb_dragging: Rc<Cell<Option<(Pixels, Pixels)>>>, // scrollbar drag
    wrap_cache: Rc<RefCell<WrapCache>>,
}
```

Only `Mode::ReadOnly` is actually implemented; `ReadOnlyDiff`/`Editable`/`EditableDiff` are reserved variants (see `diff.md` for the diff-viewer plan; edit mode is simply unbuilt).

- **Virtualization:** uses `gpui::uniform_list` (not a hand-rolled `Element` — raw gpui already ships exactly this primitive for uniform-height rows), so only visible rows are built/painted each frame regardless of file size. The originally-planned custom `EditorElement`/`CachedLine`/shaped-line cache was **not needed** — `uniform_list` covers the same virtualization goal without hand-rolled `request_layout`/`prepaint`/`paint`.
- **Scrollbar:** raw gpui has no standalone scrollbar widget (only one baked into its non-uniform `list()` element). Hand-rolled: a thin draggable thumb sized/positioned from `UniformListScrollHandle`'s live `offset()`/`max_offset()`/`bounds()`.
- **Word wrap:** implemented per `wrap.md`, using gpui's own `LineWrapper` (cached per-char glyph widths internally). Lives in `wrap.rs` as a `WrapCache` mapping buffer rows -> visual (wrapped) rows, rebuilt only when buffer version, wrap width, or the wrap-enabled flag changes — bucketed to the nearest 32px so a live window resize doesn't re-shape every row every frame. Off by default (`EditorState::with_wrap`/`set_wrap_enabled`).
- **Selection + copy:** implemented via mouse down/drag/up on each rendered row, hit-testing pixel-x to a char column using a cached monospace glyph width (`window.text_system().shape_line`). `cmd/ctrl-c` copies the selection (`ClipboardItem`), `cmd/ctrl-a` selects all. Selection is rendered as a translucent background wash merged into the same syntax-run breakpoint list, not a separate `paint_quad` overlay as originally sketched.
- IDs: `ElementId::Name`/tuple ids used throughout (`("editor-line", row)`, etc.) — never a bare list index alone, matches the original plan.
- Focus: `FocusHandle` on the view, `key_context "Editor"`, `actions!` for `Copy`/`SelectAll`. **No cursor blink timer** — there's no cursor at all yet (readonly, no edit-mode caret).
- Search/bracket-match/indent: `EditorState` has `bracket_match`, `indent_for_newline`, `set_search`/`next_match` — but **no search overlay UI, no bracket-highlight rendering, no indent-on-newline wiring** in `EditorView` yet. The model-layer logic exists; the view-layer hookup (M3 in the original plan) doesn't.

## 5. Feature Spec (v1 only) — status per item

1. **Readonly (M1):** ✅ done. `EditorState::readonly(text, lang)`. Virtualized scroll, confirmed smooth on an 8k-line file.
2. **Optional edit (M2):** ❌ not built. `Mode::Editable` exists as an enum variant and `EditorState::editable()`/`insert()` exist at the model layer, but there's no cursor, no input handler, no `cmd-z` wiring in `EditorView`.
3. **Selection (M2):** ✅ done, ahead of schedule relative to edit mode. Mouse drag + `cmd-a` implemented; shift+arrows not implemented (no keyboard caret to extend from, since there's no cursor yet).
4. **Search (M3):** 🟡 model only. `EditorState::set_search`/`next_match`/`SearchState` exist; no `cmd-f` overlay, no match-highlight rendering.
5. **Bracket match (M3):** 🟡 model only. `EditorState::bracket_match` exists (pure text scan, as planned); no highlight-quad rendering wired to cursor position (no cursor yet).
6. **Indent (M3):** 🟡 model only. `EditorState::indent_for_newline` exists; nothing calls it (no edit-mode newline handling yet).
7. **Word wrap:** ✅ done — originally listed as out-of-scope/"wrap kills perf, do later"; implemented this session per `wrap.md` once gpui's `LineWrapper` was confirmed to make it cheap. No horizontal scroll exists (never did; long lines were clipped pre-wrap).
8. **Scrollbar:** ✅ done (not in the original spec at all — added because `uniform_list` + `overflow-y: scroll` alone gave no visual scroll indicator).
9. **Theme-driven colors:** ✅ done — real lumis theme colors per scope, not a fixed hand-picked palette.

Out: multi-cursor, folding, minimap, per-line wrap toggle, diff viewer (see `diff.md`).

## 6. Threading / Data Flow

**Current (synchronous) flow — not the originally-planned background-parse flow:**

```
EditorState::set_text/set_language/set_theme
  -> buffer.edit() bumps version (set_text only)
  -> rehighlight(): full buffer.text().to_string() + syntax::highlight_themed() synchronously
  -> rebuild_row_spans(): O(all rows), not just dirty ones
  -> caller's cx.notify() repaints
```

The originally-planned flow below is **not implemented**. It only becomes
necessary once edit mode exists and highlighting must run per keystroke
without janking typing — see the "known gaps" note at the end of §7 before
building this out speculatively:

```
key/mouse -> EditorView -> EditorState::apply_edit()
  -> buffer.edit() bumps version, drops CachedLine for dirty rows
  -> cx.notify() for immediate repaint with stale highlight (fast)
  -> background_spawn parse(version): tree-sitter -> HighlightedVersion
  -> cx.update(|cx| state.highlight = v; evict + notify) if version still current
```

Render-time flow (implemented, this session):
```
EditorView::render()
  -> ensure WrapCache fresh (buffer version / wrap width / wrap-enabled changed?)
  -> uniform_list(visual_row_count, |visible_range| {
       for visual_ix in visible_range:
         resolve (buffer_row, sub_line) from WrapCache
         slice line_text + row_spans + selection down to that sub-line
         render_line(...)
     })
```
Never blocks on parsing today because nothing re-parses per frame — the risk is the *opposite* of the original doc's worry: parsing blocks on edit, not on render.

## 7. Milestones — actual status

- **M0 scaffold:** ✅ done (workspace, raw `gpui`, window + `EditorView`).
- **M1 readonly viewer:** ✅ done. Virtualized scroll confirmed smooth on an 8k-line file manually; no formal fps/open-time benchmark exists (see §9).
- **Extras beyond M1, done ahead of M2/M3:** selection + copy, draggable scrollbar, real theme colors, word wrap, `Mode` variants reserved for diff/edit.
- **M2 edit + selection:** 🟡 half done — selection is done, editing (cursor, insert/delete, undo/redo wiring, keystroke latency bench) is not.
- **M3 search + brackets + indent:** 🟡 model-only — none have view-layer UI.
- **M4 harden:** ❌ not started. No large-file guard, no `cargo clippy` in CI (manual `cargo clippy --workspace --all-targets` has been run ad hoc and is clean), no published docs beyond this file + `package.md`.

**Known gaps to fix before M2 (edit mode) lands**, flagged during a review
of this session's work (not yet actioned — listed here so they aren't lost):
- `rehighlight()` is synchronous full-buffer re-parse on every edit; fine
  today because nothing calls `insert()` in a loop, but will jank typing
  once edit mode exists.
- `rebuild_row_spans()` rebuilds every row's spans, not just rows touched by
  an edit.
- `render_spans()` rebuilds a fresh element list per visible row every
  frame — cheap today only because rows are short and few are visible.

Minimal recommended next step when edit mode starts: debounce
`rehighlight` + make `rebuild_row_spans` dirty-row-only, *before* reaching
for full background-thread parsing + a `ShapedLine` cache (that's real
threading/eviction work that only pays off at the 1MB/10k-line/60fps
budget in §0, and is easier to get right against real edit traffic than to
guess at now).

Each milestone should end with `cargo test --workspace` (per AGENTS.md) + a
manual scroll/selection/resize check, matching how every feature this
session was actually verified (no `#[gpui::test]` UI-test harness exists
yet in this repo).

## 8. Public API (embed in host app)

See `package.md` for the full embedding guide (font registration, gpui rev
pinning, etc). Minimal shape, matching current code (not the original
sketch — no `Window` param on `EditorState` constructors, `EditorView::new`
does take `&mut Context`):

```rust
use editor_ui::{EditorState, EditorView};

editor_ui::init(cx); // once at startup: binds cmd/ctrl-c, cmd/ctrl-a

let state = cx.new(|_| EditorState::readonly("fn main(){}", Some(&syntax::RUST)));
let view = cx.new(|cx| EditorView::new(&state, cx));
// in render: div().child(view.clone())
```

Builders only, no `pub` fields. Setters: `with_mode()`, `with_indent()`, `with_wrap()`/`set_wrap_enabled()`, `set_language()`, `set_theme()`.

## 9. Testing

- Unit (`edit-buffer`, `syntax`, `editor-ui`): 24 tests across the workspace today (`cargo test --workspace`) — buffer edit/undo/search/slice, highlight capture kinds + language registry resolution, `EditorState` selection/row-offset/point-conversion/wrap-flag/mode behavior.
- **Not implemented:** UI integration tests (`#[gpui::test]`) for click-to-select, copy, wrap toggle, scrollbar drag — these were verified manually per feature instead. `criterion` perf benches for `buffer.edit`/`highlight` on a 1MB file don't exist; the perf budgets in §0 are unverified beyond informal manual checks (smooth scroll on an 8k-line file).

## 10. Risks

- Shaped text cost — mitigated via `uniform_list` virtualization (only visible rows shaped/painted) rather than the originally-planned custom line-cache `Element`.
- Synchronous full-buffer re-highlight — not yet a problem (readonly, no edit loop), but the top risk once edit mode lands. See §7's "known gaps."
- tree-sitter query maintenance — delegated to `lumis`, which owns its own grammar/query versions; this repo doesn't maintain `.scm` files directly (diverges from the original plan's `languages/*.scm` layout, which was never built).
- GPUI API drift — real risk realized once already: `gpui`/`gpui_platform` are pinned to an exact git rev (`5631830c...`) in `Cargo.toml` after floating on zed's `main` briefly broke text rendering entirely. Any host app embedding `editor-ui` must pin the identical rev (see `package.md`).
