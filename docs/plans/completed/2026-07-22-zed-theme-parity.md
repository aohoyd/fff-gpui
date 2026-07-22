# Zed Theme Parity

> **For Claude:** use `/planning:execute` to implement this plan task-by-task with fresh subagents.

**Goal:** Apply theme colors to the picker exactly as Zed's telescope picker does — RGBA palette honoring translucent theme colors (fixing the invisible active-line and illegible match-highlight bugs), a `border_variant`-based chrome remap, transparent search row/status bar over one surface, and Zed-correct match/fuzzy text styling.

**Architecture:** The `Palette`/`AppTheme` color storage switches from `0xRRGGBB` to `0xRRGGBBAA` with live `rgba()` blending at paint time (no pre-blending). New tokens (`border_variant`, `text_accent`, `editor_line_number`, `editor_active_line_number`, `editor_gutter_bg`) join the sync pipeline; `status_bar_bg`/`input_bg`/`cursor_selection` are removed. Surface painting in `picker.rs` is remapped to Zed's exact token-per-surface assignment; match substrings keep their text color (bold + translucent bg only).

**Tech Stack:** Rust, gpui (zed git dep, pinned), existing theme sync pipeline in `src/theme.rs`.

**Design:** docs/plans/2026-07-22-zed-theme-parity-design.md

## Overview

- Fixes two sync bugs caused by `parse_color_rgb` truncating 8-hex colors: One Dark's `editor.active_line.background` (`#2f343ebf`) collapses to exactly `bg` (invisible preview active line), and `search.match_background` (`#74ade866`) resolves both match text color and match background to the same opaque blue (illegible matches).
- Aligns every picker surface with Zed: all chrome (input underline, results/preview divider, separators, status-bar top border) uses a new subtle `border_variant` token; search row and status bar are transparent over the single `bg` surface; the preview keeps its distinct `editor.background`; rows use `ghost_element.*` tokens.
- Match substrings render Zed-exact: original syntax/normal color + bold + translucent `match_highlight_bg`. Files-mode fuzzy tint switches to `text.accent`.
- Window stays edge-to-edge, opaque, square — NO rounded card, no window transparency, no shadow (user decision).
- BREAKING (user-approved): `[theme]` config keys `status_bar_bg` and `input_bg` are removed.

## Context (from discovery)

- **The working tree has uncommitted changes** from two completed plans (telescope restyle + grouped results). Build on the tree as-is; NEVER `git checkout`/revert files (see CLAUDE.md — this destroyed work once). Baseline: 234 tests passing, warning-free build, fmt clean.
- `src/theme.rs` (~1900 lines): `Palette` + `AppTheme` structs, hardcoded dark defaults (`DEFAULT_*` consts ~lines 18-46), sync pipeline `sync_from_config` (~806-934) → `palette_from_style` (~1577-1661) → `apply_palette`; color parsing `parse_color_rgb`/`color_from_style` (~1805-1863) — **truncates 8-hex to 6, dropping alpha**; per-key `[theme]` overrides via `apply_color` (~892-922); `blend_over` helper used for the current pre-blended `active_line_bg` fallback (~149-153).
- `src/picker.rs`: root `.bg(rgb(theme.bg))` (~2610-2636); search row `border_b_1` + `border` (~2645); rows repaint `theme.bg` when unselected (header ~2168, grep match ~2292, file ~2406); separator + split divider use `border` (~2112-2120, ~2809-2837); preview `.bg(rgb(theme.preview_bg))` + gutter text `text_dim @ 50%` via manual rgba shift (~2549) + active-line `.bg(rgb(active_line_bg))` when `has_match` (~2877); status bar `.bg(rgb(theme.status_bar_bg))` + `border_t` `border` (~2942-2965); `match_row_spans` (~908-949) currently recolors matched text with `match_highlight` + opaque `match_highlight_bg`; `render_highlighted` Tint style colors fuzzy filename chars with `match_highlight`; `render_checkbox` (~478-504) checked fill = `match_highlight`.
- `src/text_field.rs`: cursor quad painted with `palette.match_highlight` (~537); selection `selected_row @ 0x44` (~553); `AppTheme.cursor`/`cursor_selection` are written but never read.
- `src/config.rs` / theme config: `[theme]` per-key color overrides; keys must be added/removed symmetrically across ThemeConfig, `apply_color` calls, and README.
- Zed reference values and exact per-surface tokens: see the design doc §"Zed reference" (verified against zed main; One Dark hexes included).
- gpui `rgba(u32)` expects `0xRRGGBBAA`; `rgb(u32)` expects `0xRRGGBB` — the refactor swaps constructors together with the value format.

## Development Approach

- **testing approach**: Regular (code first, then tests in the same task)
- complete each task fully before moving to the next
- make small, focused changes
- **CRITICAL: every task MUST include new/updated tests** for code changes in that task
  - tests are not optional - they are a required part of the checklist
  - write unit tests for new functions/methods
  - write unit tests for modified functions/methods
  - add new test cases for new code paths
  - update existing test cases if behavior changes
  - tests cover both success and error scenarios
- **CRITICAL: all tests must pass before starting next task** - no exceptions
- **CRITICAL: update this plan file when scope changes during implementation**
- run tests after each change
- maintain backward compatibility EXCEPT the two approved breaking config keys
- GPUI render closures are untestable — extract pure functions and test those (CLAUDE.md policy). Tasks that are almost purely render-closure work (Task 3 chrome remap, Task 5's gutter bg) may legitimately add no NEW tests — updating existing assertions satisfies the mandate there; Tasks 1, 2, and 5's number-color decision are the genuinely unit-testable surfaces
- **never revert or `git checkout` working-tree files** — the tree holds uncommitted prior work

## Testing Strategy

- **unit tests**: required for every task — see the testing mandate in Development Approach above (not restated here)
- **e2e tests**: none in this project (GPUI app). Manual verification items live in Post-Completion.
- Gates for every task: `cargo test` all green, `cargo build` warning-free, `cargo fmt --check` clean.

## Progress Tracking

- mark completed items with `[x]` immediately when done
- add newly discovered tasks with ➕ prefix
- document issues/blockers with ⚠️ prefix
- update plan if implementation deviates from original scope
- keep plan in sync with actual work done

## Solution Overview

- **Task order matters**: the RGBA storage refactor lands first, then token additions/removals in the sync pipeline, then the surface/chrome remap, then text-styling changes, then preview refinements. Note: Task 1 is NOT visually neutral — opaque tokens render identically, but every alpha-bearing theme color (active-line wash, match bg, translucent syntax colors) starts blending correctly the moment the parser preserves alpha, i.e. BOTH target bugs are fixed in Task 1; Tasks 2-5 are token/chrome/styling alignment on top.
- All alpha handling is live blending at paint time via `rgba()`; nothing is pre-blended at sync.
- Fallback design keeps every token usable with zero Zed sync: `border_variant`→`border`, `text_accent`→current accent blue, `editor_gutter_bg`→`preview_bg`, `editor_line_number`→`text_dim`-derived, `editor_active_line_number`→near-`text_primary`, `active_line_bg` fallback→`selected_row @ ~0x59`.

## Technical Details

- **Color format**: `u32` = `0xRRGGBBAA`. Parser `parse_color_rgba`: 8-hex keeps alpha; 6-hex appends `FF`; 3-hex shorthand `#rgb` expands to `0xRRGGBBFF` and 4-hex `#rgba` to `0xRRGGBBAA` (the current parser supports both — do NOT regress; the `parses_hex_colors` test asserts `#f0a`/`#1234`/`#f0ab`); invalid → `None` (caller falls back to default). Paint: `rgba(value)` — gpui's `rgba(u32)` reads `0xRRGGBBAA` (verified at the pinned rev, crates/gpui/src/color.rs:14-21).
- **Bit-shift alpha sites — exactly three, all break under RGBA** (`(0xRRGGBBFF << 8) | a` overflows): picker.rs:2281 (results-row gutter number, `text_dim @ 0x80` — referenced by no other task, fix here), picker.rs:2549 (preview gutter, same expression), text_field.rs:553 (selection fill, `selected_row @ 0x44`). Each becomes `(value & 0xFFFFFF00) | alpha`, keeping its current semantics.
- **Token table** (Zed key chains — design §2 is authoritative):
  - NEW `border_variant`: `border.variant` → fallback `border` value.
  - NEW `text_accent`: `text.accent` → fallback `0x4A9EFFFF`.
  - NEW `editor_line_number`: `editor.line_number` → fallback `text_dim @ 0x80`-style.
  - NEW `editor_active_line_number`: `editor.active_line_number` → fallback `text_primary`.
  - NEW `editor_gutter_bg`: `editor.gutter.background` → fallback `preview_bg` value.
  - REMAP `hover_row`: `ghost_element.hover` → `element.hover` → default.
  - REMAP `match_highlight`: `text.accent` → fallback `0x4A9EFFFF` (fuzzy tint + checkbox fill + cursor fallback).
  - REMAP `match_highlight_bg`: `search.match_background` → `search.active_match_background`, alpha preserved; fallback `0x2C4870FF`.
  - REMAP `active_line_bg`: `editor.active_line.background` (alpha preserved); fallback `selected_row` with alpha byte `0x59`.
  - KEEP `cursor`: `editor.cursor` → fallback `text_accent` value; NOW actually consumed by TextField.
  - REMOVE `status_bar_bg`, `input_bg`, `cursor_selection` — from Palette, AppTheme, sync, `[theme]` config, and all writes.
- **Surface map** (One Dark result — design §2 preview block is authoritative): root `bg`; search row transparent + `border_variant` underline; rows transparent (selected `selected_row`, hover `hover_row`); split divider + separators `border_variant`; preview `preview_bg` + gutter `editor_gutter_bg`; status bar transparent + `border_variant` top border.
- **Match styling**: `match_row_spans` emits spans that KEEP the incoming fg color (syntax or fallback text color), set `bold: true`, and set `bg = match_highlight_bg` (translucent). `render_highlighted` `MatchStyle::Tint` colors matched chars `text_accent` (renamed source; no bg).

## What Goes Where

- **Implementation Steps** (`[ ]` checkboxes): all code, tests, docs changes below.
- **Post-Completion** (no checkboxes): manual visual verification across themes.

## Implementation Steps

### Task 1: RGBA color storage and parser

**Files:**
- Modify: `src/theme.rs`
- Modify: `src/picker.rs`, `src/text_field.rs` (constructor swaps + bit-shift fixes; `src/preview.rs` and `src/main.rs` have NO `rgb`/`rgba` paint sites — preview.rs only emits `HighlightedSpan { color: u32, bg: Option<u32> }` data that picker.rs paints)

- [x] rename/replace `parse_color_rgb` with `parse_color_rgba` in `src/theme.rs`: 8-hex `#rrggbbaa` keeps authored alpha, 6-hex appends `FF`, 3-hex `#rgb` expands to `0xRRGGBBFF`, 4-hex `#rgba` expands to `0xRRGGBBAA` (current parser handles 3/4/6/8 — do not regress; `parses_hex_colors` asserts the shorthand forms), invalid input returns `None`; update `color_from_style` and the `[theme]` `apply_color` override path to use it
- [x] convert every `Palette`/`AppTheme` color field and ALL hardcoded defaults (`DEFAULT_*` consts, inline literals, git colors, syntax_default_color if stored as u32) to `0xRRGGBBAA` (append `FF`); make `active_line_bg`'s fallback a genuinely translucent `selected_row` value with alpha byte `0x59` (replacing the pre-blended `blend_over` result; remove `blend_over` AND its tests `blend_over_endpoints_and_midpoint` / rewrite `active_line_bg_falls_back_to_blended_selection` if now unused — the warning-free gate forces the cleanup)
- [x] replace **ALL** `rgb(...)` constructor calls with `rgba(...)` in `src/picker.rs` (~47 sites) and `src/text_field.rs` — INCLUDING local-variable ones (`rgb(span.color)` at ~2350/~2912, `rgb(bg)`/`rgb(bg_color)` at ~2355/~2926, `rgb(color)` ~363, `rgb(muted)` ~460, git-bar `rgb(color)` ~2190/2313/2428, `rgb(badge_color)` ~2478, `rgb(border)`, `rgb(active_line_bg)`), not just `rgb(theme.*)` — the shared parser makes EVERY palette and syntax u32 RGBA, so any remaining `rgb()` renders garbage (`0xGGBBAA`)
- [x] fix the three bit-shift alpha sites per Technical Details: picker.rs:2281 (results-row gutter number — orphaned by every other task, fix HERE), picker.rs:2549 (preview gutter), text_field.rs:553 (selection fill) → `(value & 0xFFFFFF00) | alpha` keeping current semantics
- [x] verify: opaque tokens render identically; translucent tokens now blend correctly — BOTH target bugs (invisible active line, illegible match bg) are fixed by this task. Expect broad mechanical test churn in theme.rs (nearly every u32 color assertion gains `..ff`; One Dark-derived assertions like `active_line_bg == 0x2f343e` become `0x2f343ebf`); update tests to the RGBA values rather than deleting them
- [x] write tests for `parse_color_rgba`: 3-hex, 4-hex, 6-hex, 8-hex alpha preservation, invalid input, empty string, `#`-prefix handling
- [x] write/adjust tests proving `active_line_bg` sync now preserves alpha (One Dark-style `#2f343ebf` stays translucent, no longer equal to `bg`)
- [x] run tests - must pass before next task

**Task 1 notes:**
- `blend_over` + `ACTIVE_LINE_BG_ALPHA` removed; `active_line_bg_falls_back_to_blended_selection` rewritten as `active_line_bg_falls_back_to_translucent_selection` (asserts `#404040` selection → `0x40404059` fallback). New sync test `active_line_bg_sync_preserves_authored_alpha` applies real One Dark and asserts `active_line_bg == 0x2f343ebf != bg (0x2f343eff)` and `match_highlight_bg == 0x74ade866`.
- `cursor_selection`'s default `0x0A84FF44` was left byte-identical: it already carried an alpha low byte in RGBA layout (never read anyway; removed entirely in Task 2).
- `rgb` removed from `text_field.rs`'s gpui import list (would otherwise be an unused-import warning). Post-swap grep confirms zero `rgb(` constructor calls remain in src/.
- Test count: 234 → 237 (4 added: `parse_color_rgba_preserves_authored_alpha`, `parse_color_rgba_prefix_and_whitespace_handling`, `parse_color_rgba_rejects_invalid_input`, `active_line_bg_sync_preserves_authored_alpha`; 1 removed: `blend_over_endpoints_and_midpoint`).

### Task 2: Token additions, remaps, and removals in the sync pipeline

**Files:**
- Modify: `src/theme.rs`
- Modify: `src/config.rs` (ThemeConfig keys)
- Modify: `src/text_field.rs` (consume `cursor` token)

- [x] add `border_variant`, `text_accent`, `editor_line_number`, `editor_active_line_number`, `editor_gutter_bg` to `Palette` + `AppTheme` + `palette_from_style` with the chains and fallbacks from Technical Details; expose each as a `[theme]` config key
- [x] remap `hover_row` (`ghost_element.hover` → `element.hover`), `match_highlight` (`text.accent`), `match_highlight_bg` (`search.match_background` → `search.active_match_background`, alpha kept), `active_line_bg` (`editor.active_line.background`, alpha kept)
- [x] remove `status_bar_bg`, `input_bg`, `cursor_selection` from Palette, AppTheme, sync, `apply_palette`, and the `[theme]` config surface (ThemeConfig in config.rs — compile-driven cleanup of all writes; `DEFAULT_STATUS_BAR_BG` at theme.rs:25 becomes unused → delete it, the warning-free gate requires it)
- [x] wire the `cursor` token (chain `editor.cursor` → fallback `text_accent` value; note: One Dark has no `editor.cursor` key, so the cursor lands on the accent `#74ade8` — same color the field used before via `match_highlight`, and the fallback base intentionally changes from the old unused `0x0A84FF`) into `TextFieldElement`'s cursor quad, replacing the hardcoded `match_highlight`; selection fill stays `(palette.selected_row & 0xFFFFFF00) | 0x44` (already converted in Task 1 — verify only)
- [x] write sync tests for each new/remapped chain (key present, key absent → next in chain, all absent → fallback), including alpha preservation for `match_highlight_bg`
- [x] write tests asserting the removed keys are gone from the config surface (unknown-key behavior stays serde-tolerant as today)
- [x] run tests - must pass before next task

**Task 2 notes:**
- Status-bar paint site (picker.rs:2950): chose the temporary `theme.bg` swap with a `// Task 3 makes this transparent` note (visually identical to the incoming transparent-over-`bg` design; the `status_bar_bg` field is fully gone now rather than lingering until Task 3).
- `match_highlight_bg` and `active_line_bg` chains were already correct after Task 1 (verified, no code change): `search.match_background` → `search.active_match_background` → `0x2C4870FF`, and `editor.active_line.background` → `selected_row @ 0x59`, both alpha-preserving.
- `DEFAULT_MATCH_HIGHLIGHT` now aliases a new `DEFAULT_TEXT_ACCENT` const (`0x4A9EFFFF`) since both tokens source `text.accent`; new `LINE_NUMBER_FALLBACK_ALPHA` const (`0x80`) documents the `text_dim`-derived gutter fallback.
- `input_text` (not in the removal list) was kept; `cursor_selection` removed as planned — TextField's selection fill verified unchanged at `(selected_row & 0xFFFF_FF00) | 0x44`.
- Test count: 237 → 250 (13 added — theme.rs: `border_variant_prefers_variant_key_then_border`, `text_accent_maps_key_with_accent_blue_fallback`, `editor_line_number_falls_back_to_translucent_text_dim`, `editor_active_line_number_falls_back_to_text_primary`, `editor_gutter_bg_falls_back_to_preview_bg`, `hover_row_prefers_ghost_element_hover`, `match_highlight_follows_text_accent_not_search_background`, `match_highlight_bg_chain_preserves_alpha`, `cursor_falls_back_to_text_accent`, `one_dark_sync_resolves_zed_parity_tokens`, `apply_palette_copies_zed_parity_tokens`; config.rs: `theme_config_parses_zed_parity_token_overrides`, `theme_config_ignores_removed_keys`).

### Task 3: Chrome and surface remap in picker.rs

**Files:**
- Modify: `src/picker.rs`

- [x] search row: bottom border color `border` → `border_variant`
- [x] status bar: remove `.bg(status_bar_bg)` (transparent over root surface), top border → `border_variant`; text colors unchanged
- [x] group separator rows and the results/preview split divider: `border` → `border_variant`
- [x] unselected rows (header/grep-match/file): stop explicitly painting `theme.bg` — only selected (`selected_row`) and hover (`hover_row`) paint backgrounds
- [x] checkbox outline stays `border`; verify checked fill reads `match_highlight` (accent) and text on fill still contrasts (uses `bg`)
- [x] extract/update any pure helpers touched and their tests (e.g. if a status-text or row-style helper carries color decisions); update existing tests asserting old token usage
- [x] run tests - must pass before next task

**Task 3 notes:**
- Status bar: removed the temporary `.bg(rgba(theme.bg))` left by Task 2 (the `status_bar_bg` field was already gone) — no bg call at all now; top border switched to `border_variant`.
- Unselected rows: the three `bg(if selected { selected_row } else { bg })` sites (header/grep-match/file) became `.when(selected, |d| d.bg(rgba(theme.selected_row)))` — no else branch, rows are transparent over the root surface; hover closures untouched and still paint on top.
- No pure helpers carried these color decisions (all inline in render closures) and no existing tests asserted the old tokens, so per the plan's testing note this task added no new tests and changed none. Test count unchanged: 250.
- Post-task grep: the only `theme.border` consumer left in picker.rs is the checkbox outline (picker.rs:491, intentional); checked fill verified as `match_highlight` with `theme.bg` checkmark text (both unchanged).

### Task 4: Fuzzy tint token + dead match-color cleanup

**Files:**
- Modify: `src/picker.rs`
- Modify: `src/preview.rs` (dead-parameter cleanup)

**Premise check (verified during plan review):** the grep match-row "fg recolor" does NOT exist in the working tree — `preview::overlay_match_ranges` already ignores its `_match_color` parameter and emits matched chunks with `color: span.color`, `bold: true`, translucent bg (preview.rs:438-448), and tests already assert that contract (`match_row_spans_overlays_ranges_across_syntax_span_boundaries`, `overlay_match_ranges_makes_matches_bold`). The Zed-exact match styling is already in place once Task 1 fixes the bg alpha. The substantive changes here are the fuzzy tint and cleanup:

- [x] `render_highlighted` `MatchStyle::Tint` (files-mode fuzzy filename tint, picker.rs:359): color source switches from `theme.match_highlight` to `text_accent`; no background (with `match_highlight` remapped to `text.accent` in Task 2 these coincide — still switch the field so intent is explicit)
- [x] remove the dead `_match_color` parameter from `preview::overlay_match_ranges` and the corresponding `theme.match_highlight` argument in `match_row_spans` (compile-driven; keeps the API honest)
- [x] verify the existing syntax-color-retention tests still pass unchanged (no new equivalent test needed — it exists); update any test touched by the parameter removal
- [x] run tests - must pass before next task

**Task 4 notes:**
- `render_highlighted` now tints matched chars with `theme.text_accent` (doc comment updated to say accent color, Zed-style, no background); non-matched chars stay `text_primary`.
- ➕ `overlay_match_ranges` had a SECOND picker.rs caller beyond `match_row_spans`: the preview-load overlay (picker.rs ~1584) also passed `match_highlight` — the compile-driven removal cleaned it too, deleting the now-unused `let match_highlight = theme.match_highlight;` capture (~1555). `match_row_spans` itself dropped its pass-through `match_color` parameter.
- Tests touched mechanically only (dead argument dropped, zero assertion changes): 7 `match_row_spans` calls in picker.rs tests, 4 `overlay_match_ranges` calls in preview.rs tests. Syntax-color-retention tests (`match_row_spans_overlays_ranges_across_syntax_span_boundaries`, `overlay_match_ranges_splices_across_span_boundary`, `overlay_match_ranges_makes_matches_bold`) pass unchanged, confirming matched chunks keep `span.color` + bold + translucent bg.
- Remaining `theme.match_highlight` consumers (intentional, out of scope): checkbox checked fill/outline (picker.rs ~496) and the search-row goose glyph (~2643) — both now render the accent via the Task 2 remap.
- Test count unchanged: 250. Gates: `cargo test` 250 green, `cargo build` warning-free, `cargo fmt --check` clean.

### Task 5: Preview gutter and line-number tokens

**Files:**
- Modify: `src/picker.rs`
- Modify: `src/preview.rs` (only if line-number color plumbing lives there)

- [x] paint the preview gutter with `editor_gutter_bg` — per-line gutter-cell background (each row's gutter cell painted) is the intended approach; a separate full-height column inside the virtualized `uniform_list` is NOT required (visually equivalent, much simpler). Identical to `preview_bg` in themes where the keys match
- [x] line numbers use `editor_line_number`; the centered match line's number uses `editor_active_line_number`
- [x] active-line wash (`active_line_bg`) and match-span overlays now blend with real alpha over `preview_bg` (constructor swap done in Task 1 — verify the layering order: gutter bg → active line wash → text)
- [x] extract/update pure helpers for the gutter color/number-color decision (row == match line?) and test them
- [x] run tests - must pass before next task

**Task 5 notes:**
- Layering choice (per the "match Zed" guidance): the row wash was previously painted on the whole row (gutter included). The row is now split into two cells — a gutter cell painted `editor_gutter_bg` and a `flex_1` text-area cell that carries the `active_line_bg` wash — so the wash covers only the text area and never the gutter, exactly Zed's structure. The pane's old `px(8.0)` row padding moved inside the cells (gutter cell absorbs the left 8px via `w(gutter_width + 8) + pl(8)` so the gutter column reaches the pane edge; the text cell takes `pr(8)`), keeping number alignment and code start-x pixel-identical.
- Pure helper `gutter_number_color(has_match, line_number, active_line_number)` in picker.rs picks `editor_active_line_number` for the centered match line (same `line_has_match` predicate as the wash) and `editor_line_number` otherwise; the old inline `text_dim @ 0x80` computation at the preview capture site is gone. The results-list grep-row gutter keeps its `text_dim @ 0x80` color (out of scope per Task 1's note — no task retokens it).
- No preview.rs changes: `line_has_match` is data-side; all color decisions live in picker.rs.
- Test count: 250 → 252 (added `gutter_number_color_uses_active_token_on_match_line`, `gutter_number_color_uses_line_number_token_otherwise`). Gates: `cargo test` 252 green, `cargo build` warning-free, `cargo fmt --check` clean.

### Task 6: Verify acceptance criteria

- [x] verify all requirements from Overview are implemented (alpha bugs fixed, border_variant chrome everywhere Zed uses it, transparent search row + status bar, Zed-exact match styling, text_accent fuzzy tint, cursor token wired, removed keys gone, window still edge-to-edge opaque)
- [x] verify edge cases: theme with no `border.variant` key (falls back to `border`), theme with no alpha anywhere (all-opaque — identical to pre-refactor rendering), config `#rrggbbaa` override, config `#rrggbb` override
- [x] run full test suite: `cargo test`
- [x] `cargo build` warning-free; `cargo fmt --check` clean
- [x] quick smoke launch (`./target/debug/fff-gpui --open . --grep`, kill after a few seconds) — no startup panic

**Task 6 notes (verification evidence):**
- Alpha bugs fixed: `parse_color_rgba` (theme.rs) keeps the 8-hex alpha byte; `active_line_bg_sync_preserves_authored_alpha` asserts One Dark `active_line_bg == 0x2f343ebf != bg (0x2f343eff)` and `match_highlight_bg == 0x74ade866`; match text sources `text.accent` (`match_highlight_follows_text_accent_not_search_background`), so text and bg can no longer collapse to the same color.
- `border_variant` chrome — exactly four picker.rs sites: group separator (~2127), search-row underline (~2650), results/preview split divider (~2821), status-bar top border (~3005). Sole remaining `theme.border` consumer is the checkbox outline (~502, intentional). One Dark resolves it to `0x363c46ff` (`one_dark_sync_resolves_zed_parity_tokens`).
- Transparent surfaces: search row and status bar have NO `.bg(...)` call (transparent over root `bg`); unselected rows paint no bg (`.when(selected, ...)` only, hover on top).
- Zed-exact match styling: `preview::overlay_match_ranges` emits matched chunks with `color: span.color`, `bold: true`, `bg: match_bg` (translucent) — covered by `overlay_match_ranges_makes_matches_bold` / `match_row_spans_overlays_ranges_across_syntax_span_boundaries`.
- Fuzzy tint: `render_highlighted` colors matched chars `theme.text_accent`, no bg (picker.rs ~360).
- Cursor token wired: `TextFieldElement` paints `rgba(palette.cursor)` (text_field.rs:537); chain `editor.cursor` → `text_accent` (theme.rs:1653; `cursor_falls_back_to_text_accent`).
- Removed keys gone: zero occurrences of `status_bar_bg`/`input_bg`/`cursor_selection` in src/ outside the config.rs test asserting they parse as ignored unknown keys (`theme_config_ignores_removed_keys`).
- Window: `WindowOptions` uses gpui defaults for background — `WindowBackgroundAppearance::default()` is `Opaque` (verified at the pinned rev, crates/gpui/src/platform.rs); root div is full-bleed `.bg(rgba(theme.bg))`, no rounding/shadow (only the 3px checkbox rounding exists).
- Edge cases: `border_variant_prefers_variant_key_then_border` covers variant-key / `border`-fallback / default; ➕ added `all_opaque_theme_yields_fully_opaque_palette` (all-6-hex theme → every one of the 27 palette color tokens resolves `..ff`); ➕ added `apply_color_override_accepts_opaque_and_alpha_hex` (config `#rrggbb` → implied-opaque, `#rrggbbaa` → alpha preserved, invalid/absent → unchanged).
- Regression scans: zero `rgb(` constructor calls in src/; goose glyph (search row, `match_highlight`), git bars (`git_status_bar_color` at the three row sites), and row metrics (36px search / 30px header / 28px rows / 28px status) all untouched; `git diff --stat` scope = the 9 files carrying the three uncommitted plans, nothing stray.
- Defects found: none — no code fixes needed.
- Gates: `cargo test` 254 green (252 + 2 added), `cargo build` warning-free, `cargo fmt --check` clean. Smoke launch (`./target/debug/fff-gpui --open . --grep`) alive after 5s, killed cleanly, empty stderr/stdout — no startup panic.

### Task 7: [Final] Update documentation

- [x] update README.md: `[theme]` key list (+`border_variant`, `text_accent`, `editor_line_number`, `editor_active_line_number`, `editor_gutter_bg`, `cursor`); note that hex values accept `#rrggbb` and `#rrggbbaa` (and shorthand). The removed keys (`status_bar_bg`, `input_bg`, `cursor_selection`) were never documented in README — add one sentence noting they are no longer accepted rather than deleting prose; one line on the Zed-style match highlighting
- [x] update CLAUDE.md if new patterns discovered (RGBA palette convention is worth one line in the architecture note)
- [x] move this plan to `docs/plans/completed/`
- [x] move the linked design file too: extract with `grep -E '^\*\*Design:\*\*' docs/plans/2026-07-22-zed-theme-parity.md | sed 's/^\*\*Design:\*\* *//'`; `test -f` then `mv` to `docs/plans/completed/` — warn and continue if missing

**Task 7 notes:**
- README: rewrote the `[theme]` overrides paragraph in Configuration — the stale "adds eight more" count is gone; key list now names `active_line_bg`, the six Zed-parity tokens (`border_variant`, `text_accent`, `cursor`, `editor_line_number`, `editor_active_line_number`, `editor_gutter_bg`), and the seven git-status colors; documents `#rrggbb`/`#rrggbbaa` (+ `#rgb`/`#rgba` shorthands) with live alpha blending; one sentence notes `status_bar_bg`/`input_bg`/`cursor_selection` are no longer accepted (ignored if present); one line on Zed-style match highlighting (syntax color kept, bold, translucent accent bg). Screenshot untouched (manual retake pending).
- CLAUDE.md: added the RGBA convention to the `theme.rs` architecture bullet (`0xRRGGBBAA` u32s, paint with `rgba()` never `rgb()`, live blending).
- Plan + design doc (`2026-07-22-zed-theme-parity-design.md`) moved to `docs/plans/completed/` via plain `mv` (no git staging — uncommitted tree).
- Gates (docs-only change, re-verified): `cargo test` 254 green, `cargo build` warning-free, `cargo fmt --check` clean.

## Post-Completion

*Items requiring manual intervention or external systems - no checkboxes, informational only*

**Manual verification:**
- Visual pass under One Dark + One Light + Gruvbox + Ayu (synced from Zed): input underline visible, status bar reads as part of the surface, preview clearly darker than results, active line visible in preview, match spans legible over syntax colors, fuzzy tint uses the accent
- Verify against a real Zed window side-by-side for One Dark
- Custom `[theme]` overrides in config.toml still win (spot-check one 6-hex and one 8-hex override)
- README screenshot retake (still pending from the previous rounds)
