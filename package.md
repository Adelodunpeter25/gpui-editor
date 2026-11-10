# Embedding `editor-ui` in another app

A short guide for dropping this crate's code viewer into an existing GPUI
app. See `todo.md` for what's actually implemented vs planned.

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
- **`editor-ui`** — the full GPUI scrollable code *editor*: virtualization,
  scrollbar, mouse selection, word wrap, and (opt-in per state) typing,
  caret motion, and undo/redo. Depends on the two crates above.
  Pull this in only when you actually want a full embeddable file-viewer
  widget, not just colored text — it carries real weight (scroll/selection
  machinery) that's wasted on something like a short static code snippet.

This repo isn't published to crates.io — pull in whichever crate(s) you need
as a `git` dependency, pointing at the subdirectory. `crates/editor-demo` is
a small desktop app wiring all three together (file open, toolbar, editing)
— the reference integration if you want to see the pieces used for real:

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
gpui_platform = { version = "0.1.0", git = "https://github.com/zed-industries/zed", rev = "5631830c564afa89b3aba679f45d9c3345f9460f", features = ["font-kit", "runtime_shaders", "x11", "wayland"] }

editor-ui = { git = "<this repo's url>" } # or { path = "..." } in a workspace
```

**Your host app must pin the identical `rev`.** This isn't optional
hygiene — floating on zed's `main` branch (no `rev` pinned) is exactly what
broke this repo's text rendering entirely earlier on (every glyph silently
failed to paint, no panic, no error). Two different revs can also just fail
to compile together (mismatched types at the same `"0.2.2"` label), since
there's no real version compatibility guarantee between commits.

Linux hosts additionally need the `x11` and/or `wayland` feature on
`gpui_platform` — without one of them the platform panics at startup
("At least one of the \"wayland\" or \"x11\" features must be enabled").

## 2. Font: configurable, but registration is still on you

`editor-ui` defaults to `"JetBrains Mono"` at 14px (22px line height), but
this is a real, overridable `FontConfig` now — not a hardcoded string
scattered across paint/measure/wrap call sites (an earlier version of this
crate had exactly that bug: three separate literals that could silently
disagree with each other). Override it via `EditorState`/`diff::DiffState`:

```rust
use editor_ui::FontConfig;

state.update(cx, |editor, _cx| {
    editor.set_font(FontConfig {
        family: "Menlo".into(), // or any family gpui can resolve
        size: gpui::px(15.0),
        line_height: gpui::px(24.0),
    });
});
```

`editor-ui` still does **not** bundle or register any font file itself —
that stays an app concern (which weights, license, bundle size). If you
pick a family gpui can't resolve (not a system font, not registered via
`add_fonts`), the render falls back to whatever gpui's fallback stack
resolves instead, which likely won't be monospace and will misalign the
line-number gutter — same failure mode as before, just now something you
opt into by picking a bad family name rather than something baked in.

```rust
// once at startup, before opening any window, if bundling your own font
static FONT_JETBRAINS_MONO_REGULAR: &[u8] =
    include_bytes!("../assets/fonts/JetBrainsMono-Regular.ttf");

app.run(|cx: &mut App| {
    cx.text_system()
        .add_fonts(vec![std::borrow::Cow::Borrowed(FONT_JETBRAINS_MONO_REGULAR)])
        .expect("failed to register editor font");
    // ...
});
```

A system-installed monospace font (e.g. `"Menlo"` on macOS) needs no
`add_fonts` call at all — gpui resolves it directly.

## 3. Bind the crate's actions once

```rust
editor_ui::init(cx); // binds copy/select-all (cmd/ctrl-c, cmd/ctrl-a),
                     // undo/redo/cut/paste (cmd/ctrl-z, shift-z, x, v),
                     // and save (cmd/ctrl-s)
```

Call this once at app startup, same as `cx.bind_keys(...)` for your own
actions. Without it, those actions exist on `EditorView` but have no keys
bound to fire them — copy/select-all, the edit-mode clipboard/undo combos,
and save silently do nothing.

All bindings are scoped to the `"Editor"` key context, so they only fire
while an editor is focused and don't collide with your app's own
`cmd-s`/`cmd-z` elsewhere. To use different keys, skip `init` and bind the
exported actions (`editor_ui::{Copy, Cut, Paste, Undo, Redo, SelectAll,
Save}`) yourself.

## 4. Create state + view

```rust
use editor_ui::{EditorState, EditorView};

let state = cx.new(|_| {
    let registry = syntax::LanguageRegistry::builtin();
    EditorState::readonly(source_text, registry.for_extension("rs"))
    // or EditorState::editable(source_text, registry.for_extension("rs"))
    // for typing, caret, and undo/redo (see §3 for the init() requirement)
});
let view = cx.new(|cx| EditorView::new(&state, cx));

// anywhere in your own Render impl:
div().size_full().child(view.clone())
```

Both modes work: `EditorState::readonly` disables undo history (lower RAM);
`EditorState::editable(...)` additionally wires typing, caret motion,
Enter/Tab auto-indent, undo/redo, and saving (§8). Keys reach the buffer
only while the view is focused — clicking into it focuses it, or call
`view.focus(window, cx)` yourself after opening.

Text entry goes through the platform's text-input system, so dead-key
accents, press-and-hold variants, the emoji picker, and IME composition
(Chinese/Japanese/Korean) work; text still being composed is underlined
and becomes a single undo entry once committed.

Every edit notifies the state's observers, so the view repaints on its own
and `cx.observe(&state, …)` is all a host needs to react to changes.

## 5. Select and scroll to a line

`EditorState::select_line` selects the text of a 1-based line number, excluding
its trailing newline. `EditorView::select_line` additionally scrolls that line
into the center of the viewport. Both methods clamp line numbers outside the
buffer to the nearest available line.

Because selection and scrolling belong to the retained view, call the view
method through its GPUI entity:

```rust
view.update(cx, |editor_view, cx| {
    editor_view.select_line(42, cx); // line numbers are 1-based
});
```

For scrolling without changing the selection, use a zero-based buffer row and
choose a `gpui::ScrollStrategy`:

```rust
view.update(cx, |editor_view, _cx| {
    editor_view.scroll_to_row(41, gpui::ScrollStrategy::Center);
});
```

If a host needs to coordinate another scroll control, the underlying
`gpui::UniformListScrollHandle` is available with
`editor_view.scroll_handle()`.

## 6. Comment gutter annotations

The file-view path has an opt-in gutter annotation primitive for review and
AI-comment flows. It is disabled by default, works in both readonly and
editable mode, and is automatically disabled while word wrap is enabled
because the unwrapped row-to-screen mapping is required for accurate range
selection.

```rust
use editor_ui::{CommentTarget, EditorState};

state.update(cx, |editor, cx| {
    editor.set_comment_gutter(true, cx);
    editor.set_comment_target_handler(Some(Box::new(|target: CommentTarget| {
        // Open your host app's comment composer for this range.
        // Lines are 1-based and inclusive.
        println!("comment on lines {}–{}", target.start_line, target.end_line);
    })));
});
```

The builder form is also available when constructing state:

```rust
let state = cx.new(|_| {
    EditorState::readonly(source_text, language).with_comment_gutter(true)
});
```

Hover a code row or its line-number gutter to reveal the `+` chip. Press and
drag from the chip to select a contiguous range; releasing invokes the
callback once with the final `CommentTarget`. A single-line target is reported
as `start_line == end_line`. Escape cancels an in-progress drag. The selected
rows use a fixed translucent yellow review highlight, and the chip uses a fixed
yellow accent independent of the syntax theme. The editor does not own or
render the host comment composer—the callback is the integration seam.

## 7. Update the editor from outside (e.g. your own "open file" flow)

```rust
state.update(cx, |editor, cx| {
    editor.set_text(&new_content, cx);
    editor.set_language(registry.for_extension("py"), cx);
});
```

`set_text` loads a *different document*: it resets the selection, clears
undo history, and marks the buffer clean. Don't call it to refresh a file
the user may be editing — it replaces their unsaved changes. Check
`editor.is_dirty()` first. For an edit that should be undoable, use
`replace_range`.

Any mutation via `Entity::update` triggers a window redraw on the next
frame — you don't need to manually notify `EditorView`.

`set_text`/`set_language`/`set_theme`/`insert` all take the `cx` from your
`update` closure (previously unused, often named `_cx`) — they use it to
run syntax highlighting on a background thread for files over ~64KB, so a
large file doesn't stall the window while it parses (see
`implementation.md` §6). If your code still has `|editor, _cx|`, rename it
to `|editor, cx|` and pass `cx` through.

## 8. Saving

The editor can save on its own. Bind it to a file and cmd/ctrl-s writes
the buffer back:

```rust
// Reads the file and remembers its path:
let state = cx.new(|_| EditorState::open("src/main.rs", language))?; // io::Result

// …or attach a path to text you loaded yourself:
let state = cx.new(|_| EditorState::editable(&text, language).with_path(path));
state.update(cx, |editor, _| editor.set_path(Some(new_path)));
```

The write is atomic (temp file + rename, so a crash can't truncate the
file), follows symlinks, keeps the file's permissions, and writes the line
ending the file was loaded with. `editor.save(cx)` does the same thing
programmatically and returns `false` if there was nothing to save with.

**Dirty state.** `editor.is_dirty()` is true when the text differs from
what was last saved or loaded; undoing back to the saved text reads as
clean again. It changes inside ordinary notifications, so an observer is
enough to drive a "modified" dot.

**Results.** Subscribe for the outcome:

```rust
use editor_ui::EditorEvent;

cx.subscribe(&state, |this, _state, event: &EditorEvent, cx| match event {
    EditorEvent::Saved => { /* clear your error banner */ }
    EditorEvent::SaveFailed(message) => { /* show it */ }
})
.detach();
```

**Saving somewhere else.** For remote files, a save-as dialog,
format-on-save, or anything that isn't "write to this local path", install
a handler; the editor then hands you the text instead of touching disk:

```rust
use editor_ui::{SaveRequest, SaveStatus};

state.update(cx, |editor, _| {
    editor.set_save_handler(Some(Box::new(|request: SaveRequest| {
        // request.text — exact bytes to persist (line ending applied)
        // request.path — the editor's path, if it has one
        my_backend.write(&request.text)?;
        Ok(SaveStatus::Saved)
    })));
});
```

If the write finishes later (async upload, dialog), return
`SaveStatus::Deferred` and call `editor.mark_saved(request.revision, cx)`
when it lands — passing the revision means edits made while the save was
in flight still read as unsaved.

With no path and no handler, or in a readonly editor, the save action is
not consumed: cmd/ctrl-s falls through to whatever your app binds.

**Line endings.** The buffer always holds `\n`: `editor.text()`, every
offset, and pasted text are normalized, so a caret can't land inside a
`\r\n`. The ending the file was loaded with (`editor.line_ending()`) is
re-applied on save; `editor.set_line_ending(LineEnding::Lf)` converts the
file on its next save. A file mixing endings is saved with its first one
throughout.

## 9. Optional: theme and word wrap

```rust
state.update(cx, |editor, cx| {
    editor.set_theme(syntax::ThemePreset::Dracula, cx); // default is GitHubDark
    editor.set_wrap_enabled(true);   // default is off (horizontal scroll) — no cx needed
});
```

Syntax colors come from the real `lumis` theme, not a hand-picked palette —
switching `ThemePreset` changes every scope's actual color, not just a
handful of named tokens.

## 10. Not yet available (don't build against these expecting them to exist)

- Edit mode is plain-text only: no search overlay, no bracket-match
  rendering (the model methods exist on `EditorState`, nothing calls them
  from `EditorView` yet).
- Editing covers the basics only. Not there yet: word/line motion and
  delete (alt/cmd + arrows or backspace move and delete one character),
  shift-click, double-click word select, PageUp/PageDown, and block
  indent (Tab over a selection replaces it; Shift+Tab is not outdent).
- One caret, one selection — no multi-cursor.
- Files are read and written as UTF-8 only; `EditorState::open` errors on
  anything else rather than guessing an encoding.
- Nothing watches the file on disk: a change made outside the editor is
  not noticed, and the next save overwrites it.
- Highlighting re-parses the whole buffer per edit. Above ~64KB that
  happens off-thread, so colors can lag a keystroke behind the text.
- The diff view stays readonly — `DiffState`/`DiffView`
  (`editor_ui::{DiffState, DiffView}`) highlight each
  line independently, not the full old/new text once — a multi-line
  construct spanning a change can highlight incorrectly on adjacent
  unchanged lines. See `diff_view.rs`'s module doc for the full tradeoff.

Selection has a known rough perf edge during drag (flagged, not yet
profiled/fixed) — fine for normal use, worth knowing about if you're
stress-testing.
