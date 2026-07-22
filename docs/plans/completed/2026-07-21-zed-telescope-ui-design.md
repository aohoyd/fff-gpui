# Zed Telescope-Style UI for fff-gpui — Design

Date: 2026-07-21
Status: Approved

## Goal

Restyle fff-gpui to match Zed's new telescope-like search/picker UI (Zed PR #59604,
"Add preview to pickers and make them resizable"), with one fff-gpui-specific tweak:
a full-height git-state edge bar on each result row.

## Approved decisions

| Decision | Choice |
|---|---|
| Parity scope | Full parity incl. layout orientation: input at top, results top-down, preview right |
| Features | Visual restyle + resizable divider + preview line numbers + viewport-relative sizing |
| Git tweak | Full-row-height 3px left edge bar, theme-derived colors |
| Window mechanism | Unchanged — opaque `WindowKind::PopUp`, no titlebar, OS shadow, dismiss on focus loss. No transparent overlay. Parity applies to the window *interior* only |
| Resize scope | Divider only (no modal edge/corner handles) |
| Divider persistence | Session only (in-memory field, resets on daemon restart) |
| Config migration | Viewport percentages by default; existing px config values act as overrides (non-breaking) |
| Extras | Keep the 28px status bar (restyled) and the 🪿 goose glyph in the search row |
| Preview header | Dropped (Zed parity) |
| Preview line numbers | Always on, no config option |
| Implementation approach | A: in-place restyle of `FffPicker::render` + small sizing helper; no componentization |
| Git colors | Theme-derived (`version_control.*` / status keys) with current hexes as fallback |

## Reference: Zed's implementation (for values and patterns)

Zed repo: `~/Git/github.com/zed-industries/zed`

- Layout: `crates/picker/src/render.rs` (`render_editor` :96, `render_results` :144,
  `render_with_preview_right` :384)
- Sizing math: `crates/picker/src/shape.rs` (60%×60% viewport, preview 30% of width,
  min results 280×320px, min preview 128×96px, max 95% viewport)
- Preview mechanics: `crates/picker_preview/src/picker_preview.rs`
  (`update_from_buffer` :197, `scroll_to_focus_match` :269 — centering math)
- Row styling: `crates/ui/src/components/list/list_item.rs` (flat
  `ghost_element_selected` fill, hover `ghost_element_hover`)
- Match label highlighting: `crates/ui/src/components/label/highlighted_label.rs`
  (`text_accent` on matched char ranges only)
- Grep row rendering: `crates/search/src/text_finder/delegate.rs` :1150-1330
  (`search_match_background` bg + bold on matched substring, flat)
- Git color helper pattern: `crates/editor/src/items.rs:2200`
  (`entry_git_aware_label_color`)
- Key Zed values: search row `h_9` (36px) with `px_2p5` (10px); 1px `border_variant`
  dividers; rows `ListItemSpacing::Sparse` (≈28px total); preview divider `border_l_1`.

fff-gpui files touched:

- `src/picker.rs` — `impl Render for FffPicker` (:1407-2009), row rendering,
  `git_status_bar_color()` (:327), grep/file search population of `git_status`
- `src/theme.rs` — `AppTheme` (:59), `Palette` (:37), `palette_from_style()` (:1493),
  `sync_from_config()` (:740), existing tests at bottom
- `src/preview.rs` — `highlight_file_window()` (:444), `overlay_match_ranges()` (:348)
- `src/text_field.rs` — input box styling (drop the rounded box)
- `src/config.rs` — `AppConfig` sizing fields → `Option<f32>`
- `src/main.rs` — `open_window()` (:464) bounds computation
- `src/layout.rs` — NEW: viewport sizing + clamp + divider math
- `README.md` — Configuration section update

Constraint: GPUI comes from crates.io as `gpui 0.2.2` (not Zed's live source).
All APIs used — especially mouse drag handling and cursor styles for the divider —
must be verified against that exact version early.

## §1 — Window sizing & layout skeleton

Window: unchanged (`WindowKind::PopUp`, no titlebar, `is_resizable: false`, opaque,
OS shadow, terminate on focus loss). Only bounds computation and interior layout change.

Sizing (new `src/layout.rs`, modeled on Zed's `shape.rs` math):

- Default modal size = 60% × 60% of the active display.
- Clamps: results pane ≥ 280×320px, preview ≥ 128×96px, total ≤ 95% of display.
- `window_width` / `window_height` / `picker_pane_width` become `Option<f32>`:
  present = px override (today's behavior), absent = viewport math.
- `picker_pane_width` keeps its meaning (results-pane width). Unset → results 70%,
  preview 30% (Zed's default split).

Interior layout, top to bottom (flips today's orientation):

1. **Search row, 36px** (`h_9`), 10px horizontal padding: goose glyph + TextField,
   1px `border_variant` divider *below*.
2. **Body row (flex-1)**: results list left (`flex_1`) · 1px divider · preview right
   at computed preview width.
3. **Status bar, 28px**, restyled to new chrome, 1px top border.

Results direction: top-down — best match (rank 0) at the top directly under the
input; selection starts there; the current `total - 1 - visual_i` bottom-up
inversion is removed and scroll anchoring updated.

## §2 — Chrome, rows, git edges

Theme:

- `AppTheme.bg` remaps to the Zed theme's `elevated_surface_background`
  (fallback: `background`) so the modal surface matches Zed.
- Borders stay `border_variant`; `selected_row`/`hover_row` already map to
  `ghost_element_selected`/`ghost_element_hover` — unchanged.
- New tokens, plumbed through `Palette` → `AppTheme` → `theme_overrides` →
  `[theme]` config overrides like every other token:
  - `active_line_bg` ← `editor.active_line.background` (preview matched line)
  - five git colors (created / modified / deleted / conflict / ignored) ←
    the theme's `version_control.*` / status keys, falling back to today's hexes
    (`0x32D583`, `0xF5A524`, `0xF97066`, conflict, `0x6C6C70`).

Rows (28px kept):

- Selection = flat `ghost_element_selected` fill; hover = `ghost_element_hover`;
  no borders/outlines.
- File rows: filename at default text color with fuzzy-matched chars tinted
  `text_accent`; directory muted (replaces the current all-primary coloring).
- Grep rows: `filename:line` + matched line, matched substring gets
  search-match background + bold, **flat** (the rounded pill is removed).
- Git edge: the 3px×18px tick grows to a full-row-height 3px left bar,
  theme-derived color, on both file and grep rows; clean/no-status → no bar.

Search input: drop the TextField's boxed/rounded look — plain text on the modal
background, like Zed's picker input.

## §3 — Preview & divider

- Header: the 28px path header is removed; preview starts at the top edge.
- Gutter (new, always on): right-aligned line numbers per row inside the existing
  preview `uniform_list`; width sized to the digit count of the largest visible
  line number; muted text at ~50% opacity; 8px gap before code. Preview lines
  carry absolute line numbers (the windowed slice's start line is already known).
- Match display: matched lines get a full-width `active_line_bg` tint plus the
  tighter search-match bg on the matched substring (flat, bold — pill removed).
- Scroll: replace "8 lines above match, scroll to top" with Zed's centering:
  `target_row = match_row − (visible_rows − 1) / 2`, clamped ≥ 0;
  `visible_rows` = pane height / line height. The 500-line highlight window is
  centered on the match instead of anchored above it. Re-center after divider drags.
- Divider drag: 6px-wide invisible hit strip over the 1px divider, col-resize
  cursor; drag updates the split in a `FffPicker` session field (no persistence);
  clamped to results ≥ 280px / preview ≥ 128px; double-click resets to 70/30.
- Data flow unchanged: search → results → selection change → debounced (120ms)
  `load_preview` → highlight window → scroll.

## §4 — Config, errors, testing

Config:

- `window_width` / `window_height` / `picker_pane_width`: defaulted floats →
  `Option<f32>`. Absent = viewport math, present = px override. No new options.
- Existing `config.toml` files keep working identically (px values become overrides).
- README Configuration section documents the percentage defaults.

Error handling / fallbacks:

- Display-bounds lookup failure → today's 960×520 defaults.
- Theme missing `version_control`/status keys → hardcoded git hexes.
- Theme missing `elevated_surface_background` → `background`.
- Theme missing `active_line.background` → subtle alpha of the selection color.
- Tiny screens handled by the min/max clamps.

GPUI risk: verify divider drag + cursor APIs against crates.io `gpui 0.2.2` as the
**first implementation task**; if a needed API is missing, fall back to
keyboard-only resize keybinds.

Testing:

- Extend the existing `theme.rs` test suite for new token mappings and git-color
  fallbacks.
- Unit tests: layout math (percent sizing, px override, clamps, divider
  clamp/reset), gutter width, scroll-centering math.
- Visual verification via `fff-gpui --open` across One Dark / Ayu / Gruvbox in
  light + dark, plus a repo with modified/untracked/deleted files for edge bars.
