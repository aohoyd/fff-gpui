# Design: Dependency update + verify Zed theme/font sync

Date: 2026-06-28
Status: Approved (brainstorm)

## Goal

Update all dependencies to their latest versions (including `gpui`), and verify
that the existing Zed theme + font + icon sync still works after the bump. Font
sync already exists end-to-end; this task does **not** add a new sync feature —
it modernizes dependencies and proves no regression.

## Decisions (from brainstorm)

- **Dependency scope**: all crates to latest, including major-version bumps.
- **`gpui`**: already at `0.2.2`, the latest published version on crates.io
  (`*` does not move it further). Leave `gpui = "*"` as-is.
- **`fff-*`**: `fff-search`, `fff-query-parser`, `fff-grep` have a release newer
  than the current `0.8` — bump to latest, keep the `zlob` feature on
  `fff-search` and `fff-query-parser`.
- **Mechanism**: manual, staged `Cargo.toml` edits (not cargo-edit), building
  between groups to isolate breakage.
- **Breakage policy**: fix whatever it takes to compile on latest.
- **Font sync intent**: just verify it works (no new properties added).
- **Verification**: cargo build + cargo test + run the app against real Zed config.

## Section 1 — Staged dependency update

Build (`cargo build`, dev + release) and `cargo test` after **each** stage so
breakage is attributable to a single crate group.

1. **Semver-compatible sweep** — `cargo update`. Picks up all patch/minor bumps
   already available (zlob 1.3→1.5, wasm-bindgen, zerocopy, etc.) with no
   `Cargo.toml` edits. Establish a green baseline.
2. **`fff-*` crates** — bump `fff-search` / `fff-query-parser` / `fff-grep`
   from `0.8` to latest published (confirm exact version during impl). Keep the
   `zlob` feature. Verify the picker still returns search/grep results.
3. **tree-sitter group (together)** — `tree-sitter` core + `tree-sitter-highlight`
   + all 15 language grammars bumped as one group, since grammar crates link a
   specific core ABI. Highest-risk stage: recent tree-sitter releases renamed
   `language()` → `LANGUAGE` constants, so highlight wiring in `src/preview.rs`
   (and wherever grammars are registered) may need edits. Current grammars:
   bash, c, cpp, css, javascript, html, json, md, go, python, regex, rust,
   swift, typescript, yaml.
4. **Remaining majors** — `cocoa` 0.26→latest, `global-hotkey` 0.8→latest,
   `toml` 0.8→latest, `json5` 0.4→latest, `rust-embed` 8→latest, and any others
   not yet at latest. macOS crates (`cocoa`, `objc`) touch `src/menubar.rs` and
   window code — watch those call sites.
5. **`gpui`** — leave `gpui = "*"` (already latest 0.2.2). No change.

Exact latest versions per crate to be resolved during implementation (network
API was flaky during brainstorm).

## Section 2 — Verification & testing

### Automated
- `cargo build` (dev + release) and `cargo test` must pass after every stage.
- Existing tests already cover theme catalog loading, color parsing, and syntax
  resolution (`src/theme.rs` tests module).
- **New test (gap)**: add one unit test asserting font fields resolve from Zed
  settings — construct a `ZedSettings` with `ui_font_family`, `buffer_font_family`,
  `ui_font_size`, `buffer_font_size` and assert they propagate into the resolved
  `AppTheme` via `resolve_from_zed_settings` / the merge path. No new dependency.

### Manual (run the app)
- Launch the overlay against the real `~/.config/zed/settings.json`.
- Confirm: theme colors match the active Zed theme (light/dark/system follows OS),
  file icons match the Zed icon theme, UI font + buffer font family/size visibly
  match Zed, and syntax highlighting renders in the preview pane.
- Edge: a `buffer_font_family` value not installed on the system should fall back
  gracefully (gpui font fallback), not crash.

### Done criteria
All stages build, `cargo test` is green (including the new font test), and a
visual pass confirms theme + font + icons track Zed.

## Notes
- No commits during implementation unless explicitly requested.
- Relevant files: `src/theme.rs` (sync logic), `src/config.rs` (`FontConfig`,
  `AppConfig`), `src/picker.rs` / `src/text_field.rs` (font application),
  `src/preview.rs` (tree-sitter highlighting), `src/menubar.rs` (cocoa/objc),
  `Cargo.toml`.
