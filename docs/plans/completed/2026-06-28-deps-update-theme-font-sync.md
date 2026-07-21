# Dependency Update + Verify Zed Theme/Font Sync

> **For Claude:** use `/planning:execute` to implement this plan task-by-task with fresh subagents.

**Goal:** Update all crate dependencies to their latest versions and verify the existing Zed theme + font + icon sync still works, locking font sync against regression with a unit test.

**Architecture:** Staged manual `Cargo.toml` bumps applied crate-group at a time, building and testing between stages so any breakage is attributable to a single group. The highest-risk group is the tree-sitter core + grammar crates (shared ABI). No new sync feature is built — font sync already exists end-to-end; this modernizes deps and proves no regression.

**Tech Stack:** Rust (edition 2024), GPUI, tree-sitter (+15 grammars), fff-search/query-parser/grep, cocoa/objc (macOS).

**Design:** 2026-06-28-deps-update-theme-font-sync-design.md

## Overview
- Bring every dependency in `Cargo.toml` to its latest published version, including major bumps that require editing version constraints by hand.
- `gpui = "*"` already resolves to 0.2.2 (latest published) — leave as-is.
- `fff-search` / `fff-query-parser` / `fff-grep` move from `0.8` to latest (keep `zlob` feature).
- tree-sitter core + `tree-sitter-highlight` + all 15 language grammars move together; recent releases renamed `language()` → `LANGUAGE` constants, so highlight wiring may need edits.
- Verify theme + font + icon sync still works via `cargo test` (plus a new font-sync test) and a manual run against the real Zed config.

## Context (from discovery)
- Files/components involved: `Cargo.toml`, `src/theme.rs` (Zed sync logic), `src/config.rs` (`FontConfig`/`AppConfig`), `src/picker.rs` + `src/text_field.rs` (font application), `src/preview.rs` (tree-sitter highlighting), `src/menubar.rs` (cocoa/objc).
- Related patterns found: `ZedSettings` reads `ui_font_family`/`buffer_font_family`/`ui_font_size`/`buffer_font_size` and applies them; theme catalog loaded from built-in + installed + local Zed dirs; existing tests cover theme catalog, color parsing, syntax resolution.
- Dependencies identified: tree-sitter grammar crates share an ABI with the core crate (must bump together); `cocoa`/`objc` touch macOS window/menubar code.

## Development Approach
- **testing approach**: Regular (apply bumps, then verify via build + tests; add one new font-sync test).
- Complete each stage fully (build + `cargo test` green) before moving to the next.
- Make small, focused changes; one crate group per stage so breakage is isolated.
- **CRITICAL: every task with code changes includes tests** — here that means the new font-sync unit test (Task 6) and keeping all existing tests green after each bump stage.
- **CRITICAL: all tests must pass before starting the next task.**
- **CRITICAL: update this plan file if scope changes during implementation.**
- Fix whatever API breakage latest versions introduce in order to compile.

## Testing Strategy
- **unit tests**: existing `src/theme.rs` tests must stay green after every bump; add one new test asserting font fields resolve from Zed settings.
- **e2e tests**: project has none. Manual run of the overlay against the real Zed config substitutes (see Post-Completion).

## Progress Tracking
- Mark completed items with `[x]` immediately when done.
- Add newly discovered tasks with ➕ prefix; blockers with ⚠️ prefix.
- Record the exact resolved version per crate in this plan as bumps are applied.

## Solution Overview
- Sweep semver-compatible updates first to establish a green baseline, then apply major bumps group-by-group.
- Keep `gpui` untouched (already latest). Keep `zlob` features on the fff search crates.
- Add a regression test for font sync, then confirm visually that theme/font/icons track Zed.

## Technical Details
- `cargo update` only moves within existing constraints; major bumps require editing the version strings in `Cargo.toml`.
- tree-sitter grammar crates expose grammars via either `fn language()` or a `LANGUAGE` constant depending on version; `src/preview.rs` (and any grammar registration) must match whatever the bumped versions expose.
- Font fields flow: `ZedSettings` (`src/theme.rs`) → `resolve_from_zed_settings` → `AppTheme` → applied in `src/picker.rs` / `src/text_field.rs`.

## What Goes Where
- **Implementation Steps**: `Cargo.toml` edits, code fixes for API breakage, the new test, doc updates.
- **Post-Completion**: manual visual verification of the running overlay against the real Zed config.

## Implementation Steps

### Task 1: Semver-compatible baseline sweep

**Files:**
- Modify: `Cargo.lock`

- [ ] run `cargo update` to pull all patch/minor bumps within current constraints
- [ ] run `cargo build` (dev) and `cargo build --release`
- [ ] run `cargo test` — establish green baseline
- [ ] record notable lockfile version changes in this plan
- [ ] tests must pass before next task

### Task 2: Bump fff-search / fff-query-parser / fff-grep

**Files:**
- Modify: `Cargo.toml`
- Modify: `Cargo.lock`

- [ ] determine latest published version of `fff-search`, `fff-query-parser`, `fff-grep`
- [ ] update the three version constraints in `Cargo.toml` (keep `zlob` feature on search + query-parser)
- [ ] fix any API breakage in search/grep call sites (`src/picker.rs` and callers)
- [ ] run `cargo build` + `cargo test`
- [ ] add/adjust tests if a search/grep API surface changed; otherwise confirm existing coverage still exercises it
- [ ] tests must pass before next task

### Task 3: Bump tree-sitter core + highlight + all 15 grammars (together)

**Files:**
- Modify: `Cargo.toml`
- Modify: `src/preview.rs`
- Modify: `Cargo.lock`

- [ ] bump `tree-sitter` + `tree-sitter-highlight` to latest in `Cargo.toml`
- [ ] bump all grammar crates to latest: bash, c, cpp, css, javascript, html, json, md, go, python, regex, rust, swift, typescript, yaml
- [ ] update grammar registration in `src/preview.rs` for any `language()` → `LANGUAGE` constant renames
- [ ] run `cargo build` + `cargo test`
- [ ] verify highlighting test paths still pass; add a regression case if a grammar wiring change is non-trivial
- [ ] tests must pass before next task

### Task 4: Bump remaining major crates

**Files:**
- Modify: `Cargo.toml`
- Modify: `src/menubar.rs` (if cocoa/objc APIs changed)
- Modify: `Cargo.lock`

- [ ] bump `cocoa`, `global-hotkey`, `toml`, `json5`, `rust-embed`, and any other crate not yet at latest
- [ ] fix API breakage (watch macOS window/menubar code in `src/menubar.rs`; config parsing for `toml`/`json5`)
- [ ] run `cargo build` (dev + release) + `cargo test`
- [ ] add/adjust tests for any changed config-parsing behavior
- [ ] tests must pass before next task

### Task 5: Confirm gpui status

**Files:**
- Modify: `Cargo.toml` (only if a newer gpui is found)

- [ ] confirm 0.2.2 is the latest published gpui (`cargo update` leaves it unchanged under `*`)
- [ ] if (and only if) a newer version exists, bump and fix breakage; otherwise leave `gpui = "*"` as-is and note it
- [ ] run `cargo build` + `cargo test`
- [ ] tests must pass before next task

### Task 6: Add font-sync regression test

**Files:**
- Modify: `src/theme.rs`

- [ ] add a unit test constructing a `ZedSettings` with `ui_font_family`, `buffer_font_family`, `ui_font_size`, `buffer_font_size`
- [ ] assert those values propagate into the resolved `AppTheme` (via `resolve_from_zed_settings` or the merge path)
- [ ] include a fallback case: missing font fields fall back to the documented defaults
- [ ] run `cargo test` — new test passes alongside existing theme tests
- [ ] tests must pass before next task

### Task 7: Verify acceptance criteria
- [ ] verify every dependency in `Cargo.toml` is at its latest intended version (record final versions here)
- [ ] run full suite: `cargo test`
- [ ] run `cargo build --release` clean (no warnings introduced by the bumps where avoidable)
- [ ] confirm no behavior regressions surfaced by existing tests

### Task 8: Update documentation
- [ ] update README.md build requirements if any bumped crate changed toolchain needs (e.g. Zig/tree-sitter notes, the `fff-search`/`fff-query-parser` version snippet currently shows 0.6)
- [ ] update CLAUDE.md if new patterns discovered (none expected)
- [ ] move this plan to `docs/plans/completed/` (create dir if needed)
- [ ] move the linked design file too: extract via `grep -E '^\*\*Design:\*\*' <plan-file> | sed 's/^\*\*Design:\*\* *//'`; if non-empty and not `none`, `test -f` then `mv` it into `docs/plans/completed/`

## Post-Completion
*Items requiring manual intervention or external systems — no checkboxes, informational only*

**Manual verification:**
- Launch the overlay against the real `~/.config/zed/settings.json`.
- Confirm theme colors match the active Zed theme (light/dark/system follows OS), file icons match the Zed icon theme, UI + buffer font family/size visibly match Zed, and syntax highlighting renders in the preview pane.
- Edge: a `buffer_font_family` not installed on the system falls back gracefully (gpui font fallback), not a crash.

**External system updates:**
- None — this is a leaf application, no consuming projects.
