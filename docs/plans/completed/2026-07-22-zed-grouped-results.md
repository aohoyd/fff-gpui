# Zed Grouped Results

> **For Claude:** use `/planning:execute` to implement this plan task-by-task with fresh subagents.

**Goal:** Group grep results by file with Zed-style header/match/separator rows in a variable-height list, per-match multiselect with an explicit checkbox mode, collapsible file groups, per-line syntax-highlighted match rows, and a gpui upgrade from crates.io 0.2.2 to Zed's git repo.

**Architecture:** A derived row projection (`ResultRow::{Header, Match, Separator}` built by a pure `build_rows`) sits between the existing per-file `FileItemSnapshot` results and a variable-height `gpui::list(ListState)` that replaces the results `uniform_list`. Selection, navigation, folding, and multiselect all operate on row indices with pure, unit-tested helpers; match-line syntax spans are computed in the background search task via the existing tree-sitter infrastructure in `preview.rs`.

**Tech Stack:** Rust, gpui (git dependency on zed-industries/zed), fff-search / fff-grep 0.10.1, tree-sitter + tree-sitter-highlight.

**Design:** docs/plans/2026-07-22-zed-grouped-results-design.md

## Overview

- Grep results currently render one 28px row per FILE (first match inline, count pill capped at 5). This plan makes them Zed-parity: one row per MATCH grouped under collapsible file headers, with separators between groups, a right-aligned dimmed line-number gutter, tree-sitter-colored line text, and the match substring tinted (`match_highlight_bg` + bold). Pills are dropped in grep mode.
- Multiselect moves from per-file (`BTreeSet<PathBuf>`, ● dot) to Zed's model: explicit mode (`cmd-shift-s`, auto-entered by `tab`/cmd-click), checkbox column, `tab` = toggle + advance, per-match granularity, Enter opens each file once at its first selected match's line.
- gpui switches from the stale crates.io 0.2.2 release to a Cargo.lock-pinned git dependency on the zed repo (API surface verified identical for everything the app uses).
- Files mode keeps its flat one-row-per-file layout and gains only the checkbox mode + tab-advance semantics.

## Context (from discovery)

- **The working tree has uncommitted changes** from the completed telescope restyle (`docs/plans/completed/2026-07-21-zed-telescope-ui.md`). Build on the tree as-is; never `git checkout`/revert files (a fixer destroyed work that way last time). Baseline: 126 tests passing, warning-free build, fmt clean.
- `src/picker.rs` (~2450 lines): `FffPicker` — results state (`results: Arc<Vec<FileItemSnapshot>>`, `selected: usize`, `selected_paths: Arc<BTreeSet<PathBuf>>`), search pipeline (`run_search`, `execute_grep_search` at ~456-563 which already groups matches per file into `FileItemSnapshot.grep_matches`, capped by `max_matches_per_file: 5` at ~503), row rendering inside `impl Render` (~1502-2193, `uniform_list` at ~1810), actions (`actions!` at ~64-81), handlers (`on_select_next/prev`, `on_select_row`, `on_toggle_selected` ~1362, `on_toggle_select_all` ~1380, `on_open_selected` ~1226, `goto_for_path` ~1173), `git_status_bar_color` ~420, `segment_matches`/`render_highlighted` ~281-350, status bar ~2161-2192.
- `src/preview.rs`: tree-sitter per-language `HighlightConfiguration`s, `HighlightedLine`/`HighlightedSpan`, `syntax_lines_for_path`, `overlay_match_ranges` (adds bg+bold to match chunks — reusable for result rows), `window_start_line`.
- `src/main.rs` `bind_base_keys` (~main.rs:272-306): `tab`→ToggleSelected, `shift-tab`→ShiftTab, `ctrl-a`→ToggleSelectAll, arrows, enter, escape. Actions are imported via `use picker::{...}` at main.rs:30-34 — new actions must be added there too.
- `src/theme.rs`: `AppTheme` tokens incl. `match_highlight`, `match_highlight_bg`, `selected_row`, `hover_row`, `text_dim`, `text_secondary`, `icon_muted`, `syntax_styles`, git colors.
- `src/layout.rs`: pure math helpers (`gutter_width`, `scroll_center_row`) with tests.
- gpui 0.2.2 (crates.io) vs zed git main @ 2026-07-21: `crates/gpui` still versioned 0.2.2; identical signatures for `list()`, `ListState::{new, reset, splice, scroll_to, scroll_to_reveal_item, logical_scroll_top, scroll_by}`, `ListAlignment`, `ListOffset`, `uniform_list` + `scroll_to_item_strict`, `on_drag`, `on_drag_move`, `DragMoveEvent`, `cursor_col_resize`, `EmptyView`.
- Zed reference behavior documented in the design doc (§ "Zed reference").

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
- maintain backward compatibility (Files mode behavior, config, IPC responses)
- GPUI render closures are untestable — extract pure functions (row building, stepping, merging, dedupe, text formatting) and test those; this is the established project policy
- **never revert or `git checkout` working-tree files** — the tree holds uncommitted prior work

## Testing Strategy

- **unit tests**: required for every task — see the testing mandate in Development Approach above (not restated here)
- **e2e tests**: none in this project (GPUI app; no e2e harness). Manual verification items live in Post-Completion.
- Gates for every task: `cargo test` all green, `cargo build` warning-free, `cargo fmt --check` clean.

## Progress Tracking

- mark completed items with `[x]` immediately when done
- add newly discovered tasks with ➕ prefix
- document issues/blockers with ⚠️ prefix
- update plan if implementation deviates from original scope
- keep plan in sync with actual work done

## Solution Overview

- **Row projection**: `results` stays the per-file snapshot vec; a derived `rows: Vec<ResultRow>` (Header/Match/Separator) is rebuilt on search apply, fold toggle, and view switch. `selected` indexes `rows`. Pure helpers own all row logic so they're unit-testable.
- **Variable-height list**: results pane switches to `gpui::list(ListState)` (Zed-faithful heights: ~30px headers, 28px match rows, padded 1px separators). Preview pane keeps `uniform_list`.
- **Per-line highlighting**: background search task runs tree-sitter over each match's `line_content` alone (language by extension, plain fallback); render overlays match ranges via the existing `overlay_match_ranges` span logic.
- **Multiselect**: `SelectionKey = (PathBuf, Option<(u64, u32)>)` in a `BTreeSet`; `multi_select_mode` flag gates the checkbox column; `cmd-shift-s` toggles mode (off clears), `tab` toggles + advances, `ctrl-a` toggles all, cmd-click toggles a row.
- **Folding**: `collapsed: HashSet<PathBuf>`; `alt-z` group toggle, `alt-shift-z` toggle-all, chevron click / alt-click; cleared on new search & mode switch.

## Technical Details

- `ResultRow` (new pure module `src/rows.rs`):
  ```rust
  pub enum ResultRow {
      Header(usize),                       // index into results
      Match { file: usize, m: usize },     // results[file].grep_matches[m]
      Separator,
  }
  ```
  - `build_rows(&[FileItemSnapshot], &HashSet<PathBuf>, is_grep) -> Vec<ResultRow>`: Files view → one `Match { file, m: 0 }`-shaped row per file (no headers/separators; render branches on view); Grep view → `Separator` (except before first) + `Header` + match rows unless collapsed. (`FileItemSnapshot` is a plain data struct — construct directly in tests.)
  - `can_select(rows, ix, collapsed, results) -> bool`: Match → true; Header → collapsed only; Separator → false.
  - `step_selectable(rows, from, direction, ...) -> Option<usize>`: walk until selectable, no wrap.
  - `anchor_selection(old_key, new_rows, results) -> usize`: re-anchor after rebuild (same path + match index if present, else nearest).
  - `max_line_number(results) -> u64` for gutter width (reuse `layout::gutter_width`).
  - `dedupe_opens(selection) -> Vec<(PathBuf, Option<(u64, u32)>)>`: first selected match per file, BTreeSet order.
- `GrepMatchLine` gains `col: u32` (from `GrepMatch.col`, a 0-based BYTE offset, `usize` — cast `as u32`) and `syntax_spans: Vec<preview::HighlightedSpan>` (empty = plain).
- `SelectionKey`: `(PathBuf, Option<(u64 /*line*/, u32 /*col*/)>)` — grep rows `Some`, Files rows `None`. Replaces `selected_paths`; status-bar count = key count. **The key's `col` is match identity ONLY** — editor goto columns must keep `goto_for_path`'s existing 1-based char-column computation (picker.rs:1180-1189), parameterized by the selected match, never the raw byte offset.
- New actions in `actions!(fff_picker, ...)`: `ToggleMultiSelectMode` (cmd-shift-s), `ToggleFold` (alt-z), `ToggleFoldAll` (alt-shift-z). Existing `ToggleSelected` (tab) becomes toggle+advance; `ToggleSelectAll` (ctrl-a) enters mode.
- `ListState` lifecycle: created once (`ListState::new(0, ListAlignment::Top, px(overdraw))`); `reset(rows.len())` on search apply; `splice(changed_range, new_count)` on fold toggles (preserves scroll); `scroll_to_reveal_item(selected)` after arrow nav; `scroll_to(ListOffset::default())` on new search.
- Match-row text: `HighlightedLine { spans: syntax_spans }` + `overlay_match_ranges(line, byte_ranges)` → render spans (fg colors, match chunks get `match_highlight_bg` + bold). Empty `syntax_spans` → single plain span first.
- Open flow: selection non-empty → `dedupe_opens`; each entry opens via existing `track_open` + responder/editor path at `Some((line, col))` or file default. Cursor-row open: Match row uses its own `(line, col)`; collapsed Header no-op.
- Header dir text truncated from the left (Zed `.truncate_start()` equivalent: reuse/extend `shorten_dir_for_row` or overflow-hidden with direction-aware truncation).

## What Goes Where

- **Implementation Steps** (`[ ]` checkboxes): all code, tests, docs changes below.
- **Post-Completion** (no checkboxes): manual visual/UX verification, screenshot retake.

## Implementation Steps

### Task 1: Upgrade gpui to the zed git dependency

**Files:**
- Modify: `Cargo.toml`
- Modify: `Cargo.lock` (via cargo)
- Modify: `docs/plans/2026-07-22-zed-grouped-results.md` (record pinned rev + any API drift found)

- [x] change `gpui = { version = "*" }` to `gpui = { git = "https://github.com/zed-industries/zed" }` in `Cargo.toml`
- [x] `cargo update -p gpui` / `cargo build` to resolve and pin the rev in `Cargo.lock`; record the pinned rev in this plan's task notes
- [x] fix any compile fallout from API drift (expected zero-to-minor; verify `list`, `ListState::{new,reset,splice,scroll_to,scroll_to_reveal_item}`, `ListAlignment`, `ListOffset`, `uniform_list`/`scroll_to_item_strict`, `on_drag`, `on_drag_move`, `DragMoveEvent`, `cursor_col_resize`, `EmptyView` all compile)
- [x] confirm the app still runs (`cargo run -- --open . --grep` smoke check compiles/launches)
- [x] run existing test suite — all existing tests must pass unchanged (this task adds no new logic; existing tests are the required coverage)
- [x] `cargo build` warning-free, `cargo fmt --check` clean - must pass before next task

**Task 1 notes:** Pinned rev `bb87c47c92d0110790ee79a0fdf4a9c8b7119fb3` (gpui 0.2.2). API drift was larger than pre-verified: (1) gpui split its platform layer into a new `gpui_platform` crate — `Application::new()` is gone, so `Cargo.toml` gained `gpui_platform = { git = ..., features = ["font-kit"] }` (font-kit is REQUIRED on macOS or no text renders) and `main.rs` now uses `gpui_platform::application()`; (2) `Window::focus(&handle)` → `focus(&handle, cx)` (main.rs, picker.rs); (3) `App::on_window_closed` closure is now `FnMut(&mut App, WindowId)` (2 sites, main.rs); (4) `ShapedLine::paint` gained `align: TextAlign, align_width: Option<Pixels>` params (text_field.rs, passes `TextAlign::Left, None`); (5) `uniform_list(...).track_scroll` now takes `&UniformListScrollHandle` (2 sites, picker.rs); (6) rust-embed's `include-exclude` feature is no longer unified in by gpui — declared explicitly on our `rust-embed` dep. All Task-5+ APIs verified present at the pinned rev (`list`, `ListState::{new,reset,splice,scroll_to,scroll_to_reveal_item,logical_scroll_top,scroll_by}`, `ListAlignment`, `ListOffset`, `scroll_to_item_strict`, `on_drag`/`on_drag_move`/`DragMoveEvent`, `cursor_col_resize`, `EmptyView`). 126 tests pass, build warning-free, fmt clean, 6s smoke launch OK.

### Task 2: Per-line syntax highlighting helper in preview.rs

**Files:**
- Modify: `src/preview.rs`

- [x] add `pub fn highlight_single_line(path: &Path, line: &str) -> Vec<HighlightedSpan>` — simplest correct impl: call the existing `syntax_lines_for_path(path, line)` on the single-line content and take the first line's spans (note: there is no config cache/table — configs are built on demand via `build_highlight_config`, and `syntax_lines_for_path` already has the plain-text fallback built in)
- [x] return a single plain (uncolored) span covering the whole line for unknown extensions, highlighter errors, or empty input
- [x] write tests: a Rust line (`let x = "s";`) yields multiple colored spans; a YAML line yields spans; unknown extension yields one plain span; empty line yields empty/one-empty-span; spans concatenate back to the exact input text
- [x] write tests for error path (extension-less path, binary-ish garbage line does not panic)
- [x] run tests - must pass before next task

**Task 2 notes:** ⚠️ test-environment discovery: the process-global default theme has an EMPTY syntax table (all captures resolve to one identical style), so `append_span` merges an entire highlighted line into a single span — "multiple colored spans" is unobservable through the public helper in tests, and the only theme writer needs an `App` context (global mutation would also race parallel tests). Deviation: `highlighted_lines` core was extracted as `highlighted_lines_with(content, spec, style_for)` with the capture→style resolver injected (production path unchanged, passes `theme::syntax_render_style`); the Rust/YAML capture-boundary tests inject a deterministic per-capture color resolver, and the public `highlight_single_line` tests assert reassembly + fallback structure. `#[allow(dead_code)] // consumed from Task 3` added as anticipated (helper unused until Task 3). 132 tests pass (126 baseline + 6 new), build warning-free, fmt clean.

### Task 3: Grep pipeline — uncap matches, carry col + syntax spans

**Files:**
- Modify: `src/picker.rs`

- [x] in `execute_grep_search`, set `max_matches_per_file` to 200 (matching `page_limit`)
- [x] extend `GrepMatchLine` with `col: u32` (from `GrepMatch.col` — a `usize`, cast `as u32`) and `syntax_spans: Vec<preview::HighlightedSpan>`
- [x] populate `syntax_spans` in the background search task via `preview::highlight_single_line` (per match line, after the grep call, still off the main thread)
- [x] update all existing `GrepMatchLine` construction/usage sites (preview overlay, goto) to compile with the new fields
- [x] the new fields are not read until Tasks 6/8 — if the `dead_code`/never-read lint breaks the warning-free gate, add a temporary `#[allow(dead_code)]` with a `// consumed in Tasks 6/8` note (removed when consumed)
- [x] write tests for any pure logic added/changed (e.g. a helper mapping `GrepMatch` → `GrepMatchLine` if extracted); update existing tests touched by the struct change
- [x] run tests - must pass before next task

**Task 3 notes:** Extracted pure helper `grep_match_line(path, line_number, col, line_content, match_byte_offsets) -> GrepMatchLine` (picker.rs, next to `execute_grep_search`) — the sole `GrepMatchLine` literal construction site now goes through it, and it computes `syntax_spans` via `preview::highlight_single_line` on the background search task. New fields carry field-level `#[allow(dead_code)] // consumed in Tasks 6/8`; `highlight_single_line`'s `#[allow(dead_code)]` removed (now genuinely consumed). No other construction sites existed (usage sites only read pre-existing fields). 136 tests pass (132 baseline + 4 new), build warning-free, fmt clean.

### Task 4: Row model and pure navigation helpers (src/rows.rs)

**Files:**
- Create: `src/rows.rs`
- Modify: `src/main.rs` (add `mod rows;`)

- [x] create `src/rows.rs` with `ResultRow` enum and `build_rows(&[FileItemSnapshot], &HashSet<PathBuf>, is_grep)` operating directly on the snapshot slice (`FileItemSnapshot` is a plain data struct — construct instances directly in tests)
- [x] implement `can_select`, `step_selectable` (direction stepping, no wrap), `anchor_selection` (re-anchor to same path+match after rebuild, else nearest selectable), `dedupe_opens` (first selected match per file, sorted order), `max_line_number`
- [x] write tests for `build_rows`: grep grouping (separator placement — none before first group), collapsed file omits match rows, files-mode flat rows, empty results
- [x] write tests for `can_select`/`step_selectable`: headers skipped when expanded, selectable when collapsed, separators always skipped, clamping at ends, all-collapsed navigation
- [x] write tests for `anchor_selection` (match survives rebuild; collapsed → header; removed file → nearest) and `dedupe_opens` (multi-match same file → one entry at first match; files-mode keys; empty)
- [x] run tests - must pass before next task

**Task 4 notes:** Signatures as implemented (all pure, no gpui): `build_rows(&[FileItemSnapshot], &HashSet<PathBuf>, is_grep) -> Vec<ResultRow>`; `can_select(&[ResultRow], ix, &HashSet<PathBuf>, &[FileItemSnapshot]) -> bool`; `step_selectable(rows, from, Direction::{Next,Prev}, collapsed, results) -> Option<usize>` (steps EXCLUSIVE of `from`, no wrap, None at ends); `anchor_selection(Option<(&Path, usize /*match ix*/)>, rows, collapsed, results) -> usize` — takes `collapsed` in addition to the planned args (needed for the collapsed→header branch and selectability checks); fallback order: exact (path, m) row → header if collapsed → file's LAST match row if m gone → first selectable → 0. "Removed file → nearest" is implemented as first selectable row (the old key alone carries no positional info). `dedupe_opens(&BTreeSet<SelectionKey>) -> Vec<SelectionKey>`; `max_line_number(&[FileItemSnapshot]) -> u64`. Extras: `SelectionKey` type alias lives in rows.rs (Task 8 should use `rows::SelectionKey`); pub `first_selectable(rows, collapsed, results) -> Option<usize>` added (anchor_selection needs it internally; Task 5's seed-to-first-selectable can reuse it directly). Module carries `#![allow(dead_code)] // wired into the picker in Task 5`. 159 tests pass (136 baseline + 23 new), build warning-free, fmt clean.

### Task 5: Swap results pane to variable-height list(ListState)

**Files:**
- Modify: `src/picker.rs`

- [x] add `rows: Vec<ResultRow>`, `collapsed: HashSet<PathBuf>`, and a `ListState` field to `FffPicker`; rebuild `rows` on search apply and view switch (collapsed cleared there too); remove the now-orphaned `list_scroll: UniformListScrollHandle` field and its `.track_scroll` call (`List` has no track_scroll; a never-read field fails the warning-free gate)
- [x] replace the results `uniform_list` with `list(state)` rendering by `ResultRow` dispatch — for this task, render Header rows with the existing file-row layout, Match rows with the existing content-match layout, and Separator as a plain divider (full Zed visuals come in Task 6); preview pane's `uniform_list` untouched. Use `cx.processor(...)` for the render closure (verified generic over the item arg in gpui) so `cx.listener` keeps working for row `on_click` — a raw closure only gets `&mut App` and cannot build listeners
- [x] rewire `selected` to index `rows`: add a row→(file, match) resolver helper and route EVERY `self.results.get(self.selected)` / `results.len()` bound-check site through it — `on_open_selected` (~picker.rs:1229), `on_toggle_selected` (~:1368), `on_toggle_select_all`, `on_select_row` (~:1344-1349), `load_preview` (~:1077) — a row index used as a results index toggles/opens/previews the wrong file
- [x] update `on_select_next/prev` to use `rows::step_selectable`, `on_select_row` to respect `can_select`, and replace `scroll_to_item*` calls with `scroll_to_reveal_item` / `scroll_to(ListOffset::default())` on new search
- [x] seed `selected` to the FIRST SELECTABLE row after every rebuild (`run_search` currently sets `selected = 0`, but in grep view row 0 is a Header, which is not selectable while expanded — use `step_selectable`/`can_select` scan from 0)
- [x] keep `load_preview`/`goto_for_path` compiling against row indices (full per-match preview behavior comes in Task 10)
- [x] write tests for newly extracted pure helpers (row→file/match resolution, first-selectable-row seeding); existing tests stay green
- [x] run tests - must pass before next task

**Task 5 notes:** Resolver is a pure fn in `rows.rs` (small deviation — task listed only picker.rs, but rows.rs is the established pure-helper home): `resolve_row(&[ResultRow], ix) -> Option<(usize /*file*/, Option<usize> /*match ix; None for headers*/)>`; picker wraps it as `FffPicker::selected_row_snapshot() -> Option<&FileItemSnapshot>` and routes `load_preview`, `on_open_selected`, `on_toggle_selected`, the render theme-change branch, and `selected_path` through it (`on_toggle_select_all` needed no change — it iterates `results` per-file, no row-index misuse). Seeding uses `rows::first_selectable(...).unwrap_or(0)` (Task 4's helper) instead of a manual scan. `rebuild_rows()` (build_rows + `ListState::reset`) runs on search apply and in `switch_mode`, both clearing `collapsed` first; `ListState::new(0, ListAlignment::Top, px(512.))`. rows.rs's module-level `#![allow(dead_code)]` removed; targeted allows remain on `anchor_selection` (Task 7), `SelectionKey` (Task 8), `dedupe_opens` (Task 9), `max_line_number` (Task 6). ⚠️ Accepted interim oddities (by design, later tasks fix): status bar "N shown" still counts FILES while grep rows are per-match (Task 11); grep Match rows keep the file-row pill column (count/✨/●) and dim filename prefix (Task 6 restyles); multiselect still per-file so ● marks every row of a marked file (Task 8); clicking an expanded Header is a no-op (chevrons come in Task 7); Enter on a collapsed Header opens the file (Task 9 makes it a no-op). 163 tests pass (159 baseline + 4 new in rows.rs), build warning-free, fmt clean, 8s smoke launch OK.

### Task 6: Zed-style row visuals

**Files:**
- Modify: `src/picker.rs`
- Modify: `src/layout.rs` (gutter width reuse if adjustment needed)

- [x] Header row (~30px): chevron ˅/˃ (`icon_muted`, rotates when collapsed), 16px file icon, filename `text_sm` `text_primary`, dir `text_xs` `text_secondary` truncated from the left; `selected_row` bg only when collapsed+cursor-selected; `hover_row` hover; 1px top-border separator handled as its own Separator row: 1px `theme.border` line with 8px vertical padding
- [x] Match row (28px): right-aligned line-number gutter sized from `rows::max_line_number` digit count (color `text_dim` @ 50% alpha), gap, then `syntax_spans` rendered with `overlay_match_ranges(line, byte_ranges)` so match chunks get `match_highlight_bg` + bold; single line, truncated; no count/✨ pills in grep mode
- [x] Files-mode rows unchanged (icon, fuzzy tint, dir, ✨ pill)
- [x] git edge bar: 3px full-height on Header AND Match rows (continuous per group), transparent when clean; none on separators
- [x] extract any nontrivial span-assembly logic into pure helpers and write tests (span merge output for a line with syntax spans + match ranges; plain-span fallback; truncation-safe char boundaries)
- [x] run tests - must pass before next task

**Task 6 notes:** `render_result_row` is now a dispatcher over `render_header_row` (30px, chevron glyphs ˅ U+02C5 / ˃ U+02C3, chevron has its own `("chevron", row_ix)` element id for Task 7's click handler), `render_grep_match_row` (28px), and `render_file_row` (Files-view layout unchanged); Separator renders as a 1px `theme.border` line inside 8px vertical padding. Pure helpers added in picker.rs: `match_row_spans(syntax_spans, line_content, byte_ranges, fallback_color, match_color, match_bg)` (trims leading/trailing whitespace out of the spans, shifts the match ranges, falls back to one plain span when `syntax_spans` is empty, then delegates to `preview::overlay_match_ranges`), `slice_spans(spans, start, end)` (char-boundary-clamped byte-window cut, used by the trim), and `truncate_start(text, max_chars)` (Zed `.truncate_start()` equivalent for the header dir; ellipsis counted in the budget). Gutter: `rows::max_line_number` is cached as a `max_match_line` field in `rebuild_rows()` (avoids rescanning all matches per rendered row) and fed to the existing `layout::gutter_width` — layout.rs needed no change. Deviations: (1) `MatchStyle` enum removed — its `Emphasized` variant became dead with the old content-match layout (never-constructed-variant warning), so `render_highlighted` is now tint-only; (2) the per-file ● mark is KEPT on grep match rows (right-aligned) as an interim multiselect indicator — dropping it would leave working tab-marking invisible until Task 8's checkboxes replace it; (3) `col`'s `#[allow(dead_code)]` remains with its note updated to Tasks 8/9 (`syntax_spans`'s allow removed — now consumed). 172 tests pass (163 baseline + 9 new: 6 `match_row_spans_*`, 1 `slice_spans_*`, 2 `truncate_start_*`), build warning-free, fmt clean.

### Task 7: Fold interactions

**Files:**
- Modify: `src/picker.rs`
- Modify: `src/main.rs` (keybindings)

- [x] add `ToggleFold` / `ToggleFoldAll` actions to `actions!(fff_picker, ...)`; export them and add them to the `use picker::{...}` import in `src/main.rs` (~:30-34); bind `alt-z` / `alt-shift-z` in `bind_base_keys` (main.rs — note the function is `bind_base_keys`, NOT `bind_keys`)
- [x] `alt-z`: toggle the current row's group (works from header or any of its match rows); collapse re-anchors cursor to the header, expand to the first match; `splice` the changed row range into `ListState` to preserve scroll
- [x] `alt-shift-z`: toggle-all (any collapsed → expand all, else collapse all), cursor re-anchored via `rows::anchor_selection`
- [x] chevron click toggles that group; alt-click on chevron toggles all (stop propagation so the row click doesn't also fire)
- [x] clear `collapsed` on new search and on view/mode switch
- [x] write tests for pure fold logic: toggle-all semantics, re-anchor targets on collapse/expand, splice range computation if extracted
- [x] run tests - must pass before next task

**Task 7 notes:** Pure helpers in rows.rs: `group_match_range(rows, file) -> Option<Range<usize>>` (the match-row range right after the group's header — empty when collapsed, `None` when the header is absent, i.e. Files view; the splice contract is old-rows range + new-rows count, same start index) and `toggle_all_collapsed(&collapsed, &results) -> HashSet<PathBuf>` (any collapsed → empty set, else all paths); `anchor_selection`'s `#[allow(dead_code)]` removed (now consumed). Picker methods: `toggle_group_fold(file, cx)` (splices via `group_match_range` old/new; falls back to `reset` only if the header vanished) and `toggle_all_folds(cx)` — toggle-ALL takes the plan's accepted simpler path of a full `rebuild_rows()`/`reset` instead of many splices. Both re-anchor through `selected_fold_key()` (cursor's (path, match); header rows key as match 0 so expand lands on the first match) + `reanchor_after_fold()` (`anchor_selection` + `scroll_to_reveal_item`); chevron toggles of OTHER groups keep the cursor on its own shifted row via the same path. Deviation (minor): `reanchor_after_fold` deliberately does NOT call `load_preview` — the previewed FILE can never change across a fold rebuild (the anchor stays within the old key's file, which is still present), so reloading would be pure churn; Task 10 owns per-match re-centering. Fold entry points no-op cleanly in Files view (`is_grep_view` guard; `group_match_range` returns None there anyway). Chevron `on_click` calls `cx.stop_propagation()` before dispatch and branches on `event.modifiers().alt` (verified on the pinned rev: bubble-phase click listeners fire child-first). 182 tests pass (172 baseline + 10 new: 4 `group_match_range_*`, 3 `toggle_all_*`, 3 `fold_*` re-anchor scenarios), build warning-free, fmt clean.

### Task 8: Multiselect rework — SelectionKey, mode, checkboxes

**Files:**
- Modify: `src/picker.rs`
- Modify: `src/main.rs` (keybinding for cmd-shift-s)

- [x] replace `selected_paths: Arc<BTreeSet<PathBuf>>` with `Arc<BTreeSet<SelectionKey>>` (`SelectionKey = (PathBuf, Option<(u64, u32)>)`); grep Match rows key `Some((line, col))`, Files rows `None`; prune to visible results after each search
- [x] add `multi_select_mode: bool` + `ToggleMultiSelectMode` action bound to `cmd-shift-s` in `bind_base_keys` (main.rs; also add the action to the `use picker::{...}` import ~:30-34); toggling OFF clears the selection; clear-on-query-change keeps working
- [x] `tab` (`on_toggle_selected`): enter mode, toggle current row's key, advance to next selectable row; `ctrl-a`: enter mode, toggle-all visible match keys; headers/separators never selectable
- [x] cmd-click on a row toggles its key (enters mode); plain click keeps cursor-move behavior
- [x] render 16px checkbox slot on Match/Files rows only while `multi_select_mode` (14px rounded square, `theme.border` outline; checked = `match_highlight` fill + check glyph); remove the ● dot badge
- [x] write tests: SelectionKey ordering (BTreeSet gives path-then-line order), prune logic, toggle-all enter/exit semantics, mode-off clears
- [x] run tests - must pass before next task

**Task 8 notes:** Pure helpers live in rows.rs (the established pure-helper home): `selection_key_for_row(rows, ix, results, is_grep) -> Option<SelectionKey>` (grep Match → `(path, Some((line, col)))`, Files row → `(path, None)`; Header — collapsed or not — Separator, and out-of-range carry NO key, so toggling there is a no-op), `key_survives(key, results) -> bool` (prune predicate: grep keys need the exact (path, line, col) triple, files keys the path), `toggle_all_selection(selection, results, is_grep)` (any selected → empty, else all per-match triples / per-file keys), and `toggle_multi_select_mode(mode, selection) -> (bool, BTreeSet)` (off clears, on keeps); `SelectionKey`'s `#[allow(dead_code)]` removed, and `GrepMatchLine.col`'s allow removed too (now consumed for key identity; its byte-offset-identity-only comment stays). Picker: `selected_paths` replaced by `multi_select_mode: bool` + `selection: Arc<BTreeSet<rows::SelectionKey>>`; search-apply prunes via `key_survives`; tab enters mode, toggles via new `toggle_key_at(row_ix)`, advances through `rows::step_selectable` + `scroll_to_reveal_item` + preview reload; ctrl-a enters mode + `toggle_all_selection`; cmd-shift-s handler delegates to the pure toggle. Cmd-click: all three row types route clicks through `on_row_click`, which checks `event.modifiers().platform` — cmd-click toggles the key and enters mode WITHOUT moving the cursor; on keyless rows (headers) it is a full no-op (deliberate: also avoids the click-selected-header-opens-file path); plain clicks keep `on_select_row`; the chevron's own handler still stops propagation first. Checkbox: `render_checkbox(checked, theme)` free fn — 16px slot, 14px `.rounded(px(3.0))` square outlined `theme.border`; checked = `match_highlight` fill + border and a ✓ glyph in `theme.bg`; it leads the row content of grep Match and Files rows only while the mode is on; the ● dot badge is gone everywhere (grep interim + files mode). Interims/decisions: (1) `on_open_selected` opens the FILE part of each key once (consecutive `dedup()` over the path-ordered BTreeSet) at `goto_for_path`'s first-match position — per-match goto + `rows::dedupe_opens` are Task 9; (2) query change and view switch clear the selection but leave the mode flag on (the plan mandates clear-on-query-change only; cmd-shift-s is the explicit way out of the mode). Tests: 12 new in rows.rs (`selection_key_grep_match_rows_key_line_and_col`, `selection_key_files_rows_key_path_only`, `selection_key_header_separator_and_out_of_range_have_none`, `selection_keys_order_by_path_then_line_then_col`, `selection_keys_same_line_different_cols_are_distinct`, `key_survives_grep_key_requires_exact_line_and_col`, `key_survives_files_key_requires_path_only`, `prune_retains_only_surviving_keys`, `toggle_all_selection_grep_selects_every_match_triple`, `toggle_all_selection_files_selects_per_file_keys`, `toggle_all_selection_any_selected_clears_everything`, `toggle_mode_off_clears_selection_on_keeps_it`); the rows test fixture gained `snap_with_cols` (per-match cols). 194 tests pass (182 baseline + 12), build warning-free, fmt clean, 6s smoke launch OK.

### Task 9: Open behavior — once-per-file dedupe and per-match goto

**Files:**
- Modify: `src/picker.rs`

- [x] `on_open_selected`: when selection non-empty → `rows::dedupe_opens` → open each file once at its first selected match's line (both IPC responder batch and daemon editor paths), `track_open` per file. **Editor column caveat:** the SelectionKey `col` is a 0-based BYTE offset used for match identity only — the editor goto column must come from `goto_for_path`'s existing 1-based char-column computation (picker.rs:1180-1189) parameterized by the resolved match, never the raw key col
- [x] with empty selection: current Match row opens at ITS OWN match's line/char-col (replace the `grep_matches.first()` assumption in `goto_for_path` with the selected match); Files row unchanged; collapsed Header row = no-op
- [x] write tests for the goto-resolution helper (match row → its own line/col; files row → None; header → no open) and dedupe integration (same file two matches selected → single open at first)
- [x] write tests for error/edge cases (selection pointing at a path no longer in results)
- [x] run tests - must pass before next task

**Task 9 notes:** `goto_for_path` is REMOVED (on_open_selected was its only caller), replaced by four pure free fns in picker.rs (the plan's "clean shape"): `match_goto(&GrepMatchLine) -> (usize /*line*/, usize /*1-based CHAR col*/)` (existing char-column computation, now parameterized by the match; col 1 fallback for empty `byte_ranges` or a byte offset off a char boundary); `cursor_open(&[ResultRow], selected, &[FileItemSnapshot]) -> Option<(PathBuf, Option<(usize, usize)>)>` (Match row → its OWN match's goto; Files-view row → `(path, None)`; Header/Separator/out-of-range → `None` = no-op — Enter on a collapsed header now does nothing, closing Task 5's interim); `goto_for_key(&rows::SelectionKey, &[FileItemSnapshot]) -> Option<(PathBuf, Option<(usize, usize)>)>`; and `opens_for_selection(&BTreeSet<SelectionKey>, &[FileItemSnapshot]) -> Vec<(PathBuf, Option<(usize, usize)>)>` = `rows::dedupe_opens` → `goto_for_key` per key. Defensive picks (the plan offered a choice — BOTH implemented, both tested): key's exact (line, col) match gone but path still listed → open at the key's line, column 1; path gone from results entirely → skip that entry (grep AND files keys). If everything resolves to nothing (all paths vanished, or cursor on a header), `on_open_selected` returns early as a FULL no-op — no IPC response sent, window stays open. `track_open` gets the once-per-file path list; `PickEntry`/`PickResponse` shape unchanged. `dedupe_opens`' `#[allow(dead_code)]` removed (now consumed); stale `goto_for_path` comment references updated (picker.rs `GrepMatchLine.col`, rows.rs `SelectionKey`). 14 new tests (3 `match_goto_*`, 4 `cursor_open_*`, 4 `goto_for_key_*`, 3 `opens_for_selection_*`). 208 tests pass (194 baseline + 14), build warning-free, fmt clean.

### Task 10: Preview integration for per-match rows

**Files:**
- Modify: `src/picker.rs`
- Modify: `src/preview.rs` (only if the center-line helper needs a tweak)

- [x] `load_preview`: cursor on a Match row → preview that file centered on THAT match's line, with ALL the file's matches overlaid (not just the first); cursor on a collapsed Header → centered on the file's first match
- [x] moving between match rows of the same file re-centers without reloading highlight work where the existing structure allows (keep it simple: reuse current reload path if caching is nontrivial)
- [x] write tests for the pure mapping (row index → (path, center line, overlay ranges)) extracted as a helper
- [x] run tests - must pass before next task

**Task 10 notes:** Pure helpers are picker.rs free fns (next to Task 9's `cursor_open` family): `preview_target(&[ResultRow], selected, &[FileItemSnapshot]) -> Option<(PathBuf, Option<usize> /*center line*/, Vec<GrepMatchLine> /*overlay*/)>` (center is `usize` not the sketched `u64` — fits `highlight_file_window(Option<usize>)`; Match row → its own line, Header → file's first match via `.min()`, Files row → `None` center + empty overlay, Separator/out-of-range → `None`) and `recenter_scroll_row(loaded_path, loading, start_line, loaded_lines, path, center_line) -> Option<usize>`. The overlay loop already covered all `grep_matches` (pre-existing), so the load-path behavioral change is only the CENTER following the cursor row. Same-file re-center choice: the cheap path IS implemented, but guarded to windows that hold the WHOLE file (`start_line == 1 && loaded_lines < preview::MAX_PREVIEW_LINES` — a file at exactly the cap conservatively reloads; larger files always reload so the window re-centers with full context) — there the skip is exactly correct since the window and overlay cannot change within a file. New `preview_path: Option<PathBuf>` field records which file (under which results/theme) the loaded window belongs to; it is invalidated at every point the loaded spans could go stale: search apply (`this.preview_path = None` — same file can get different overlay ranges from a new query), `switch_mode`, theme-change re-highlight in `render` (span colors are baked in), and `load_preview`'s no-target branch; a `!preview_loading` guard keeps the cheap path off while a reload is in flight. Fold-reanchor choice: KEPT Task 7's skip-preview — collapsing from a match row keeps that match's center until the next cursor move (same file, same overlay; no reload/scroll churn on fold); `reanchor_after_fold`'s comment documents the accepted nuance. preview.rs needed NO change (`MAX_PREVIEW_LINES` was already pub). 7 new tests (4 `preview_target_*`: own-line center + all-matches overlay, collapsed-header first-match center, files-row no-center/no-overlay, separator/out-of-range None; 3 `recenter_*`: zero-based scroll row incl. window edges, other-file/no-file/in-flight reloads, truncated-window/outside-line/no-center reloads). 215 tests pass (208 baseline + 7), build warning-free, fmt clean.

### Task 11: Status bar counts and key hints

**Files:**
- Modify: `src/picker.rs`

- [x] left text (grep view): `"N matches in M files  K selected  X indexed"` where N = total visible matches computed FRESH as the sum of `grep_matches.len()` across results (⚠️ do NOT reuse `self.total_matched` — `execute_grep_search` sets it to the deduped FILE count, picker.rs:550), M = file count, K = selection size; Files view keeps its current counts; grep submode hint (`mode: plain ⇧tab mode`) retained
- [x] right hints: `"↑↓ nav  ⇥ mark  ⌘⇧S multi  ⌥Z fold  ⏎ open  esc quit"` (grep) / without fold hint in Files view
- [x] extract status-text building into a pure helper and write tests (grep vs files, zero/singular/plural counts, selected count, indexing state, status_message override; include a case where the match total differs from the file count to prove `total_matched` isn't being reused)
- [x] run tests - must pass before next task

**Task 11 notes:** Two pure free fns in picker.rs (next to the Task 9/10 helper family): `status_left_text(view, grep_mode, status_message: Option<&str>, scan_done, indexed_count, total_files, total_matched, selected_count, results: &[FileItemSnapshot]) -> String` (carries `#[allow(clippy::too_many_arguments)]` — plain state snapshot by design) and `status_right_hints(view) -> &'static str`, plus a tiny `plural(n, one, many)`. Grep left counts are summed FRESH from `grep_matches.len()` inside the helper; `total_matched` is still a parameter but only the Files-view branch reads it (a test passes a bogus 99 in grep view with 5 matches / 2 files to prove it). Deviations/decisions: (1) counts are pluralized — `"1 match in 1 file"` singular forms, `"0 matches in 0 files"` at zero (the plan's literal format string read "matches/files" but its own zero/singular/plural test bullet implies grammatical forms; Zed parity); (2) the mode-switch hint is KEPT in BOTH views per the clarified task — grep right hints are `↑↓ nav  ⇥ mark  ⌘⇧S multi  ⌥Z fold  cmd-f files  ⏎ open  esc quit` (mode hint sits before ⏎ open, same slot as today), Files drops only the fold segment; (3) status_message override and indexing-in-progress branches preserved verbatim, including today's behavior that the grep submode hint is appended AFTER a status_message / indexing text too, and that an all-empty base in grep view renders the hint with no leading `•`; (4) `indexed` still falls back to `indexed_count` when `total_files == 0`. The render block shrank to one `status_left_text(...)` call and the right-hand child is `status_right_hints(self.view)` (the old `mode_hint` local is gone). 10 new tests (`status_left_grep_sums_visible_matches_not_total_matched`, `status_left_grep_singular_match_and_file`, `status_left_grep_zero_matches`, `status_left_grep_selected_count_and_submode`, `status_left_files_keeps_flat_counts_no_submode_hint`, `status_left_status_message_wins`, `status_left_indexing_in_progress`, `status_left_indexed_falls_back_to_indexed_count`, `status_right_hints_grep_includes_fold_and_mode_switch`, `status_right_hints_files_omits_fold_keeps_mode_switch`). 225 tests pass (215 baseline + 10), build warning-free, fmt clean.

### Task 12: Verify acceptance criteria

- [x] verify all requirements from Overview are implemented (grouped rows, all four features, per-match multiselect, once-per-file open, gpui git dep, custom fold keys, cmd-shift-s mode, Zed-parity header navigation, pills dropped in grep)
- [x] verify edge cases: empty results, single file, all collapsed, selection across collapsed groups, files-mode regression pass
- [x] run full test suite: `cargo test`
- [x] `cargo build` warning-free; `cargo fmt --check` clean
- [x] verify no `unwrap` on user-input paths in new code; no behavior change to Files-mode open/IPC contract

**Task 12 notes:** Verified by code inspection + tests; no defects found — nothing to fix. Requirement→evidence: (1) grouped rows / separators — `rows::build_rows` (rows.rs:42), tests `build_rows_*`; Separator render 1px border + 8px v-pad (picker.rs `render_result_row`); (2) collapsible groups — `collapsed` set + `toggle_group_fold`/`toggle_all_folds` (picker.rs) with splice/reset, chevron + alt-click chevron handler in `render_header_row`, tests `group_match_range_*`/`toggle_all_*`/`fold_*`; (3) syntax-highlighted rows — `preview::highlight_single_line` → `grep_match_line` (background task) → `match_row_spans` overlay (match bg + bold), tests `grep_match_line_*`/`match_row_spans_*`; (4) checkbox multiselect — `render_checkbox` gated on `multi_select_mode` in grep+files rows, ● dot gone, per-match `SelectionKey` via `rows::selection_key_for_row`, tests `selection_key_*`/`toggle_all_selection_*`/`toggle_mode_*`; (5) tab toggle+advance `on_toggle_selected`, ctrl-a `on_toggle_select_all`, cmd-click `on_row_click` (platform modifier); (6) once-per-file open — `rows::dedupe_opens` → `opens_for_selection` → `on_open_selected`, char-col via `match_goto`, tests `dedupe_opens_*`/`opens_for_selection_*`; (7) gpui git dep — Cargo.toml `git = ".../zed"`, lock pins `bb87c47c92d0`; (8) fold keys alt-z/alt-shift-z + cmd-shift-s bound in `bind_base_keys` (main.rs:282-284), actions in `actions!` + main.rs import; (9) Zed-parity header nav — `can_select`/`step_selectable` (headers only when collapsed, separators never, no wrap), Enter on collapsed header no-op via `cursor_open` returning None, tests `can_select_*`/`step_selectable_*`/`cursor_open_header_row_is_a_no_op`; (10) pills dropped in grep — `render_grep_match_row` has no count/✨/●; Files rows keep ✨. Edge cases: empty results, all-collapsed, files-mode all had coverage; ADDED 2 tests for the gaps — `rows::build_rows_single_file_has_no_separators` (single file, expanded + collapsed) and picker `selection_survives_collapsing_marked_group_and_still_opens_once_per_file` (marks keys from expanded rows via `selection_key_for_row`, collapses the group, proves `key_survives` + `opens_for_selection` are row-independent; code trace: fold handlers never touch `selection`, prune runs only on search apply). No non-test `unwrap`/`expect` in rows.rs/new picker helpers (fallbacks: `match_goto` col 1, `get`/`?` everywhere); preview.rs `expect("at least one line exists")` is a seeded-vec invariant, not user input. Files-mode open/IPC contract unchanged: `PickEntry{path, line, column}` shape intact, files keys resolve to `(path, None)` → line/column None, empty-response Drop path untouched. `git diff` scope clean: this plan's files + text_field.rs (Task 1 `ShapedLine::paint` drift) + config.rs/theme.rs/README.md (previous uncommitted telescope plan — left alone). Gates: 227 tests pass (225 baseline + 2 new), `cargo build` 0 warnings, `cargo fmt --check` clean, 6s smoke launch (`--open . --grep`) alive with no panic output.

### Task 13: [Final] Update documentation

- [x] update README.md: grouped grep results + fold keybinds (`alt-z`, `alt-shift-z`, chevron/alt-click), multiselect mode (`cmd-shift-s`, tab toggle+advance, cmd-click, checkboxes, once-per-file open), gpui git-dependency build note (first build fetches the zed repo), keybinding table refresh
- [x] update CLAUDE.md if present and new patterns discovered (no CLAUDE.md exists today — skip unless one was added)
- [x] move this plan to `docs/plans/completed/` (create dir if needed)
- [x] move the linked design file too: extract with `grep -E '^\*\*Design:\*\*' docs/plans/2026-07-22-zed-grouped-results.md | sed 's/^\*\*Design:\*\* *//'`; `test -f` then `mv` to `docs/plans/completed/` — warn and continue if missing

**Task 13 notes:** README.md changes: (1) Features list gained two bullets — grouped per-match grep results under collapsible file headers with syntax-highlighted match lines, and checkbox multi-select mode with once-per-file open; (2) Build from source gained the gpui git-dependency note (git dep on zed-industries/zed, rev pinned in Cargo.lock, first build fetches the full Zed repo — large download); (3) Running gained a new "Keys" subsection documenting navigation (`↑`/`↓`, `enter`, `esc`, `cmd-f`/`cmd-g`, `shift-tab` grep-mode cycle, `ctrl-u`/`ctrl-d`, `ctrl-up`), folding (`alt-z`, `alt-shift-z`, chevron click / alt-click), and multi-select (`cmd-shift-s` mode toggle with off-clears, `tab` mark+advance, `cmd`-click, `ctrl-a`, per-match marks in grep, once-per-file open at first marked match). No pre-existing key descriptions were stale (the README had none); the Zed-integration line "with grep, the editor jumps to the matched line" remains accurate under per-match goto. Screenshot left untouched (manual retake pending, see Post-Completion). No CLAUDE.md exists — skipped. Plan + design doc moved to `docs/plans/completed/`. Gates re-verified after docs-only changes: 227 tests pass, `cargo build` 0 warnings, `cargo fmt --check` clean.

## Post-Completion

*Items requiring manual intervention or external systems - no checkboxes, informational only*

**Manual verification:**
- Visual pass in light + dark themes: header/separator spacing vs Zed, gutter alignment with 1-4 digit line numbers, syntax colors sanity, match tint, checkbox styling, continuous git edge bars per group
- Interaction feel: tab-through marking, fold/unfold + scroll preservation, alt-click chevron, cmd-click toggling, Enter with mixed selections, preview re-centering while arrowing through a file's matches
- Perf check on a large repo (200-match pages, per-line highlighting cost while typing)
- First `cargo build` after the gpui switch on a clean machine (zed repo fetch size/time)
- README screenshot retake (also still pending from the previous restyle)
