# Embedding `editor-ui` in another app

A short guide for dropping this crate's code viewer into an existing GPUI
app. See `implementation.md` for what's actually implemented vs planned.

## 0. Each crate is a separate, independently-pullable feature

This isn't one monolithic package — pull in only what you need:

- **`edit-buffer`** — rope-backed text storage (edit/undo/search/point
  conversion). Zero GPUI dependency. Useful standalone if you need
  efficient text storage/editing without any UI at all.
- **`syntax`** — language registry + `highlight_themed()` (via `lumis`),
  returning theme-colored spans as plain data. Zero GPUI dependency. Use
  this alone if you just need syntax-highlighted spans to paint yourself —
  e.g. coloring fenced code blocks in a markdown renderer — without
  pulling in the rest of the editor.
- **`editor-ui`** — the full GPUI scrollable code *viewer*: virtualization,
  scrollbar, mouse selection, word wrap. Depends on the two crates above.
  Pull this in only when you actually want a full embeddable file-viewer
  widget, not just colored text — it carries real weight (scroll/selection
  machinery) that's wasted on something like a short static code snippet.

This repo isn't published to crates.io — pull in whichever crate(s) you need
as a `git` dependency, pointing at the subdirectory:

```toml
# your host app's Cargo.toml — pick only what you need
edit-buffer = { git = "https://github.com/Adelodunpeter25/gpui-editor.git" }
syntax      = { git = "https://github.com/Adelodunpeter25/gpui-editor.git" }
editor-ui   = { git = "https://github.com/Adelodunpeter25/gpui-editor.git" }
```

Cargo resolves each crate by name from the workspace root automatically —
no `path =` needed even though they live under `crates/`. Add `rev = "..."`
once you've picked a commit to pin to, same reasoning as pinning `gpui`
below: floating on this repo's `main` means a future change here could
shift under you without warning. The rest of this guide covers `editor-ui`
specifically, since that's the one with GPUI-integration gotchas;
`edit-buffer` and `syntax` are plain Rust crates with no special setup
beyond adding the dependency.

## 1. Pin the same `gpui` revision — this is the part that actually breaks

(Only relevant if you're pulling in `editor-ui`, which is the only crate of
the three with a `gpui` dependency at all.)

`gpui`/`gpui_platform` aren't on crates.io with real semver; they're pinned
to an exact git commit here:

```toml
# your host app's Cargo.toml
gpui = { version = "0.2.2", git = "https://github.com/zed-industries/zed", rev = "5631830c564afa89b3aba679f45d9c3345f9460f" }
gpui_platform = { version = "0.1.0", git = "https://github.com/zed-industries/zed", rev = "5631830c564afa89b3aba679f45d9c3345f9460f", features = ["font-kit", "runtime_shaders"] }

editor-ui = { git = "<this repo's url>" } # or { path = "..." } in a workspace
```

**Your host app must pin the identical `rev`.** This isn't optional
hygiene — floating on zed's `main` branch (no `rev` pinned) is exactly what
broke this repo's text rendering entirely earlier on (every glyph silently
failed to paint, no panic, no error). Two different revs can also just fail
to compile together (mismatched types at the same `"0.2.2"` label), since
there's no real version compatibility guarantee between commits.

## 2. Register a monospace font

`editor-ui` hardcodes the font family name `"JetBrains Mono"` for editor
rows (`crates/editor-ui/src/lib.rs`, `render_line`) but does **not** bundle
or register the font itself — only `editor-demo` does that, since font
bundling is an app concern (which weights, license, bundle size). If your
host app doesn't already have a font registered under that exact family
name, editor text will fall back to a font gpui *can* resolve, which won't
be monospace and will misalign the line-number gutter.

```rust
// once at startup, before opening any window
static FONT_JETBRAINS_MONO_REGULAR: &[u8] =
    include_bytes!("../assets/fonts/JetBrainsMono-Regular.ttf");

app.run(|cx: &mut App| {
    cx.text_system()
        .add_fonts(vec![std::borrow::Cow::Borrowed(FONT_JETBRAINS_MONO_REGULAR)])
        .expect("failed to register editor font");
    // ...
});
```

If you want a different monospace font, there's currently no
`with_font_family()` setter — you'd need to change the hardcoded string in
`editor-ui` (a small, worthwhile addition if you need this; not built since
no consumer has needed it yet).

## 3. Bind the crate's actions once

```rust
editor_ui::init(cx); // binds cmd/ctrl-c (copy), cmd/ctrl-a (select all)
```

Call this once at app startup, same as `cx.bind_keys(...)` for your own
actions. Without it, copy/select-all silently do nothing (the actions exist
on `EditorView` but have no keys bound to fire them).

## 4. Create state + view

```rust
use editor_ui::{EditorState, EditorView};

let state = cx.new(|_| {
    let registry = syntax::LanguageRegistry::builtin();
    EditorState::readonly(source_text, registry.for_extension("rs"))
});
let view = cx.new(|cx| EditorView::new(&state, cx));

// anywhere in your own Render impl:
div().size_full().child(view.clone())
```

`EditorState::readonly` disables undo history (lower RAM) and is the only
fully-implemented mode today — `EditorState::editable()` exists but there's
no cursor/input handling wired up in `EditorView` yet (see
`implementation.md` §7).

## 5. Update the editor from outside (e.g. your own "open file" flow)

```rust
state.update(cx, |editor, _cx| {
    editor.set_text(&new_content);
    editor.set_language(registry.for_extension("py"));
});
```

Any mutation via `Entity::update` triggers a window redraw on the next
frame — you don't need to manually notify `EditorView`.

## 6. Optional: theme and word wrap

```rust
state.update(cx, |editor, _cx| {
    editor.set_theme(syntax::ThemePreset::Dracula); // default is GitHubDark
    editor.set_wrap_enabled(true);                  // default is off (clip long lines)
});
```

Syntax colors come from the real `lumis` theme, not a hand-picked palette —
switching `ThemePreset` changes every scope's actual color, not just a
handful of named tokens.

## Not yet available (don't build against these expecting them to exist)

- No cursor, no typing, no `cmd-z` — readonly viewer only.
- No search overlay, no bracket-match rendering (the model methods exist on
  `EditorState`, nothing calls them from `EditorView` yet).
- No `with_font_family()` — font is hardcoded to `"JetBrains Mono"` (§2).
- No diff view (`diff.md` has the research + plan, not implemented).

Selection has a known rough perf edge during drag (flagged, not yet
profiled/fixed) — fine for normal use, worth knowing about if you're
stress-testing.
