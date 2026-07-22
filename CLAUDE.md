# CLAUDE.md

A GPUI (gpui) file/grep picker styled after Zed. Rust, edition 2024.

## Working-tree safety (read first)

NEVER run `git checkout`, `git restore`, `git reset --hard`, `git stash`, or
anything else that reverts working-tree files without first confirming the
changes are not uncommitted in-progress work. This has destroyed real work in
this repo before. When in doubt, ask — the tree usually holds unsaved effort.

## Quality gates (completion criteria)

A change is done only when all three pass:

- `cargo test` — all green (extend, don't regress the suite).
- `cargo build` — warning-free (warnings are treated as failures).
- `cargo fmt --check` — clean.

## Testing policy

GPUI render closures are effectively untestable. Extract pure logic into
gpui-free modules and unit-test that instead of the render code:

- `src/layout.rs` — viewport sizing, pane split/clamp, gutter/row-chrome math.
- `src/rows.rs` — result-row model (headers, matches, separators, selection).
- `src/path_shortening.rs` — directory truncation helpers (unit-tested; extend
  the `#[cfg(test)]` coverage alongside any behavior change).

Pattern: a `render_*` method computes nothing non-trivial inline — it calls a
pure helper that has its own `#[cfg(test)]` tests.

## Architecture

- `picker.rs` — orchestration + all GPUI render/event wiring (the big module).
- `ui.rs` — Zed-style component builders (disclosure, checkbox, icon_button);
  render-only, no unit tests per the GPUI policy above.
- `layout.rs`, `rows.rs`, `path_shortening.rs` — pure, unit-tested helpers.
- `preview.rs` — tree-sitter syntax highlighting; compiled
  `HighlightConfiguration`s are cached per language (compiling one is ~10ms).
- `theme.rs` — syncs Zed theme (colors, syntax tokens, `theme_overrides`).
  All palette/syntax colors are `u32` in `0xRRGGBBAA` form, painted with gpui
  `rgba()` (never `rgb()`); alpha blends live at paint time, nothing pre-blends.
- `editor.rs`, `service.rs`, `hotkey.rs`, `menubar.rs`, `config.rs` — open in
  editor, background search service, global hotkey, menu bar, config loading.

## Dependencies

`gpui` is a git dependency on `zed-industries/zed`, pinned via `Cargo.lock`.
Bump with `cargo update -p gpui` and expect API drift — fix the fallout at the
call sites rather than pinning around it.

`gpui_platform` is a second git dependency from the same `zed-industries/zed`
repo (`features = ["font-kit"]`). It supplies the `gpui_platform::application()`
entry point used in `main.rs`; keep it revision-aligned with `gpui` when bumping.
