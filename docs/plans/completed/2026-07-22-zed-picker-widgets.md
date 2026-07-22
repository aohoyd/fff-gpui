# Zed picker widget parity

> **For Claude:** use `/planning:execute` to implement this plan task-by-task with fresh subagents.

**Goal:** Bring five picker UI details into visual parity with Zed's picker-based Text Finder: 12px grep file-header font, SVG fold toggle, Zed-spec checkbox, always-visible multiselect-mode toggle button, and a 50/50 default preview split.

**Architecture:** New `src/ui.rs` module with Zed-style component builders (`disclosure`, `checkbox`, `icon_button`) returning styled `Stateful<Div>`s; `picker.rs` call sites attach their own `.on_click` handlers so all orchestration/event wiring stays in `picker.rs`. Four SVGs vendored from Zed's `assets/icons/` into `vendor/zed/icons/`, embedded via a new `#[include]` on the existing `Assets` embed. The only pure-logic change is `DEFAULT_RESULTS_FRACTION` 0.70 → 0.50 in `layout.rs`.

**Tech Stack:** Rust (edition 2024), gpui (git dep on zed-industries/zed), rust_embed.

**Design:** docs/plans/2026-07-22-zed-picker-widgets-design.md

## Overview
- Match Zed's Text Finder picker visuals, verified against Zed sources at `~/Git/github.com/zed-industries/zed`:
  1. Grep file-header filename drops from 14px (`.text_sm()`) to 12px (`.text_xs()`) — same size as the muted directory text, color the only differentiator (`crates/search/src/text_finder/delegate.rs:1133-1145`).
  2. Fold toggle becomes a Disclosure-style icon button: 14px chevron SVGs, hover background (`crates/ui/src/components/disclosure.rs`).
  3. Multiselect checkbox becomes Zed-spec: 16px box, 2px radius, 1px `border`, checked = 14px `check.svg` in `text_accent` on subtle fill (`crates/ui/src/components/toggle.rs`).
  4. New always-visible multiselect-mode toggle button (`file_multiple.svg`) at the trailing edge of the query input; accent tint when active; click = existing `ToggleMultiSelectMode` (`crates/picker/src/render.rs:111-142`).
  5. Preview pane default split changes from 70/30 to 50/50 (`crates/picker/src/shape.rs:647-671`).
- Purely visual pass: no changes to fold semantics, selection keys, or multiselect-mode state.

## Context (from discovery)
- Files involved: `src/picker.rs` (render sites: header row ~2236-2273, checkbox ~496-529 + call sites ~2352-2364 / ~2476-2488, input row ~2700), `src/layout.rs:27` (`DEFAULT_RESULTS_FRACTION`), `src/assets.rs:11-14` (`Assets` embed), new `src/ui.rs`, new `vendor/zed/icons/`.
- Theme tokens already available in `src/theme.rs` (`AppTheme`): `text_accent`, `hover_row`, `border`, `icon_muted`, `bg`.
- Zed's Disclosure chevrons (`assets/icons/chevron_*.svg`) differ slightly from the `file_icons/` chevrons we already vendor — must copy the `assets/icons/` variants.
- gpui tints monochrome SVGs via `.text_color()`; no color edits to vendored files needed.
- Project testing policy (CLAUDE.md): GPUI render code is untestable; pure logic lives in `layout.rs`/`rows.rs` with `#[cfg(test)]` tests.

## Development Approach
- **testing approach**: Regular (code first, then tests in the same task)
- complete each task fully before moving to the next
- make small, focused changes
- **CRITICAL: every task MUST include new/updated tests** for code changes in that task
  - tests are not optional - they are a required part of the checklist
  - exception per project policy: gpui render code (`src/ui.rs`, `picker.rs` render methods) is untestable; those tasks instead verify via `cargo build` (warning-free) and keep any testable math in `layout.rs`
  - tests cover both success and error scenarios
- **CRITICAL: all tests must pass before starting next task** - no exceptions
- **CRITICAL: update this plan file when scope changes during implementation**
- run tests after each change
- maintain backward compatibility (config `picker_pane_width` and session drag precedence unchanged)

## Testing Strategy
- **unit tests**: required for every task — see the testing mandate in Development Approach above
- **e2e tests**: none in this project (GPUI app, no e2e harness)
- quality gates on every task (CLAUDE.md): `cargo test` green, `cargo build` warning-free, `cargo fmt --check` clean

## Progress Tracking
- mark completed items with `[x]` immediately when done
- add newly discovered tasks with ➕ prefix
- document issues/blockers with ⚠️ prefix
- update plan if implementation deviates from original scope
- keep plan in sync with actual work done

## Solution Overview
- Component-module approach (chosen over in-place edits): `src/ui.rs` mirrors Zed's component library at miniature scale so future Zed-parity work has a home.
- Builders return `Stateful<Div>` with the id attached; call sites chain `.on_click(...)` — no callback plumbing across the module boundary.
- No new state anywhere: toggle button reads existing `multi_select_mode`, disclosure reads `collapsed`, checkbox reads `selection`.

## Technical Details
- **Asset keys** (match Zed's convention): `icons/chevron_down.svg`, `icons/chevron_right.svg`, `icons/check.svg`, `icons/file_multiple.svg`; embedded from `vendor/zed/icons/` via `#[include = "icons/**/*"]` on the `Assets` struct (folder is already `vendor/zed`).
- **`ui::disclosure(id, expanded, &theme)`**: 16×16 button, `rounded(px(2.0))`, hover bg `theme.hover_row`; child svg 14px, path `icons/chevron_down.svg` when expanded / `icons/chevron_right.svg` when collapsed, tinted `theme.icon_muted`.
- **`ui::checkbox(id, checked, &theme)`**: 16px outer slot; 16×16 box, `rounded(px(2.0))`, `border_1()` `theme.border`; unchecked: transparent; checked: fill `theme.hover_row` (deterministic stand-in for Zed's `element_background` — subtle, not accent) + 14px svg `icons/check.svg` tinted `theme.text_accent`.
- **`ui::icon_button(id, icon_path, active, &theme)`**: square button sized to fit the input row, hover bg `theme.hover_row`, `rounded(px(2.0))`; svg 14px tinted `theme.icon_muted` when idle, `theme.text_accent` when `active`.
- **Font**: `render_header_row` filename div `.text_sm()` → `.text_xs()` (`src/picker.rs:2258`); directory stays `.text_xs()` muted.
- **Split**: `DEFAULT_RESULTS_FRACTION` `0.70` → `0.50` (`src/layout.rs:27`); precedence unchanged (session drag → `picker_pane_width` config → default); double-click divider reset lands at 50/50 automatically.

## What Goes Where
- **Implementation Steps** (`[ ]` checkboxes): code changes, tests, docs updates in this repo
- **Post-Completion** (no checkboxes): manual visual verification against Zed side-by-side

## Implementation Steps

### Task 1: Vendor Zed UI icon SVGs and extend the asset embed

**Files:**
- Create: `vendor/zed/icons/chevron_down.svg`, `vendor/zed/icons/chevron_right.svg`, `vendor/zed/icons/check.svg`, `vendor/zed/icons/file_multiple.svg`
- Modify: `src/assets.rs`

- [x] copy the four SVGs verbatim from `~/Git/github.com/zed-industries/zed/assets/icons/` (NOT the `file_icons/` subdirectory — the chevron paths differ) into `vendor/zed/icons/`
- [x] add `#[include = "icons/**/*"]` to the `Assets` embed in `src/assets.rs:11-14`
- [x] write a `#[cfg(test)]` test in `src/assets.rs` asserting `Assets::get("icons/<name>.svg")` returns `Some` for all four keys (mirrors Zed's `test_all_icons_exist`; also covers a negative case, e.g. `icons/nonexistent.svg` → `None`)
- [x] run `cargo test` + `cargo build` — green/warning-free before task 2

### Task 2: Create the `src/ui.rs` component module

**Files:**
- Create: `src/ui.rs`
- Modify: `src/main.rs` (register `mod ui;`)

- [x] implement `disclosure(id, expanded, &theme)` per Technical Details spec
- [x] implement `checkbox(id, checked, &theme)` per Technical Details spec
- [x] implement `icon_button(id, icon_path, active, &theme)` per Technical Details spec
- [x] each builder returns `Stateful<Div>` with id attached; no click handlers, no state, no logic beyond styling (render-only per project GPUI policy — no unit tests possible; `cargo build` warning-free is the gate)
- [x] run `cargo build` + `cargo fmt --check` — clean before task 3 (temporary `#[allow(dead_code)]` acceptable until call sites land in tasks 3-6; remove by task 6)

### Task 3: Swap the fold toggle to `ui::disclosure`

**Files:**
- Modify: `src/picker.rs` (render_header_row, ~2236-2253)

- [x] replace the Unicode `\u{02C3}`/`\u{02C5}` glyph div with `ui::disclosure(("chevron", row_ix), !is_collapsed, &theme)`
- [x] re-attach the existing `.on_click` (stop_propagation; `toggle_group_fold` on click, `toggle_all_folds` on alt-click) — behavior and keybindings (`alt-z`, `alt-shift-z`) unchanged
- [x] confirm the 16px leading slot still aligns with `layout::ROW_LEADING_SLOT` math (no constant change expected; if a size constant must change, move it to `layout.rs` with a test)
- [x] run `cargo test` + `cargo build` — green/warning-free before task 4

### Task 4: Replace the hand-drawn checkbox with `ui::checkbox`

**Files:**
- Modify: `src/picker.rs` (delete `render_checkbox` ~496-529; call sites ~2352-2364 grep rows, ~2476-2488 files rows)

- [x] replace both `render_checkbox` call sites with `ui::checkbox(...)` + existing `.on_click` wiring (`toggle_key_at`)
- [x] delete the old `render_checkbox` helper
- [x] verify header rows still get no checkbox and the 24px `ROW_LEADING_SLOT` budget still holds with the 16px box (no truncation-math change expected)
- [x] run `cargo test` + `cargo build` — green/warning-free before task 5

### Task 5: Drop grep header filename to 12px

**Files:**
- Modify: `src/picker.rs` (render_header_row, ~2258)

- [x] change the filename div from `.text_sm()` to `.text_xs()`; directory text stays `.text_xs()` muted
- [x] check `render_header_row`'s truncation/char-width math for any assumption tied to the 14px filename (e.g. `char_px` usage at ~2189) and adjust in `layout.rs` with a test if needed
- [x] run `cargo test` + `cargo build` — green/warning-free before task 6

### Task 6: Add the multiselect-mode toggle button to the query-input row

**Files:**
- Modify: `src/picker.rs` (search-row div ~2722, text-field child ~2751; toggle appends after it)

- [x] add trailing `ui::icon_button(("multi-select-toggle"), "icons/file_multiple.svg", self.multi_select_mode, &theme)` to the query-input row, visible in both grep and files modes
- [x] wire `.on_click` to the existing `on_toggle_multi_select_mode` path (same effect as cmd-shift-s) with `cx.stop_propagation()` so root handlers don't fire; ensure focus stays on the query input after click (runtime check — `install_focus_lost_dismiss` must not trigger)
- [x] remove any `#[allow(dead_code)]` left in `src/ui.rs` from task 2
- [x] run `cargo test` + `cargo build` — green/warning-free before task 7

### Task 7: Change default preview split to 50/50

**Files:**
- Modify: `src/layout.rs`

- [x] change `DEFAULT_RESULTS_FRACTION` from `0.70` to `0.50` (`src/layout.rs:27`)
- [x] update split tests to the new default (1000px modal → ~499.5px results / ~499.5px preview): `split_defaults_to_seventy_thirty` at `src/layout.rs:324-331` (rename to `split_defaults_to_fifty_fifty`) AND `reset_split_restores_default` at `src/layout.rs:390-395` (hardcodes 699.3/299.7 — will fail otherwise); min-width/clamp tests at `src/layout.rs:364-376` pass unchanged (clamp is fraction-independent) — re-check only
- [x] update stale doc comments referencing 70/30 in `src/layout.rs`: `split` doc (~line 110, "wins over the 70/30 default") and `reset_split` doc (line 132) → 50/50
- [x] verify precedence tests still cover: session drag → `picker_pane_width` config → 50/50 default (add a test case if precedence isn't already covered)
- [x] run `cargo test` — all green before task 8

### Task 8: Verify acceptance criteria
- [x] verify all five parity changes from Overview are implemented (evidence: header 12px `picker.rs:2208`; disclosure `ui.rs:14-36` + `picker.rs:2194-2203`; checkbox `ui.rs:42-62` + `picker.rs:2304`/`2427`, old `render_checkbox` deleted; toggle `ui.rs:68-90` + `picker.rs:2708-2723`; `DEFAULT_RESULTS_FRACTION = 0.50` `layout.rs:27`)
- [x] verify edge cases: narrow modal (min-width clamping at 50/50 default), folded groups with multiselect active, toggle button in both grep and files modes
- [x] run full quality gates: `cargo test` green (294 passed), `cargo build` warning-free (0 warnings on forced recompile), `cargo fmt --check` clean
- [ ] launch the app and visually sanity-check all five changes
  - ⚠️ partial: `./target/debug/fff-gpui --open .` launched and ran 8s without panicking (no crash, no early exit), but a real visual inspection was not possible from the agent context — `screencapture` failed with "could not create image from display" (no screen-recording permission). Manual side-by-side check remains (see Post-Completion).

### Task 9: [Final] Update documentation
- [x] update README.md if it mentions the 70/30 preview default (change to 50/50)
- [x] update CLAUDE.md architecture list: add `ui.rs` (Zed-style component builders, render-only)
- [x] move this plan to `docs/plans/completed/` (create dir if needed)
- [x] move the linked design file too: `test -f docs/plans/2026-07-22-zed-picker-widgets-design.md && mv docs/plans/2026-07-22-zed-picker-widgets-design.md docs/plans/completed/`

## Post-Completion
*Items requiring manual intervention - no checkboxes, informational only*

**Manual verification:**
- side-by-side visual comparison with Zed's Text Finder (header font, chevrons, checkbox, toggle tint) in both light and dark themes
- confirm divider double-click resets to 50/50 and drag/config overrides still win
