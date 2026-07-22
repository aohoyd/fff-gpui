# Zed Picker Widget Parity — Design

Date: 2026-07-22
Status: Approved (brainstorm)

## Goal

Bring five picker UI details into visual parity with Zed's picker-based
search UI ("Text Finder"), using Zed's sources at
`~/Git/github.com/zed-industries/zed` as the reference:

1. Grep-mode file-header font size
2. Fold (collapse/expand) toggle
3. Multiselect checkbox
4. Multiselect-mode pictogram/indicator
5. Default preview-pane width ratio (50%)

Reference paradigm chosen by user: **Zed's picker-based Text Finder**
(`crates/search/src/text_finder/delegate.rs`), not the editor multibuffer
project-search view. Full parity chosen for fold toggle, checkbox, and
mode indicator (always-visible clickable toggle).

## Zed reference specs (verified against sources)

- **Header font**: Text Finder file header renders filename with
  `Label::new(file_name).size(LabelSize::Small)` = 12px UI font;
  directory same 12px, `Color::Muted`, `truncate_start()`
  (`crates/search/src/text_finder/delegate.rs:1133-1145`).
- **Fold toggle**: `Disclosure` component = IconButton with
  `IconName::ChevronDown` (open) / `IconName::ChevronRight` (closed),
  `IconSize::Small` (14px), `Color::Muted`, hover background; positioned
  left of the file icon; alt-click toggles all
  (`crates/ui/src/components/disclosure.rs:30-31,92-115`,
  `delegate.rs:1086-1132`).
- **Checkbox**: 20px hit target, 16px box, `rounded_xs` (2px), 1px border
  (theme `border`), subtle fill when checked (not accent-filled); checked
  shows `IconName::Check` at `IconSize::Small` (14px) in `text_accent`
  (`crates/ui/src/components/toggle.rs:43-287`). Only match rows get
  checkboxes; header rows never do
  (`crates/picker/src/picker.rs:1397-1412`, `delegate.rs:1150-1188`).
- **Multiselect-mode indicator**: always-visible `IconButton` with
  `IconName::FileMultiple`, `IconSize::Small` (14px), at the trailing
  edge of the query-input row; icon tint switches from default to
  `text_accent` when multiselect mode is active; clicking dispatches
  the toggle action. No "N selected" count exists anywhere in Zed
  (`crates/picker/src/render.rs:111-142`).
- **Preview split**: picker preview defaults to exactly 50:50
  (`crates/picker/src/shape.rs:647-671`); Zed's min sizes (280px
  results / 128px preview) already match ours.
- **SVG assets** (repo-relative in Zed): `assets/icons/chevron_down.svg`,
  `assets/icons/chevron_right.svg`, `assets/icons/check.svg`,
  `assets/icons/file_multiple.svg`. Note: the Disclosure chevrons differ
  slightly (wider sweep) from `assets/icons/file_icons/chevron_*.svg`,
  which is what we currently vendor for file icons — vendor the
  `assets/icons/` variants.

## Current state (this repo)

- Header filename: `.text_sm()` (14px) at `src/picker.rs:2258`;
  directory `.text_xs()` (12px) at `src/picker.rs:2265`.
- Fold toggle: Unicode `\u{02C3}`/`\u{02C5}` glyphs at
  `src/picker.rs:2236-2253`, no hover background.
- Checkbox: hand-drawn in `render_checkbox` (`src/picker.rs:496-529`) —
  14px box, 3px radius, accent-filled when checked with bg-colored ✓ glyph.
- No multiselect-mode indicator (mode toggled via cmd-shift-s only).
- Preview split: `DEFAULT_RESULTS_FRACTION = 0.70` (`src/layout.rs:27`),
  i.e. 70/30 default.
- Theme tokens available: `text_accent`, `hover_row`, `border`,
  `icon_muted`, `bg` (see `src/theme.rs:210-246`).

## Design

Approach chosen: **component module** (Approach B).

### Assets

- New `vendor/zed/icons/` with 4 SVGs copied verbatim from Zed
  `assets/icons/`: `chevron_down.svg`, `chevron_right.svg`, `check.svg`,
  `file_multiple.svg`.
- `src/assets.rs`: add `#[include = "icons/**/*"]` to the existing
  `Assets` embed (folder `vendor/zed`). Asset keys become
  `icons/check.svg` etc., matching Zed's key convention. gpui tints
  monochrome SVGs via `.text_color()`; no color edits needed.

### New module `src/ui.rs`

Zed-style component builders; gpui-bound render code, no testable logic:

- `disclosure(id, expanded, &theme)` — fold chevron button.
- `checkbox(id, checked, &theme)` — multiselect checkbox.
- `icon_button(id, icon_path, active, &theme)` — toggle-capable icon
  button (used for the multiselect-mode toggle).

Each returns a styled `Stateful<Div>` (id attached); `picker.rs` call
sites chain `.on_click(...)` themselves — no callback plumbing across
the module boundary. `picker.rs` keeps all orchestration/event wiring;
its `render_checkbox` helper is deleted. Sizing constants that feed
row-chrome/truncation math stay in `layout.rs` (tested) and `ui.rs`
references them.

### The five changes

1. **Header font**: `render_header_row` filename `.text_sm()` →
   `.text_xs()` (12px). Directory stays 12px muted. Both now 12px,
   color the only differentiator (Text Finder parity).
2. **Fold toggle**: replace glyph div with `ui::disclosure` — 16×16
   button, 2px corner radius, hover bg `theme.hover_row`, 14px svg
   (`icons/chevron_down.svg` expanded / `icons/chevron_right.svg`
   collapsed) tinted `theme.icon_muted`. Click/alt-click behavior,
   keybindings (`alt-z`, `alt-shift-z`), and fold state untouched.
3. **Checkbox**: `ui::checkbox` — box 14→16px, radius 3→2px, border 1px
   `theme.border`. Checked state: subtle fill + 14px `icons/check.svg`
   in `theme.text_accent` (replaces accent-filled box with ✓ glyph).
   Unchecked: transparent + border. Fits existing `ROW_LEADING_SLOT`
   (24px = 16 + 8 gap) — no truncation-math change. Placement
   unchanged: match rows and files-mode rows only, never headers.
4. **Multiselect toggle**: query-input row gains trailing
   `ui::icon_button` with `icons/file_multiple.svg` at 14px — always
   visible in both grep and files modes, `theme.icon_muted` idle,
   `theme.text_accent` when multiselect mode active, hover bg
   `theme.hover_row`. Click dispatches existing `ToggleMultiSelectMode`
   (same as cmd-shift-s).
5. **Preview split**: `DEFAULT_RESULTS_FRACTION` 0.70 → 0.50 in
   `src/layout.rs`. Session drag value and `picker_pane_width` config
   override keep precedence; double-click divider reset lands at 50/50.

### Testing

- `layout.rs`: the fraction change is the only pure-logic change —
  update split tests (1000px modal → ~499.5/499.5), re-check
  min-width/clamp tests; invariants unchanged.
- `rows.rs`: no changes (fold semantics, selection keys, mode state
  untouched — purely visual pass).
- `ui.rs`: render-only by construction; per project GPUI testing policy
  nothing testable lives there. Any new constant feeding truncation
  math goes to `layout.rs` with a test.

### Data flow / error handling

No new state: toggle button reads existing `multi_select_mode` and
dispatches existing action; disclosure reads `collapsed`; checkbox reads
`selection`; colors via `theme::current()`. No fallible paths — embedded
SVGs are compile-time guaranteed by `rust_embed`.

### Docs

Update README preview-pane default-ratio mention to 50/50 if present.

## Quality gates (CLAUDE.md)

- `cargo test` — all green
- `cargo build` — warning-free
- `cargo fmt --check` — clean
