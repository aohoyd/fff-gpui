# Zed Picker Theme Parity — RGBA Palette & Surface Remap — Design

Date: 2026-07-22
Status: Approved (all sections)
Follows: `docs/plans/completed/2026-07-22-zed-grouped-results.md` (grouped results — currently **uncommitted** in the working tree together with the telescope restyle; this work builds on the tree as-is)

## Goal

Apply theme colors to fff-gpui's picker exactly the way Zed's telescope picker
does: correct per-surface token mapping (subtle `border_variant` chrome,
transparent search row / status bar over one elevated surface, distinct
editor-background preview), and honor translucent theme colors via an RGBA
palette — fixing two real bugs caused by the current alpha-channel drop
(invisible preview active-line highlight; illegible match highlight where the
match text color and its background resolve to the same opaque color).

## Zed reference (verified in ~/Git/github.com/zed-industries/zed, main @ 2026-07-21/22)

- Picker container: `Picker::render` applies `elevation_3(cx)` →
  `bg(elevated_surface_background)`, `rounded_lg` (8px), `border_1` with
  `border_variant`, 4-layer ModalSurface shadow (crates/ui/src/traits/styled_ext.rs:6-12).
  The workspace ModalLayer adds NO dimming wash for pickers
  (`fade_out_background()` = false).
- Search row (crates/picker/src/render.rs:96-126): transparent, `h_9` (36px),
  `px_2p5`, followed by `Divider::horizontal()` = 1px `border_variant`.
  Query editor: transparent bg, text `editor_foreground`, placeholder
  `text_placeholder`.
- Results list: transparent over the container surface. Rows (ListItem):
  selected `ghost_element_selected`, hover `ghost_element_hover`; line numbers
  `text_muted` @50% opacity; dir `text_muted`. Separators = `border_variant`.
- Preview (picker_preview): a real editor → bg `editor_background` (DISTINCT,
  darker than the surface), gutter `editor_gutter_background`, line numbers
  `editor_line_number` / `editor_active_line_number`, matched line
  `editor_active_line_background` (translucent), match span
  `search_match_background` (translucent). Divider between results and
  preview: 1px `border_variant`.
- Footer (crates/picker/src/footer.rs:95-123): transparent bg, `border_t_1`
  `border_variant`, `p_1p5`.
- Match text in result rows: syntax/normal color preserved + `FontWeight::BOLD`
  + translucent `search_match_background` behind it (delegate.rs:1327-1330) —
  the text is NOT recolored.
- Zed's picker never uses the strong `border` token — all chrome is
  `border_variant`.
- One Dark values: elevated_surface/surface `#2f343e`, background `#3b414d`,
  editor_background `#282c33`, border `#464b57`, border_variant `#363c46`,
  ghost_element_hover `#363c46`, ghost_element_selected `#454a56`,
  editor_active_line_background `#2f343ebf` (75% alpha),
  search_match_background `#74ade866` (40% alpha).

## Current fff-gpui state (bugs & gaps)

- `parse_color_rgb` truncates 8-hex colors to 6 — ALL alpha is dropped:
  - One Dark `editor.active_line.background` `#2f343ebf` → `0x2F343E` == `bg`
    exactly → the preview's active-line highlight is invisible.
  - `search.match_background` `#74ade866` feeds BOTH `match_highlight` (text
    color) and `match_highlight_bg` (background) → same opaque blue on blue →
    matched substrings illegible under sync.
- No `border_variant` token; all chrome uses the strong `border` key
  (single-key mapping, no fallback chain).
- Status bar paints `status_bar.background` (One Dark `#3b414d` — LIGHTER than
  the surface; opposite of Zed's transparent footer).
- `input_bg` is parsed but never painted (box chrome removed in the restyle).
  `cursor`/`cursor_selection` palette fields are written but never read;
  TextField hardcodes `match_highlight` as its cursor color.
- Rows explicitly repaint `theme.bg`; hover maps `element.hover` instead of
  `ghost_element.hover`; fuzzy filename tint sources a background token.

## Decisions (user-confirmed)

1. **Scope**: full Zed token parity (not minimal fixes).
2. **Window chrome**: keep edge-to-edge opaque window — NO rounded card, no
   window transparency, no shadow. Only internal surface remapping.
3. **Match style**: Zed-exact — matched substring keeps syntax/normal text
   color, gains bold, gets translucent `match_highlight_bg` behind it; no text
   recoloring.
4. **Fuzzy tint**: files-mode matched filename chars colored from `text.accent`.
5. **Config keys**: `status_bar_bg` and `input_bg` are DROPPED entirely from
   palette, sync, and `[theme]` config (breaking, documented).
6. **Alpha**: Approach A — RGBA palette (`0xRRGGBBAA`), live blending via
   `rgba()`; not pre-blending at sync time.

## §1 RGBA palette

- All `Palette`/`AppTheme` color fields become `0xRRGGBBAA` (alpha low byte).
- `parse_color_rgb` → `parse_color_rgba`: 8-hex keeps authored alpha; 6-hex
  appends `FF`. Invalid input keeps existing fallback-to-default behavior.
- Every paint site switches `rgb(theme.x)` → `rgba(theme.x)` (picker.rs,
  text_field.rs, preview line/gutter/span colors, git bars). Syntax styles
  unchanged (they carry their own colors).
- Hardcoded dark defaults gain `FF`. `active_line_bg` fallback becomes a
  genuinely translucent `selected_row @ ~0x59` (replacing the pre-blended
  opaque value); synced value keeps the theme's authored alpha.
- Config `[theme]` hex overrides accept `#rrggbb` (implied opaque) and
  `#rrggbbaa`.
- Drop never-read `cursor_selection`. Wire the `cursor` token (from
  `editor.cursor`, fallback accent) into TextField's cursor paint, replacing
  the hardcoded `match_highlight`. TextField selection fill stays
  `selected_row @ 0x44` as today.

## §2 Token additions & remapping

NEW tokens (each also a `[theme]` config key):
- `border_variant`: `border.variant` → fallback `border`.
- `text_accent`: `text.accent` → fallback current accent blue.
- `editor_line_number`: `editor.line_number` → fallback derived from `text_dim`.
- `editor_active_line_number`: `editor.active_line_number` → fallback `text_primary`-ish.
- `editor_gutter_bg`: `editor.gutter.background` → fallback `preview_bg`.

REMAPS:
- `hover_row`: `ghost_element.hover` → `element.hover` → default.
- `match_highlight` (fuzzy filename tint + checkbox fill accent): `text.accent`
  → fallback accent blue.
- `match_highlight_bg`: `search.match_background` →
  `search.active_match_background`, WITH alpha preserved.
- `active_line_bg`: `editor.active_line.background`, WITH alpha.

REMOVED: `status_bar_bg`, `input_bg` (palette + sync + config + docs).

UNCHANGED: `bg` (elevated_surface.background → background → surface.background
→ editor.background), `preview_bg` (editor.background → surface.background),
`selected_row` (ghost_element.selected chain), `border`, text tokens, git
tokens, `icon_muted`/`icon_accent`, fonts.

One Dark surface map result:
```
window/root      bg               #2f343e
search row       transparent      (bg shows)
─ divider        border_variant   #363c46
results rows     transparent      (bg shows)
  selected       selected_row     #454a56
  hover          hover_row        #363c46
│ split divider  border_variant   #363c46
preview          preview_bg       #282c33
  gutter col     editor_gutter_bg #282c33
  line numbers   editor_line_number
  active line    active_line_bg   #2f343e @75%
  match span     match_hl_bg      #74ade8 @40%
─ top border     border_variant   #363c46
status bar       transparent      (bg shows)
```

## §3 Surface & row styling changes

- Search row: transparent (unchanged), bottom border `border` → `border_variant`.
- Status bar: stops painting its own background — transparent over the surface
  (Zed-footer style); top border → `border_variant`; text stays `text_dim`.
- Group separators + results/preview divider → `border_variant`.
- Unselected rows stop explicitly repainting `theme.bg` (transparent; identical
  look, correct layering under translucent hovers).
- Grep match substrings: syntax/normal text color preserved + bold + translucent
  `match_highlight_bg` behind — no text recoloring.
- Files-mode fuzzy tint: matched filename chars `text_accent`, no bg.
- Preview: gutter column painted `editor_gutter_bg`; line numbers
  `editor_line_number`, with the match line's number `editor_active_line_number`;
  active-line wash and match overlays blend with real alpha over `preview_bg`.
- Checkbox checked fill follows the accent (`match_highlight` → text.accent).
- Unchanged: git edge bars, goose, 36px search row / 28px rows / 30px headers,
  window mechanism (opaque, edge-to-edge, PopUp, focus-loss dismiss).

## §4 Data flow, errors, testing, docs

- `palette_from_style` gains the new chains, loses removed ones; `apply_palette`
  and the `[theme]` override loop updated symmetrically.
- `[theme]` config: + `border_variant`, `text_accent`, `editor_line_number`,
  `editor_active_line_number`, `editor_gutter_bg`, `cursor`;
  − `status_bar_bg`, `input_bg` (breaking — called out in README).
- Errors: unparseable hex → existing fallback behavior.
- Testing (pure-helper policy): `parse_color_rgba` (6-hex, 8-hex, invalid,
  alpha preservation), each new/changed token chain via the existing theme-sync
  test suite, `match_row_spans` updated (bg+bold only, no fg recolor),
  fuzzy-tint token selection, active-line alpha survival, and updates to every
  existing test asserting old opaque values.
- Gates: all tests green, `cargo build` warning-free, `cargo fmt --check` clean.
- README: `[theme]` key list refreshed, match-styling note, removed keys
  documented.
