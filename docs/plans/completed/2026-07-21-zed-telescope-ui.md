# Zed Telescope UI

> **For Claude:** use `/planning:execute` to implement this plan task-by-task with fresh subagents.

**Goal:** Restyle fff-gpui to match Zed's new telescope-like picker UI (input on top, top-down results, resizable preview with line numbers, viewport-relative sizing) plus a full-height theme-derived git edge bar per row.

**Architecture:** In-place restyle of the existing `FffPicker::render` function (no componentization), plus one new module `src/layout.rs` holding the viewport sizing / clamp / divider math. Theme tokens are extended through the existing `Palette` → `AppTheme` pipeline. The window itself (opaque `WindowKind::PopUp`, focus-loss dismiss) is unchanged — only bounds math and interior change.

**Tech Stack:** Rust, GPUI 0.2.2 (crates.io), tree-sitter highlighting, Zed theme JSON sync

**Design:** docs/plans/2026-07-21-zed-telescope-ui-design.md

## Overview
- Flips the current layout to Zed's orientation: 36px search row on top (goose kept), results top-down with best match first, preview on the right, restyled 28px status bar at the bottom.
- Window sizes to 60%×60% of the active display by default; existing `window_width`/`window_height`/`picker_pane_width` px config values become optional overrides (non-breaking).
- Preview gains an always-on line-number gutter, Zed-style match highlighting (full-line `active_line_bg` tint + flat bold search-match background), and match-centered scrolling; the 28px path header is removed.
- Results/preview divider becomes drag-resizable (session-only, clamped, double-click reset).
- The 3px×18px git tick grows to a full-row-height 3px edge bar with colors derived from the synced Zed theme's `version_control`/status keys (current hexes as fallback).

## Context (from discovery)
- Files involved: `src/picker.rs` (render :1407-2009, `git_status_bar_color` :327), `src/theme.rs` (`AppTheme` :59, `Palette` :37, `palette_from_style` :1493, `sync_from_config` :740), `src/preview.rs` (`highlight_file_window` :444, `overlay_match_ranges` :348), `src/text_field.rs`, `src/config.rs`, `src/main.rs` (`open_window` :464), `README.md`; new `src/layout.rs`.
- Zed reference values (from `~/Git/github.com/zed-industries/zed`, PR #59604): search row `h_9`/36px with 10px padding, 1px `border_variant` dividers, flat `ghost_element_selected` selection, `text_accent` fuzzy-char tint, preview split 30% (min results 280×320px, min preview 128×96px, max 95% viewport), match-centered scroll `target = match_row − (visible_rows − 1)/2`.
- Constraint: GPUI is the crates.io `gpui 0.2.2` package, not Zed's live source — divider drag/cursor APIs must be verified against it before building on them (Task 1); fallback is keyboard-only resize keybinds.
- Existing patterns: theme tokens flow `Palette` → `AppTheme` → Zed `theme_overrides` → `[theme]` config overrides, with tests at the bottom of `theme.rs`; git status strings already populate `FileItemSnapshot.git_status` in both file and grep paths.

## Development Approach
- **testing approach**: Regular (code first, then tests)
- complete each task fully before moving to the next
- make small, focused changes
- **CRITICAL: every task MUST include new/updated tests** for code changes in that task
  - tests are not optional - they are a required part of the checklist
  - write unit tests for new functions/methods
  - write unit tests for modified functions/methods
  - add new test cases for new code paths
  - update existing test cases if behavior changes
  - tests cover both success and error scenarios
  - GPUI render closures are not unit-testable — extract testable pure helpers (sizing, clamps, gutter width, scroll target, color mapping) and test those
- **CRITICAL: all tests must pass before starting next task** - no exceptions
- **CRITICAL: update this plan file when scope changes during implementation**
- run tests after each change (`cargo test`)
- maintain backward compatibility (existing config.toml files must behave identically)

## Testing Strategy
- **unit tests**: required for every task — see the testing mandate in Development Approach above (not restated here)
- **e2e tests**: none in this project; final visual verification is manual (see Post-Completion)

## Progress Tracking
- mark completed items with `[x]` immediately when done
- add newly discovered tasks with ➕ prefix
- document issues/blockers with ⚠️ prefix
- update plan if implementation deviates from original scope
- keep plan in sync with actual work done

## Solution Overview
- All chrome/spacing/color values are taken from Zed's picker implementation so the two UIs read identically; fff-gpui's theme sync already exposes most tokens, and the few missing ones (`elevated_surface_background` remap, `active_line_bg`, git status colors) are added to the pipeline.
- Sizing becomes viewport-relative with the same clamps Zed uses; a single pure module (`src/layout.rs`) owns that math so it is unit-testable and shared by window-bounds computation and divider dragging.
- The divider split lives as session state on `FffPicker` (no persistence, per design decision); `picker_pane_width` config, when set, seeds it.

## Technical Details
- `AppConfig`: `window_width: Option<f32>`, `window_height: Option<f32>`, `picker_pane_width: Option<f32>` (were defaulted floats).
- `src/layout.rs`: `modal_size(display: Size, cfg_w: Option<f32>, cfg_h: Option<f32>) -> Size` (60%×60% default, ≤95% display, min floor 408×320 [results 280 + preview 128], fallback 960×520 when display lookup fails); `split(modal_w: f32, override_results_w: Option<f32>) -> Split { results_w, preview_w }` (default 70/30, clamps results ≥280, preview ≥128); `clamp_drag(...)`, `reset_split(...)`; `scroll_center_row(match_row, visible_rows) -> usize`; `gutter_width(max_line_no, char_w) -> f32`.
- `AppTheme` additions: `active_line_bg: u32`, `git_created/git_modified/git_deleted/git_conflict/git_renamed/git_untracked/git_ignored: u32`; `bg` sourced from `elevated_surface_background` (fallback `background`); `picker_pane_width` becomes `Option<f32>`.
- Git color fallback hexes = today's exact values (no behavior change on themes lacking keys): created `0x32D583`, modified `0xF5A524`, deleted `0xF97066`, renamed `0x8E8E93`, untracked `0xA48EFF` (kept distinct — NOT remapped to created), ignored `0x6C6C70`; theme keys tried first (`version_control.added`/`created`, `version_control.modified`/`modified`, etc.; untracked tries `version_control.untracked` then falls back to its hex).
- Status-string mapping stays exhaustive as today (`picker.rs:330-337`): `staged_new`/`staged_modified` → created, `staged_deleted`/`deleted` → deleted, `modified` → modified, `renamed` → renamed, `untracked` → untracked, `ignored` → ignored, `clean`/`None` → no bar.
- Preview gutter numbers are computed in the render closure as `preview_start_line + i` — `FffPicker.preview_start_line` already exists (field :146, set :1054); no new field on `HighlightedLine`. The 500-line window is centered on the first match instead of anchored 8 lines above.
- `visible_rows` for scroll centering derives from layout math (modal height − 36px search row − 28px status bar − borders, ÷ line height), not live rendered bounds — `load_preview` runs in an async spawn with no window geometry access.
- Processing flow unchanged: search → results → selection change → debounced (120ms) `load_preview` → highlight window → scroll.

## What Goes Where
- **Implementation Steps** (`[ ]` checkboxes): tasks achievable within this codebase - code changes, tests, documentation updates
- **Post-Completion** (no checkboxes): items requiring external action - manual testing, changes in consuming projects, deployment configs, third-party verifications

## Implementation Steps

### Task 1: Verify gpui 0.2.2 API surface for divider drag

**Files:**
- Modify: `docs/plans/2026-07-21-zed-telescope-ui.md` (record findings)

- [x] inspect the vendored/downloaded `gpui 0.2.2` source (`cargo doc` or registry checkout) for: mouse down/move/up handlers on `Div`, drag tracking, `cursor_style`/col-resize cursor, double-click detection
- [x] confirm `uniform_list` scroll APIs used by the redesign (`scroll_to_item`, scroll handle offsets) exist unchanged
- [x] record findings and the chosen drag implementation pattern as a ➕ note under this task
- [x] if drag APIs are missing, note the keyboard-resize fallback decision here and adjust Task 10 accordingly — **not needed: drag APIs are fully present**, Task 10 stands as written
- [x] no code changes in this task — no new tests required (spike)

➕ **Findings (gpui 0.2.2 source: `~/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/gpui-0.2.2/`):**

*Mouse handlers on `Div` (all exist; imperative `Interactivity` impls in `src/elements/div.rs`, fluent trait wrappers same file):*
- `InteractiveElement` (works on plain `div()`): `on_mouse_down(button, fn)` :696, `on_mouse_up` :751, `on_mouse_up_out` :790, `on_mouse_move` :803, `on_drag_move::<T>` :818, `on_scroll_wheel`, `capture_any_mouse_down/up`, `on_mouse_down_out`.
- `StatefulInteractiveElement` (requires `.id(...)`, trait at div.rs:1041): `on_click` :1117, `on_drag` :1132, `on_hover` :1151. Already the established pattern in this codebase — `src/picker.rs:1694` rows do `div().id(("row", i)).hover(...).cursor_pointer().on_click(...)`; `src/text_field.rs:624-627` uses `on_mouse_down`/`on_mouse_up`/`on_mouse_up_out`/`on_mouse_move`.

*Drag tracking (built-in drag system — chosen mechanism):*
- `on_drag(value: T, constructor) -> Entity<W: Render>` (div.rs:499 imperative, :1132 fluent): arms a drag; gpui starts it after the mouse moves > `DRAG_THRESHOLD` = 2px (div.rs:47, :2159) and stores it in `cx.active_drag`. Constructor builds the floating drag-preview view — return `cx.new(|_| gpui::EmptyView)` (`src/view.rs:367`, renders `Empty`) for an invisible preview.
- `on_drag_move::<T>(fn(&DragMoveEvent<T>, ...))` (div.rs:282): fires during **capture phase for every mouse move window-wide** (no hitbox check — doc: "called for all move events, inside or outside of this element … useful for implementing draggable UIs that don't conform to a drag and drop style interaction, like resizing"). `DragMoveEvent<T>` (div.rs:61): `pub event: MouseMoveEvent`, `pub bounds: Bounds<Pixels>` (the listener element's own bounds), `.drag(cx) -> &T` payload accessor.
- Drag end: no callback — `window.rs:3716-3724` clears `cx.active_drag` on any `MouseUpEvent` and refreshes. Update split state live on every `on_drag_move`; nothing to commit on release.
- Click suppression: once a drag starts, `pending_mouse_down` is taken (div.rs:2164-2175), so `on_click` never fires for that gesture — double-click reset and drag coexist safely on the same element.

*Cursor styles:*
- Styled methods are macro-generated (`gpui_macros::cursor_style_methods!()`, `src/styled.rs:31`; bodies in `gpui-macros-0.2.2/src/styles.rs:159-330`): `cursor_col_resize()` → `CursorStyle::ResizeColumn` (platform.rs:1466), `cursor_ew_resize()` → `CursorStyle::ResizeLeftRight` (platform.rs:1442). Both exist; use `cursor_col_resize()` on the hit strip.
- During the drag, the cursor style is taken from the dragged element's own `mouse_cursor` style (`drag_cursor_style`, div.rs:2082) — so `.cursor_col_resize()` on the strip also keeps col-resize while dragging. 

*Double-click:*
- `MouseDownEvent.click_count: usize` (interactive.rs:104) and `MouseUpEvent.click_count` (:131); `ClickEvent::click_count()` (interactive.rs:256) returns `up.click_count`. Reset on `event.click_count() == 2` inside `on_click`.

*Chosen Task 10 drag pattern:*
1. 6px hit strip: `div().id("divider").cursor_col_resize().on_drag(DividerDrag, |_, _, _, cx| cx.new(|_| EmptyView)).on_click(cx.listener(… if event.click_count() == 2 { reset split }))` where `DividerDrag` is a zero-size marker struct.
2. On the body row (results+divider+preview container): `.on_drag_move::<DividerDrag>(cx.listener(|this, e, _, cx| { let results_w = e.event.position.x - e.bounds.origin.x; this.split = layout::clamp_drag(results_w, e.bounds.size.width); cx.notify(); }))` — `e.bounds` is the body row's own bounds, so the math needs no stored drag-start state.

*Gotchas:*
- Plain `on_mouse_move` on a `Div` only fires while that element's hitbox is hovered (bubble phase, `hitbox.is_hovered`, div.rs:263-276) — NOT window-wide; that's exactly why `on_drag_move` is the right tool (capture phase, no hitbox check).
- `on_drag`/`on_click` require a stateful element (`.id(...)`) — compile error otherwise.
- gpui paints the active drag's view following the cursor every frame (window.rs:2048-2080); `EmptyView` keeps it invisible.

*`uniform_list` scroll APIs (all confirmed unchanged, `src/elements/uniform_list.rs`):*
- `uniform_list(id, item_count, f)` :22, `UniformListScrollHandle` :80, `ScrollStrategy::{Top, Center, Bottom}` :84, `scroll_to_item(ix, strategy)` :146 (non-strict: no-op if already visible). Existing call sites compile-verified by today's build (picker.rs:916, 1059, 1221, 1364).
- ➕ Also available: `scroll_to_item_strict(ix, strategy)` :159 (always positions, even if visible) and `*_with_offset` variants :178/:197. **Note for Task 9:** `scroll_to_item_strict(match_row, ScrollStrategy::Center)` centers without needing a `visible_rows` estimate — prefer it over the manual `scroll_center_row` + `Top` combination where the handle is available; keep `layout::scroll_center_row` only for the highlight-window centering math.

### Task 2: Add layout math module

**Files:**
- Create: `src/layout.rs`
- Modify: `src/main.rs` (register `mod layout`)

- [x] implement `modal_size` (60%×60% of display, ≤95% clamp, min floor 408×320 for tiny-but-valid displays, px overrides win, 960×520 fallback when display size unavailable)
- [x] implement `split`, `clamp_drag`, `reset_split` (70/30 default, results ≥280px, preview ≥128px)
- [x] implement `scroll_center_row` (`match_row − (visible_rows − 1)/2`, clamped ≥0) and `gutter_width` (digit count of max visible line number × char width + 8px gap)
- [x] write tests for `modal_size` (default, override, max clamp, min floor on a display smaller than the minimum footprint, fallback cases)
- [x] write tests for `split`/`clamp_drag`/`reset_split` (default split, min clamps both sides, reset)
- [x] write tests for `scroll_center_row` (centered, near top clamp) and `gutter_width` (1-5 digit line numbers)
- [x] run `cargo test` - must pass before task 3

### Task 3: Make sizing config optional and viewport-relative

**Files:**
- Modify: `src/config.rs`
- Modify: `src/main.rs`
- Modify: `src/theme.rs`

- [x] change `window_width`/`window_height`/`picker_pane_width` to `Option<f32>` in `AppConfig` (remove baked-in defaults)
- [x] update the config sanitizer (`config.rs:179-198`) for the `Option` shape (`is_finite`/`<= 0.0` checks apply only when `Some`; invalid values become `None`)
- [x] update `open_window` (`src/main.rs:464`) to compute bounds via `layout::modal_size` from the active display, honoring overrides
- [x] change `AppTheme.picker_pane_width` to `Option<f32>` and reconcile the full plumbing: `Palette.picker_pane_width` (`theme.rs:55`), the `palette()` copy (:715), `apply_palette`, and the `is_finite` resolve in `sync_from_config` (`theme.rs:821`)
- [x] write tests for config parsing with absent and present sizing keys (absent → `None`, present → override value, invalid → `None`)
- [x] update any existing tests broken by the `Option` change
- [x] run `cargo test` - must pass before task 4

➕ **Task 3 notes:**
- Sanitizer extracted into a testable `sanitize_dimension(Option<f32>, key) -> Option<f32>` helper in `config.rs` (invalid `Some` → warn + `None`).
- Interim `picker.rs` resolution until Tasks 5/10: `theme.picker_pane_width.unwrap_or(config::DEFAULT_PICKER_PANE_WIDTH)` (430.0) — keeps current visuals; `layout::split` takes over in the layout flip.
- `DEFAULT_CONFIG` template no longer writes `window_width`/`window_height`/`picker_pane_width`, so fresh installs get viewport-relative sizing; existing configs with px values behave exactly as before.
- `DEFAULT_WINDOW_WIDTH`/`DEFAULT_WINDOW_HEIGHT` consts removed (fallback 960×520 lives in `layout.rs`); `DEFAULT_PICKER_PANE_WIDTH` kept only for the interim picker fallback.
- `layout.rs` file-level `#![allow(dead_code)]` replaced with per-item allows on `split`/`clamp_drag`/`reset_split` (Tasks 5/10) and `scroll_center_row`/`gutter_width` (Tasks 8/9); `modal_size` is now live via `open_window`.

### Task 4: Extend theme tokens (elevated bg, active line, git colors)

**Files:**
- Modify: `src/theme.rs`
- Modify: `src/config.rs`

- [x] remap `AppTheme.bg` to `elevated_surface_background` with fallback to `background` in `palette_from_style` (`theme.rs:1493`)
- [x] add `active_line_bg` token (`editor.active_line.background`; fallback: selection color at subtle alpha)
- [x] add git tokens (`git_created`, `git_modified`, `git_deleted`, `git_conflict`, `git_renamed`, `git_untracked`, `git_ignored`) parsed from the theme's `version_control.*`/status keys with the current hardcoded hexes as fallback (untracked keeps its distinct purple `0xA48EFF` fallback — see Technical Details)
- [x] plumb all new tokens through `Palette`, `apply_palette`, Zed `theme_overrides` deep-merge, and `[theme]` config overrides (`ThemeConfig` in `config.rs`)
- [x] write tests: token mapping from a theme JSON that has the keys, fallback when keys are missing, `[theme]` override precedence
- [x] run `cargo test` - must pass before task 5

➕ **Task 4 implementation notes:**
- Key lookup order per token: `bg` = `elevated_surface.background` → `background` → `surface.background` → `editor.background` → default; git tokens = `version_control.added|modified|deleted|conflict|renamed|untracked|ignored` → flat status key (`created`/`modified`/`deleted`/`conflict`/`renamed`/`ignored`) → legacy hex. `git_untracked` deliberately tries ONLY `version_control.untracked` (no flat key — Zed remaps untracked to created, which would lose the distinct purple `0xA48EFF`).
- **Conflict fallback chosen: deleted's red `0xF97066`** (`DEFAULT_GIT_CONFLICT = DEFAULT_GIT_DELETED`) — no new arbitrary hex; real themes carry a `conflict` key so the fallback rarely shows.
- `active_line_bg` fallback: colors render opaquely via `rgb(...)` (`parse_color_rgb` strips alpha), so the "selection at subtle alpha" fallback is pre-blended: `blend_over(selected_row, preview_bg, 0.35)` (new `blend_over` helper in theme.rs).
- Also plumbed through `theme::palette()` (AppTheme → Palette back-conversion at theme.rs:752), which the checklist didn't list — required to compile; it also makes the new AppTheme fields "read", so no `#[allow(dead_code)]` was needed.
- Existing One Dark tests that overrode `"background"` and asserted `theme.bg` were updated to override `"elevated_surface.background"` (One Dark has that key, so `bg` now sources from it): `override_merges_color_and_syntax_onto_base`, `fff_config_color_wins_over_zed_override`, `mixed_case_override_key_applies_via_normalized_overrides`, `dynamic_selection_applies_override_for_resolved_variant_only`.

### Task 5: Flip layout to Zed orientation

**Files:**
- Modify: `src/picker.rs`

- [x] move the search row to the top: 36px height, 10px horizontal padding, goose glyph + TextField, 1px `border_variant` divider below (replaces the 46px bottom bar)
- [x] restructure the body row: results left (`flex_1`) · 1px divider · preview right at `layout::split` width; status bar stays at the bottom (28px, 1px top border, new chrome colors)
- [x] remove the `total - 1 - visual_i` inversion so results render top-down with rank 0 at the top; selection starts at the top and arrow keys move naturally
- [x] fix scroll-to-selection anchoring for the new direction (selection follows into view at top/bottom)
- [x] if a pure index/ordering helper remains after removing the inversion (e.g. `visual_index` :1240 doesn't collapse to identity), write tests for it; otherwise note "no pure helper — covered by build + manual verification"
- [x] run `cargo test` and `cargo build` - must pass before task 6

➕ **Task 5 implementation notes:**
- `visual_index` collapsed to identity and was deleted — no pure helper remains; ordering is covered by build + manual verification (`layout::split` itself already has Task 2 tests).
- Modal width comes from `window.viewport_size().width` inside `render` (the window IS the modal); preview pane is fixed at `layout::split(modal_w, theme.picker_pane_width).preview_w` + `flex_shrink_0`, results pane is `flex_1` with `min_w(0)`. Path-shortening `avail_px` now uses `results_pane_width = modal_w − preview_w − 1` (the 1px divider).
- Scroll anchoring for the top-down list: new-results apply → `scroll_to_item(selected, Top)`; down arrow (`on_select_next`, now `selected += 1`) → non-strict `scroll_to_item(selected, Bottom)`; up arrow (`on_select_prev`, now `selected -= 1`) → non-strict `scroll_to_item(selected, Top)`; row click keeps non-strict `Center` (no-op for a visible row). Non-strict = no scroll when already visible, so the selection "follows into view" at the edge being moved toward.
- The search bar moved out of the results pane, so the pane's inner wrapper div was collapsed; the results/empty/indexing `.when` branches now sit directly on the pane div.
- `config::DEFAULT_PICKER_PANE_WIDTH` (interim Task 3 fallback) removed — `layout::split` now owns the default; `split`'s scoped `#[allow(dead_code)]` removed in `layout.rs`.

### Task 6: Restyle result rows and git edge bar

**Files:**
- Modify: `src/picker.rs`

- [x] file rows: filename at default text color with fuzzy-matched chars tinted `text_accent` (via existing `render_highlighted` ranges), directory muted; selection stays flat `selected_row`, hover `hover_row`
- [x] grep rows: currently `render_highlighted` (:200-259) colors matched chars via text color only — additionally give the matched substring a flat search-match background + bold (the rounded pill is a preview-pane concern, handled in Task 8)
- [x] replace `git_status_bar_color()` hardcoded hexes with the new `AppTheme` git tokens; keep the status-string mapping exhaustive per Technical Details (staged_new/staged_modified → created, staged_deleted/deleted → deleted, untracked → untracked [distinct], renamed → renamed, ignored → ignored)
- [x] grow the git bar from 3px×18px to full-row-height 3px, flush left, on both file and grep rows; clean/no-status renders nothing
- [x] write tests for the status-string → theme-token mapping (all observed status strings + `None`/`clean`)
- [x] run `cargo test` - must pass before task 7

➕ **Task 6 implementation notes:**
- `git_status_bar_color(status: Option<&str>, theme: &AppTheme) -> Option<u32>` now resolves the new `AppTheme` git tokens. Mapping: `modified` → `git_modified`, `staged_new`/`staged_modified` → `git_created`, `staged_deleted`/`deleted` → `git_deleted`, `renamed` → `git_renamed`, `untracked` → `git_untracked` (distinct), `ignored` → `git_ignored`, `clean`/`None` → no bar, unknown strings → `git_ignored` (preserves the old catch-all gray behavior).
- **Conflict:** fff-search 0.10.1's `format_git_status_opt` never emits `"conflict"` (a `Status::CONFLICTED` file falls through to `None`), so `Some("conflict")` → `git_conflict` is a defensive mapping only.
- Git bar: `h(18px)` → `h_full()` (full 28px row height), 3px wide, first child of the row (flush left — row has no left padding). **No-bar alignment choice:** rows without status keep the transparent 3px strip (existing pattern) so text alignment stays identical across rows.
- Grep match emphasis: `render_highlighted` gained a `bg_emphasis: bool` — matched ranges get `match_highlight_bg` flat background + `FontWeight::BOLD` on top of the `match_highlight` tint. Grep rows pass `true`, file rows `false`.
- File rows unchanged (already correct): filename base `text_primary` (explicit in ranged spans, inherited from the root container otherwise), fuzzy chars `match_highlight`, directory kept at its current muted `text_secondary`, selection `selected_row` / hover `hover_row` flat fills verified untouched.
- Tests live in a new `picker::tests` module. Gotcha: `use super::*` there would pull in gpui's `test` attribute macro (picker.rs glob-imports `gpui::*`) and shadow the built-in `#[test]` (manifested as a "recursion limit reached" expansion error) — the module uses targeted imports instead.

### Task 7: Restyle search input to plain Zed-style text

**Files:**
- Modify: `src/text_field.rs`
- Modify: `src/picker.rs`

- [x] remove the boxed/rounded input rendering (`text_field.rs:636`) — plain text on the modal background, cursor/selection painting kept (also dropped the field's own `h(36)`/`px(10)` — the picker search row already provides 36px height + 10px padding, so text no longer double-pads and centers via the row; no focus-border code existed to remove — cursor paint was already focus-gated)
- [x] align input text styling (font, placeholder color) with the new chrome tokens (placeholder stays muted `text_dim`, text `text_primary`, buffer font size/family inherited from the search row — already aligned; `input_bg`/`input_text` tokens have no render consumers but are left in the Palette/config plumbing since removal would ripple through config overrides)
- [x] pure render change — no pure helper — covered by build + manual verification
- [x] run `cargo test` and verify typing/IME still works via `cargo build` + quick manual launch (85 tests pass, build clean; app launched and rendered without errors — interactive typing/IME not exercisable by the agent, but input-handling code paths are untouched)
- [x] run `cargo test` - must pass before task 8 (85 passed)

### Task 8: Preview gutter and match highlighting

**Files:**
- Modify: `src/preview.rs`
- Modify: `src/picker.rs`

- [x] remove the 28px preview path header (`picker.rs:1912`)
- [x] render the gutter in the preview `uniform_list` closure: right-aligned, `layout::gutter_width`-sized, muted at ~50% opacity, 8px gap before code, always on — line number for row `i` = `preview_start_line + i` (existing field :146, set :1054; no new field on `HighlightedLine`)
- [x] matched lines: full-width `active_line_bg` tint plus flat bold search-match background on the matched substring (remove the pill treatment here too)
- [x] write tests for the windowing change (start line returned, line numbers correct at file start/middle/end)
- [x] write tests for `overlay_match_ranges` still splicing correctly with the new line struct
- [x] run `cargo test` - must pass before task 9 (96 passed)

➕ **Task 8 implementation notes:**
- `preview_start_line` verified 1-based (init 1, reset 1, set from `highlight_file_window`'s 1-based start), so `preview_start_line + i` directly renders editor-style line numbers.
- Matched-line flagging: no struct change — `HighlightedLine` is untouched. Added pure `preview::line_has_match(&HighlightedLine)` deriving it from `span.bg.is_some()` (only `overlay_match_ranges` ever sets `bg`); the render closure calls it per row.
- Bold emphasis lives in `overlay_match_ranges` (match chunks now emit `bold: true`), so the render closure's existing `.when(span.bold, ...)` handles it; span bg render changed from `rgba(..|0x77)+px(3)+rounded(4)` pill to flat `rgb(match_highlight_bg)`.
- Gutter sizing: `layout::gutter_width(preview_start_line + preview_lines.len(), buffer_font_size * 0.6)` computed once per render outside the list closure (per the plan formula; over-provisions by one line number at digit rollovers — never clips). Number right-aligned via `flex().justify_end()` with `pr(GUTTER_GAP)` (gap is inside the width per the `gutter_width` contract), `text_dim` at 50% alpha.
- Header removal also deleted the now-dead `FffPicker.base_path` field (the header path strip was its only reader; scan/open paths use locals or `FilePicker::base_path()`).
- `highlight_file_window` windowing verified unchanged (8-above anchor stays for Task 9); tests exercise it end-to-end via temp files on the plain-text path.

### Task 9: Match-centered preview scrolling

**Files:**
- Modify: `src/picker.rs`
- Modify: `src/preview.rs`

- [x] center the 500-line highlight window on the first match (replace the fixed 8-lines-above anchor); remove or repurpose the now-obsolete `preview::MATCH_CONTEXT_BEFORE` const (`preview.rs:455`, `picker.rs:1060`) — removed
- [x] compute `visible_rows` via layout math (see Technical Details — not live rendered bounds) and scroll via `layout::scroll_center_row` in `load_preview` (`picker.rs:1058`) — superseded per Task 1 findings: `scroll_to_item_strict(row, ScrollStrategy::Center)` needs no `visible_rows` estimate (see notes)
- [x] re-center the match after divider drags change the preview width/height — `FffPicker::recenter_preview()` added; Task 10 must call it on drag
- [x] write tests for the window-centering math (match near file start, middle, end; file shorter than window)
- [x] run `cargo test` - must pass before task 10 (102 passed)

➕ **Task 9 implementation notes:**
- **In-pane scroll = strict Center, not manual math.** Per the Task 1 findings note, `load_preview`'s continuation now calls `preview_scroll.scroll_to_item_strict(match_row, ScrollStrategy::Center)` (wrapped in `FffPicker::recenter_preview()`), replacing the old `scroll_to_item(match_row − 8, Top)`. The deferred scroll resolves against the pane's real bounds at next layout, so the checklist's `visible_rows` layout-math estimate (Technical Details) became unnecessary — no estimate drift after resizes.
- **Window centering** lives in new pure `preview::window_start_line(center_line: Option<usize>, total_lines) -> usize` (1-based): window start = `layout::scroll_center_row(center_line − 1, MAX_PREVIEW_LINES) + 1`, clamped so the window never runs past the last line; no match or a file shorter than 500 lines → line 1. A deep match gets ~250 lines of context each side (249 above, 250 below).
- `layout::scroll_center_row` is therefore consumed by the window-centering math (its `#[allow(dead_code)]` was removed); the in-pane centering no longer needs it.
- `preview::MATCH_CONTEXT_BEFORE` removed; the Task 8 windowing tests that asserted the 8-above anchor were rewritten against the centered-window contract (early/middle/end matches now use a 1200-line file so the clamps actually engage) plus direct `window_start_line` unit tests.
- ➕ **Note for Task 10:** call `self.recenter_preview()` from the divider drag-move handler (and double-click reset) so the match stays vertically centered after the preview pane resizes.

### Task 10: Drag-resizable divider

**Files:**
- Modify: `src/picker.rs`

- [x] add a session-only split field on `FffPicker`, seeded from `picker_pane_width` override or `layout::split` default
- [x] render a 6px invisible hit strip over the 1px divider with col-resize cursor (per Task 1 findings)
- [x] implement drag: update the split through `layout::clamp_drag` (results ≥280px, preview ≥128px) and re-render live
- [x] double-click on the divider resets to the 70/30 default via `layout::reset_split`
- ➕ [x] call `FffPicker::recenter_preview()` after drag-move updates and double-click reset so the match stays centered when the preview pane resizes (Task 9 note)
- [x] write tests for any new pure drag-state helpers (delta application + clamping, reset) — `clamp_drag`/`reset_split` themselves are already covered in Task 2, so this is re-verification plus any glue introduced here
- [x] run `cargo test` - must pass before task 11 (107 passed)

➕ **Task 10 implementation notes:**
- Session state: `FffPicker.session_results_width: Option<f32>` (results-pane px), init `None`. Resolution lives in new pure `picker::effective_split(modal_w, session, config) -> layout::Split` = `layout::split(modal_w, session.or(config))` — precedence session drag → `theme.picker_pane_width` config → 70/30 default, all clamped by `split`. Render's split call site now goes through it.
- Hit strip geometry: the divider stays the 1px `border`-colored flex child, made `relative()`; the hit strip is an absolutely-positioned child (`id("divider-hit")`, `left(-2.5px)`, `w(6px)`, `h_full()`) centered over the line — ~2.5px overhang into each pane, visible line stays 1px.
- Drag wiring per Task 1 findings: strip has `.cursor_col_resize().on_drag(DividerDrag, |…| cx.new(|_| EmptyView)).on_click(double-click reset)`; the body row has `.on_drag_move::<DividerDrag>` computing `results_w = e.event.position.x − e.bounds.origin.x`, then `session_results_width = clamp_drag(results_w, e.bounds.size.width).results_w`, `recenter_preview()`, `cx.notify()` per move. No drag-end hook needed (state updates live; gpui clears the drag on mouse-up).
- Double-click reset stores `reset_split(modal_w).results_w` as the session value (a config `picker_pane_width` would win over a bare `None`, so reset must pin the default explicitly).
- Borrow gotcha: the results-list `.when` closure `move`s `cx`, so the divider's `on_click` listener is prebuilt into a local (`divider_double_click`) before the element chain; the `on_drag_move` listener sits earlier in the chain and needed no hoisting.
- Scoped `#[allow(dead_code)]`s on `layout::clamp_drag`/`reset_split` removed (now consumed).
- Tests: 5 new in `picker::tests` — precedence (session>config, config>default, default), session clamping to both pane minimums through the glue, and reset-value-over-config (double-click lands on 70/30 even with a config override present).

### Task 11: Verify acceptance criteria
- [x] verify all requirements from Overview are implemented (layout flip, viewport sizing + px overrides, gutter, centered scroll, divider drag, git edge bars, theme-derived colors, header removed, status bar + goose kept)
- [x] verify edge cases: tiny display (clamps), missing theme keys (fallbacks), empty results, non-git directories (no bars), grep multi-line matches
- [x] run full test suite: `cargo test`
- [x] run `cargo build --release` cleanly (no new warnings)
- [x] verify existing config.toml behavior is unchanged (px values honored)

➕ **Task 11 verification evidence (all requirements met, no fixes needed):**
- *Overview requirements:* layout flip — 36px search row on top with 10px padding, goose glyph, TextField, 1px `border` divider below (`picker.rs:1587-1613`); results top-down rank-0-first, no index inversion (`picker.rs:1738-1743`); preview right at `effective_split` width with `flex_shrink_0` (`picker.rs:1457-1462`, `:2003-2007`); 28px status bar with 1px top border kept at the bottom (`picker.rs:2086-2116`). Viewport sizing — `open_window` computes bounds via `layout::modal_size` from the active display, px config values act as overrides (`main.rs:481-503`, `layout.rs:33-55`). Gutter — 1-based `preview_start_line + i` numbers, right-aligned in `layout::gutter_width` width, `text_dim` at 50% alpha (`picker.rs:1474-1481`, `:2049-2058`). Centered scroll — `recenter_preview()` uses `scroll_to_item_strict(row, ScrollStrategy::Center)` (`picker.rs:1106-1109`) plus window centering via `preview::window_start_line` (`preview.rs:455-464`). Divider drag — 6px hit strip at `left(-2.5)` with `cursor_col_resize`, `on_drag(DividerDrag, EmptyView)`, drag-move through `layout::clamp_drag` + `recenter_preview`, double-click reset via `layout::reset_split` (`picker.rs:1496-1504`, `:1627-1636`, `:1978-2001`). Git edge bars — full-row-height 3px flush-left, transparent strip when no status (`picker.rs:1812-1825`); colors from `AppTheme` git tokens (`picker.rs:359-372`). Theme-derived colors — `bg` from `elevated_surface.background` with fallback chain, `active_line_bg` from `editor.active_line.background` with pre-blended fallback, git tokens from `version_control.*` → flat keys → legacy hexes (`theme.rs:1576-1660`). Header removed — preview pane renders placeholder/uniform_list directly, no path header child (`picker.rs:2003-2083`). TextField box chrome gone — no `rounded`/`border`/`bg` in `text_field.rs`.
- *Edge cases:* tiny display — `modal_size_floors_on_display_smaller_than_minimum_footprint`, `modal_size_floors_tiny_overrides`, `split_at_minimum_modal_width_fits_both_minimums` (layout.rs tests). Missing theme keys — `palette_new_tokens_fall_back_when_keys_missing`, `palette_git_tokens_fall_back_to_flat_status_keys`, `palette_maps_new_tokens_from_theme_json` (theme.rs tests). Empty results — `scan_done && results.is_empty()` branch renders "No files matched" / grep tips (`picker.rs:1664-1722`). Non-git — `format_git_status_opt` yields `None` → `git_status_bar_color` returns `None` (test `picker.rs:2220-2221`) → transparent 3px strip. Grep multi-line matches (README:105) — each per-line `GrepMatchLine` gets `overlay_match_ranges` in `load_preview` (`picker.rs:1063-1073`); ranges are clamped to span boundaries (`preview.rs:379`) and char boundaries (`preview.rs:437-449`) so cross-line ranges are safe with the flat match styling; tests `overlay_splices_range_across_span_boundaries`, `overlay_handles_multiple_ranges_on_one_line`.
- *`cargo test`:* 107 passed, 0 failed, 0 ignored (single test binary `fff_gpui`).
- *`cargo build --release`:* finished clean in 3m 48s; forced recompile of the crate (`touch src/main.rs`) produced 0 warnings.
- *Smoke test:* `timeout 10 ./target/release/fff-gpui --open .` — no panic, no error output.
- *Config backward-compat:* `window_width=960.0`/`window_height=520.0`/`picker_pane_width=430.0` parse to `Some` (`config.rs` test `sizing_keys_present_parse_as_overrides`), `modal_size` returns exactly 960×520 on any display where the override fits under the 95% clamp (`modal_size_px_overrides_win_over_percentage`), `sync_from_config` carries the pane width to `AppTheme` (`theme.rs:888-890`, tests `apply_palette_preserves_config_owned_picker_pane_width`, `picker_pane_width_defaults_to_none_across_the_pipeline`), and `effective_split(960, None, Some(430))` yields a 430px results pane (`effective_split_config_wins_over_default_without_session` + layout `split` tests). Absent keys → `None` end-to-end → 60%×60% viewport + 70/30 split (`sizing_keys_absent_parse_as_none`, `modal_size_defaults_to_sixty_percent_of_display`, `effective_split_defaults_to_seventy_thirty`). Minor note: no single end-to-end test parses a full config.toml and asserts the final split, but every link in the chain is unit-tested individually — not worth new plumbing.

### Task 12: [Final] Update documentation
- [x] update README.md Configuration section: percentage defaults, px values as overrides, divider drag/reset behavior (sizing keys commented out as optional in the example config; new prose paragraph documents 60%×60% default with 408×320 min / 95% max clamps, 70/30 split, px overrides, divider drag with 280px/128px pane minimums, double-click reset, session-only drag state; `[theme]` prose only references the general override mechanism — no per-key list existed, none needed)
- [x] update CLAUDE.md if new patterns discovered — skipped: no CLAUDE.md exists at the repo root (not created per task instructions)
- [x] move this plan to `docs/plans/completed/` (create dir if needed)
- [x] move the linked design file too, if any. Extract the value with: `grep -E '^\*\*Design:\*\*' <plan-file> | sed 's/^\*\*Design:\*\* *//'`. If the extracted value is empty, the literal placeholder text, or `none`, skip. Otherwise `test -f <design-path>` and `mv <design-path> docs/plans/completed/` — if the file is missing, print a warning and continue.

## Post-Completion
*Items requiring manual intervention or external systems - no checkboxes, informational only*

**Manual verification:**
- visual pass via `fff-gpui --open .` across One Dark / Ayu / Gruvbox in light + dark modes
- verify git edge bars in a repo containing modified / untracked / staged / deleted / ignored files
- verify divider drag feel, double-click reset, and that the split resets on daemon restart (session-only)
- verify global-keybind launch sizing on multiple displays (different resolutions)
- verify Zed task integration still opens files / jumps to grep lines

**External system updates:**
- none (self-contained app; Homebrew formula unaffected)
