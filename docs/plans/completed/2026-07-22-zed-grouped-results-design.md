# Zed-Parity Grouped Grep Results, Per-Match Multiselect & gpui Git Upgrade — Design

Date: 2026-07-22
Status: Approved (all sections)
Follows: `docs/plans/completed/2026-07-21-zed-telescope-ui.md` (telescope restyle — currently **uncommitted** in the working tree; this work builds directly on top of it)

## Goal

Align fff-gpui's results panel with Zed's new telescope-style text finder:
grep results grouped by file (header rows + one row per match), Zed-style
match-row visuals (line-number gutter, syntax-highlighted lines, match tint),
Zed-style multiselect (explicit mode, checkboxes, tab = toggle+advance,
per-match granularity), collapsible file groups, and an upgrade of the gpui
dependency from crates.io 0.2.2 to Zed's git repo for full parity.

## Zed reference (verified in ~/Git/github.com/zed-industries/zed, main @ 2026-07-21)

- Feature lives in `crates/search/src/text_finder/` (delegate.rs is the core)
  on top of the generic `crates/picker` crate. PR #59604.
- Data model: flat `matches: Vec<SearchMatch>` projected into
  `entries: Vec<Entry>` with `Entry::{Header(ProjectPath), Match(usize), Separator}`
  (`rebuild_entries`, delegate.rs:366-402). `collapsed_paths: HashSet<ProjectPath>`
  omits a file's Match entries when folded. Selection re-anchors across rebuilds.
- Rendered with a **variable-height** list (`Picker::list_with_preview`,
  `ContainerKind::List`), not uniform_list.
- Header row: Disclosure chevron (ChevronDown/ChevronRight, muted) + file icon
  (muted, small) + filename (Label small) + directory (small, muted,
  `.truncate_start()`); selected bg = `ghost_element_selected`.
- Match row: right-aligned line-number gutter sized `digits × 0.5rem`
  (`justify_end`), number color `text_muted.opacity(0.5)`, `gap_2p5`, `text_sm`,
  single-line `.truncate()`. Line text is tree-sitter highlighted; the match
  substring gets `search_match_background` + `FontWeight::BOLD`
  (delegate.rs:1327-1331). Context clipped to 512 bytes each side.
- Separator: `Divider::horizontal()` with `py(Base04)`, between groups only.
- Multiselect (picker crate): `ToggleMultiSelect` (cmd-shift-s),
  `MultiSelectNext` (tab = toggle current + advance, auto-enters mode),
  cmd-click toggles. Indicator = Checkbox start_slot; no row tint, no counter.
  Confirm with selection → `confirm_multi` opens every selected match.
  Selection storage is per-match (path + range). Headers/separators can't be
  selected (`search_match_for_entry` returns None).
- Navigation: `can_select` — Match always; Header only when collapsed;
  Separator never. Arrow keys step until selectable. Enter on collapsed
  header is a no-op.
- Fold: fold/unfold actions + chevron click; alt-click chevron toggles all;
  collapsed state cleared on new search, preserved while results stream.

## Decisions (user-confirmed)

1. **Grouping**: one row per match under file headers, **all matches**
   (no per-file cap; page limit / time budget still bound totals).
2. **Multiselect granularity**: **per-match** in grep mode; per-file in Files mode.
3. **Features in scope**: collapsible file groups, syntax-highlighted result
   rows, checkbox multiselect UI, group separators — all four.
4. **Tab**: toggle + advance (Zed parity). shift-tab unchanged.
5. **Checkboxes**: explicit mode like Zed — cmd-shift-s toggles the mode
   (off = clear selection); tab and cmd-click auto-enter it.
6. **Headers**: Zed parity — arrows skip expanded headers, collapsed headers
   are selectable, Enter on a collapsed header is a no-op, headers can't be
   multi-selected.
7. **Pills**: dropped in grep mode (no count, no ✨ on match rows);
   ✨ frecency pill stays in Files mode.
8. **Multi-open**: dedupe by file — each file opens once, at its first
   selected match's line.
9. **Fold keys (custom, diverges from Zed)**: `alt-z` toggle fold current
   group, `alt-shift-z` toggle fold all; chevron click toggles group,
   **alt-click chevron toggles all**.
10. **Mode key**: `cmd-shift-s`.
11. **List infra**: **variable-height `gpui::list` (ListState)** — Zed-faithful
    heights and padded separators (Option B chosen over uniform_list).
12. **Row highlighting**: per-line standalone tree-sitter (Option i) —
    computed in the background search task; accepts rare mis-colors inside
    multiline constructs.
13. **gpui**: upgrade to git dependency for full Zed parity.

## §1 gpui upgrade (Task 1 — everything else builds on it)

- `Cargo.toml`: `gpui = { version = "*" }` → `gpui = { git = "https://github.com/zed-industries/zed" }`.
  Cargo.lock pins the exact rev (reproducible builds incl. Homebrew
  from-source); future bumps via `cargo update -p gpui`.
- Verified against Zed main @ 2026-07-21 (crates/gpui still versioned 0.2.2):
  identical signatures for `ListState::{new, reset, splice, scroll_to,
  scroll_to_reveal_item}`, `list()`, `uniform_list` + `scroll_to_item_strict`,
  `on_drag`, `on_drag_move::<T>`, `DragMoveEvent`, `cursor_col_resize`,
  `EmptyView`. Expected fallout: zero-to-minor compile fixes.
- Zed's `ui` component crate (Disclosure/Checkbox/ListItem) is unpublished and
  NOT pulled in — chevron and checkbox are hand-rolled divs in app style.
- README build section: note that the first build fetches the zed repo
  (large download).

## §2 Data model & grouping

- `execute_grep_search` (src/picker.rs): `max_matches_per_file` 5 → 200
  (= page_limit; effectively uncapped per page).
- `GrepMatchLine` gains `syntax_spans` (spans of `line_content` with resolved
  theme colors, or plain fallback), computed in the background search task via
  a new pure helper in src/preview.rs that runs the existing tree-sitter
  per-language highlighter over the single line (language from file
  extension; plain-text fallback for unknown/failed).
- New row projection in src/picker.rs:
  `enum ResultRow { Header(usize), Match { file: usize, m: usize }, Separator }`
  and `rows: Vec<ResultRow>`, built by pure
  `build_rows(&results, &collapsed, view)`:
  - Files view: one flat Match-like row per file — no headers/separators.
  - Grep view: Separator (except before first group) + Header + the file's
    Match rows unless its path ∈ `collapsed`.
- Fold state: `collapsed: HashSet<PathBuf>`, cleared on new search and on
  mode switch.
- `selected: usize` indexes `rows`. On rebuild, selection re-anchors to the
  same (file path, match index) when possible (pure helper).
- Multiselect storage replaces `selected_paths: BTreeSet<PathBuf>` with
  `BTreeSet<SelectionKey>`, `SelectionKey = (PathBuf, Option<(u64 /*line*/, u32 /*col*/)>)`:
  per-match in grep (`Some((line, col))`), per-file in Files mode (`None`).
  Pruned to still-visible results after each search.
- Widest line number across visible matches drives gutter width.

## §3 Visual spec

```
┃˅ 🗋 helm.yaml  .gitlab/includes         ← header ~30px
┃    26  HELM_USER: "$HELM_USER"          ← match rows 28px
┃    42  --username "$HELM_USER" \
 ──────────────────────────────           ← separator: 1px border, 8px v-pad
 ˃ 🗋 stand.yaml  .gitlab/includes        ← collapsed group (chevron rotated)
 ──────────────────────────────
┃˅ 🗋 AGENTS.md
┃   ☑  17  ├─ docs/        # User docs   ← checkbox slot only in multiselect mode
┃   ☐  21  ├─ auth-center/ # JWT issuer
```

- **Header row** (~30px): chevron ˅/˃ (rotates when collapsed, `icon_muted`),
  16px file icon, filename `text_sm` `text_primary`, directory `text_xs`
  `text_secondary` truncated from the left; bg `selected_row` only when
  collapsed AND cursor-selected; `hover_row` on hover.
- **Separator**: 1px `theme.border` line, 8px vertical padding, between
  groups only.
- **Match row** (28px): [16px checkbox slot when mode active] →
  right-aligned line-number gutter (width from widest line number's digit
  count; color `text_dim` @ 50% alpha) → gap → line text with tree-sitter
  colors from `theme.syntax_styles`, match ranges overlaid
  `match_highlight_bg` + bold (pure span-merge helper), single line,
  truncated. No pills.
- **Files-mode rows**: unchanged (icon, fuzzy-tinted name, dir, ✨ pill) +
  checkbox slot in mode.
- **Checkbox**: 14px rounded square, `theme.border` outline; checked =
  accent (`match_highlight`) fill + check glyph. Replaces the ● dot.
- **Git edge bar**: 3px full-height on header AND match rows → continuous
  colored edge per file group; transparent when clean.
- Cursor row keeps `selected_row` bg.

## §4 Interaction

- `multi_select_mode: bool`. `cmd-shift-s` toggles; toggling OFF clears the
  selection. `tab` and cmd-click auto-enter the mode.
- `tab`: enter mode + toggle current row + advance to next selectable row.
  `shift-tab` unchanged (grep submode cycle / move-up in Files).
  `ctrl-a`: toggle-all visible matches (enters mode).
- cmd-click toggles a row's selection; plain click moves cursor + preview
  (as today).
- Navigation `can_select`: Match rows always; Headers only while collapsed;
  Separators never. `select_next/prev` step until selectable; no wrap.
- Folding: `alt-z` toggles current row's group (collapse re-anchors cursor to
  the header, expand to first match); `alt-shift-z` toggle-all (any collapsed
  → expand all, else collapse all); chevron click = that group; alt-click
  chevron = toggle all. Cleared on new search / mode switch.
- Enter/open: selection non-empty → dedupe by path, open each file ONCE at
  its first selected match's line (BTreeSet order), `track_open` per file.
  Else current row: Match opens at THAT match's line (`goto_for_path` now
  uses the selected match, not the file's first); collapsed Header = no-op.
- Esc unchanged (quits).
- Status bar left: `"N matches in M files  K selected  X indexed"`
  (+ grep submode hint). Right:
  `"↑↓ nav  ⇥ mark  ⌘⇧S multi  ⌥Z fold  ⏎ open  esc quit"`.

## §5 List infra, preview, errors, testing

- Results pane: replace `uniform_list` with variable-height `list(ListState)`;
  state on `FffPicker`; `reset(rows.len())` after each new search (scroll to
  top); `splice(range, count)` on fold toggles (preserves scroll);
  `scroll_to_reveal_item(selected)` on arrow navigation. Preview pane keeps
  its `uniform_list` (uniform line heights). Row heights measured naturally
  by the list element.
- Preview: selecting a match row loads that file centered on THAT match's
  line with all the file's matches overlaid; collapsed header previews the
  file centered on its first match.
- Errors: unknown language / highlight failure → plain uncolored spans;
  unreadable file → existing empty-preview path; editor-open failure →
  existing `status_message` path.
- Testing (pure-helper policy; render closures untested): `build_rows`
  (grouping/separators/folding/files-mode), selection re-anchoring across
  rebuilds, `can_select` + stepping, SelectionKey ordering + once-per-file
  open dedupe, syntax×match span merge, gutter digit-width, toggle-all fold
  semantics, status-bar text. All existing tests stay green; `cargo build`
  warning-free; `cargo fmt --check` clean.
- README: grouped results + multiselect + fold keybinds documented; gpui
  git-dep build note; screenshot retake remains a manual item.
