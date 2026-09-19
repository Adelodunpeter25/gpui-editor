# Diff viewer — research notes (not implemented yet)

## Does lumis provide a diff algorithm?

**No.** Checked `lumis` 0.13.1 source
(`~/.cargo/registry/src/.../lumis-0.13.1/src/{lib,languages,themes,formatter/mod}.rs`)
for anything diff-related:

- The only "Diff" in lumis is `Language::Diff` in `languages.rs` — a
  tree-sitter **grammar for syntax-highlighting unified-diff/patch text**
  (`.diff` files: lines starting with `+`/`-`/`@@` colored appropriately).
  It highlights diff *output that already exists as text* — it does not
  compute a diff between two buffers.
- No Myers diff, no LCS, no line/word/char diffing algorithm, no patch
  application anywhere in the crate. `formatter/mod.rs` only formats
  already-highlighted spans (HTML/ANSI/etc.), unrelated to diffing.

**Conclusion:** lumis gives us "if I already have diff text, highlight it
nicely" — nothing else. Actually computing a diff (old text vs new text →
list of changed/added/removed line or char ranges) is a separate concern
this repo would own itself.

## What a future `diff` crate/module would need

When this gets built (later, not now), scope per this repo's existing
architecture (`edit-buffer`/`syntax`/`editor-ui` split, engine-agnostic core
+ thin GPUI view):

1. **Diff algorithm** — a `diff` crate (or module in `edit-buffer`?), GPUI-free,
   computing a `Vec<DiffHunk>` (line-level, Myers or similar) between two
   `Rope`/`&str` inputs. Candidates: hand-roll Myers, or depend on a small
   crate (`similar` is the common Rust choice — pure algorithm, no UI).
2. **Rendering** — a `DiffView` in `editor-ui` (or a new `diff-ui` crate)
   that reuses `EditorView`'s virtualized `uniform_list` row-rendering
   approach (crates/editor-ui/src/lib.rs) side-by-side or unified, painting
   add/remove/context line backgrounds — same `LINE_HEIGHT`/scroll-snapping
   pattern already built for the plain editor.
3. **Syntax coloring inside a diff** — once hunks are computed, each line's
   *content* (minus the +/-/context marker) can still go through
   `syntax::highlight_themed` exactly as today, so diff lines stay
   syntax-colored, not just add/remove-colored.
4. **Embeddable like the editor** — per `implementation.md`'s existing goal
   ("Embeddable: `Entity<EditorState>` + `EditorView`"), a diff viewer should
   follow the same pattern: `Entity<DiffState>` + `DiffView`, addable by a
   host app without pulling in anything else.

No timeline commitment — this file exists so the option is documented
and can be picked up later without re-researching lumis.
