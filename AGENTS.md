# AGENTS.md

This repo is a fast, low-RAM / low-CPU GPUI code editor + syntax package.
Follow `implementation.md` for architecture and milestones.

## Testing — no inline tests

- NEVER add inline tests in source files. No `#[cfg(test)]`, no `mod tests` inside `src/*.rs`.
- ALL tests live in each crate's `tests/` folder as integration tests:
  - `crates/edit-buffer/tests/*.rs`
  - `crates/syntax/tests/*.rs`
  - `crates/editor-ui/tests/*.rs`
- Each `tests/*.rs` file tests the crate's public API only. Do not reach into privates.
- UI tests use `#[gpui::test]` per the gpui 0.2.2 docs.
- Run with `cargo test -p <crate>` or `cargo test --workspace`.
- When moving or adding code, move existing inline tests to the crate's `tests/` folder instead of deleting coverage.

## Commits — single line, every task

- After every task, commit with a SINGLE LINE commit message (no body, no bullets).
- Keep it under 72 chars, imperative mood, e.g. `Move editor tests to tests folders`.
- One commit per task; never batch unrelated tasks into one commit.
