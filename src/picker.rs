use std::collections::{BTreeSet, HashSet};
use std::io::Write as _;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, AtomicU32, Ordering},
};
use std::time::{Duration, Instant};

use fff_query_parser::{FFFQuery, FileSearchConfig, FuzzyQuery, QueryParser};
use fff_search::{
    ContentCacheBudget, FFFMode, FilePickerOptions, FuzzySearchOptions, GrepMode,
    GrepSearchOptions, PaginationArgs, SharedFilePicker, SharedFrecency, SharedQueryTracker,
    file_picker::FilePicker, frecency::FrecencyTracker, git::format_git_status_opt,
    query_tracker::QueryTracker,
};
use gpui::prelude::*;
use gpui::*;
use tracing::{debug, error, info, trace, warn};

use crate::editor;
use crate::history;
use crate::layout;
use crate::log;
use crate::path_shortening::PathShortenStrategy;
use crate::preview::{self, HighlightedLine};
use crate::rows::{self, ResultRow};
use crate::service::{ClientStream, PickEntry, PickResponse};
use crate::text_field::TextField;
use crate::theme::{self, AppTheme, FileIconPath};
use crate::ui;

pub type ResponderArc = Arc<Mutex<Option<ClientStream>>>;

// Keep live grep snappy by returning partial results quickly; newer keystrokes
// will preempt any still-running search.
const GREP_TIME_BUDGET_MS: u64 = 150;

// Write a PickResponse to the client and shut the stream so the client unblocks.
// `responder` is consumed; passing None or a stream that's already been taken is a no-op.
#[cfg(unix)]
fn send_pick_response(responder: Option<ResponderArc>, entries: &[PickEntry]) {
    let Some(arc) = responder else { return };
    let Ok(mut guard) = arc.lock() else { return };
    let Some(mut stream) = guard.take() else {
        return;
    };
    let payload = match serde_json::to_string(&PickResponse {
        paths: entries.to_vec(),
    }) {
        Ok(s) => s,
        Err(err) => {
            warn!(error = %err, "failed to serialize pick response");
            return;
        }
    };
    if let Err(err) = writeln!(stream, "{payload}") {
        warn!(error = %err, "failed to write pick response to client");
    }
    let _ = stream.shutdown(std::net::Shutdown::Both);
}

#[cfg(not(unix))]
fn send_pick_response(_responder: Option<ResponderArc>, _entries: &[PickEntry]) {}

actions!(
    fff_picker,
    [
        Quit,
        OpenSelected,
        SelectNext,
        SelectPrev,
        ToggleSelected,
        ToggleSelectAll,
        ToggleMultiSelectMode,
        ToggleFold,
        ToggleFoldAll,
        ShiftTab,
        CycleGrepMode,
        HistoryPrev,
        HistoryNext,
        PreviewScrollUp,
        PreviewScrollDown,
        SwitchFiles,
        SwitchGrep,
    ]
);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SearchView {
    Files,
    Grep,
}

// Live cursor into the query history, present only while shift-up/shift-down
// navigation is in progress.
#[derive(Clone, Debug)]
struct HistoryNav {
    // Offset handed to `query_tracker` (0 = most recent).
    offset: usize,
    // What the user was typing before entering history, restored by stepping
    // past the newest entry.
    draft: String,
    // The last text we wrote into the field. The text-field observer compares
    // against this to tell our own writes from real user edits — without it,
    // every history step would look like typing and drop the cursor.
    injected: String,
}

#[derive(Clone, Debug, Default)]
pub struct PickerSharedState {
    pub shared_picker: SharedFilePicker,
    pub shared_frecency: SharedFrecency,
    pub shared_query_tracker: SharedQueryTracker,
}

// Return a sensible worker count for fff searches on the current machine.
fn search_threads() -> usize {
    std::thread::available_parallelism()
        .map(usize::from)
        .unwrap_or(4)
}

// A single grep-matched line within a file, with byte ranges for that line.
#[derive(Clone)]
pub struct GrepMatchLine {
    pub line_number: u64,
    pub line_content: String,
    pub byte_ranges: Vec<(u32, u32)>,
    // 0-based BYTE offset of the first match start within the line. Match
    // identity only (multiselect `SelectionKey`s) — editor goto columns come
    // from `match_goto`'s 1-based char-column computation.
    pub col: u32,
    // Per-line tree-sitter spans, computed on the background search task.
    // Empty means render the line as plain text.
    pub syntax_spans: Vec<preview::HighlightedSpan>,
}

// A file path snapshot captured from a FileItem for render and preview work.
#[derive(Clone)]
pub struct FileItemSnapshot {
    pub file_name: String,
    pub dir: String,
    pub absolute_path: PathBuf,
    pub git_status: Option<String>,
    pub frecency_score: i16,
    pub match_ranges: Vec<Range<usize>>,
    pub grep_matches: Vec<GrepMatchLine>,
}

// Marker payload for the results/preview divider drag. gpui tracks it in
// `cx.active_drag`; `on_drag_move::<DividerDrag>` on the body row then receives
// every mouse move window-wide until mouse-up clears the drag.
struct DividerDrag;

// Resolve the effective results/preview split for the body row. Precedence:
// session drag value -> `picker_pane_width` config override -> 50/50 default;
// every path goes through `layout::split`'s pane-minimum clamps
// (results >=280px, preview >=128px).
fn effective_split(modal_w: f32, session: Option<f32>, config: Option<f32>) -> layout::Split {
    layout::split(modal_w, session.or(config))
}

// Process-wide (daemon-lifetime) storage for the divider's results-pane width.
// The `FffPicker` entity is recreated every time the window opens, so its
// `session_results_width` field alone would lose the drag on every close. This
// static outlives the window entity, so a drag survives window close/reopen for
// as long as the daemon runs, and resets on restart — the design's "session
// only" divider persistence. In-memory only; never written to disk.
//
// Stored as f32 bits in an AtomicU32 with a NaN sentinel meaning "unset" (fall
// back to the config override, then the 50/50 default). The encode/decode logic
// is factored into pure functions so it can be unit-tested without touching the
// global.
const SESSION_WIDTH_UNSET: u32 = 0x7FC0_0000; // canonical quiet NaN
static SESSION_RESULTS_WIDTH: AtomicU32 = AtomicU32::new(SESSION_WIDTH_UNSET);

// Encode an optional divider width for atomic storage. Non-finite widths (and
// None) collapse to the unset sentinel.
fn encode_session_width(width: Option<f32>) -> u32 {
    match width {
        Some(w) if w.is_finite() => w.to_bits(),
        _ => SESSION_WIDTH_UNSET,
    }
}

// Decode a stored divider width; the NaN sentinel (or any non-finite value)
// decodes to None.
fn decode_session_width(bits: u32) -> Option<f32> {
    let width = f32::from_bits(bits);
    width.is_finite().then_some(width)
}

// Seed a fresh picker's divider position from the process-wide store.
fn load_session_results_width() -> Option<f32> {
    decode_session_width(SESSION_RESULTS_WIDTH.load(Ordering::Relaxed))
}

// Persist the divider position so it survives the window entity's destruction.
fn store_session_results_width(width: Option<f32>) {
    SESSION_RESULTS_WIDTH.store(encode_session_width(width), Ordering::Relaxed);
}

pub struct FffPicker {
    shared_picker: SharedFilePicker,
    shared_frecency: SharedFrecency,
    shared_query_tracker: SharedQueryTracker,
    view: SearchView,
    excluded_dirs: Vec<PathBuf>,
    print_stdout: bool,
    grep_mode: GrepMode,
    query: String,
    // Set while walking the query history; cleared by any user edit and by a
    // view switch (Files and Grep keep separate history stacks).
    history_nav: Option<HistoryNav>,
    results: Arc<Vec<FileItemSnapshot>>,
    total_files: usize,
    total_matched: usize,
    indexed_count: usize,
    // Cursor position as an index into `rows` (NOT `results`) — resolve it
    // through `rows::resolve_row` / `selected_row_snapshot` before touching
    // per-file state.
    selected: usize,
    // Multiselect: explicit checkbox mode plus the per-key selection set. Keys
    // are per-match `(path, Some((line, col)))` in grep view and per-file
    // `(path, None)` in Files view (`rows::SelectionKey`). The selection is
    // cleared on query change, view switch, and mode-off; after each search it
    // is pruned to the keys still visible in the results.
    multi_select_mode: bool,
    selection: Arc<BTreeSet<rows::SelectionKey>>,
    // Derived row projection over `results`: grep view gets
    // Header/Match/Separator groups, Files view flat per-file Match rows.
    // Rebuilt on every search apply and view switch.
    rows: Vec<ResultRow>,
    // Collapsed grep file groups feeding `build_rows`. Cleared on search
    // apply and view switch; fold interactions land in Task 7.
    collapsed: HashSet<PathBuf>,
    // Widest grep line number across `results`, cached by `rebuild_rows` so
    // every match row shares one gutter width without rescanning all matches
    // per rendered row.
    max_match_line: u64,
    scan_done: bool,
    search_epoch: u64,
    search_in_flight: bool,
    search_abort: Option<Arc<AtomicBool>>,
    preview_epoch: u64,
    preview_loading: bool,
    preview_loading_visible: bool,
    preview_scroll_row: usize,
    preview_start_line: usize,
    // The file the current `preview_lines` were highlighted for, valid only
    // while that window still reflects CURRENT results and theme output. Lets
    // same-file cursor moves re-center without reloading
    // (`recenter_scroll_row`); reset to None whenever the loaded window may be
    // stale — search reapply (overlay ranges can change for the same file),
    // view switch, and theme change (span colors are baked in).
    preview_path: Option<PathBuf>,
    // Session-only divider position (results-pane width in px), set by drag
    // and double-click reset. `None` falls back to the `picker_pane_width`
    // config override, then the 50/50 default (see `effective_split`).
    // Seeded from and written back to the process-wide `SESSION_RESULTS_WIDTH`
    // store so it survives window close/reopen for the daemon's lifetime.
    session_results_width: Option<f32>,
    theme_version: u64,
    focus_handle: FocusHandle,
    // Variable-height results list. Cheap to clone for render (Rc inside);
    // `reset` on rebuild, `scroll_to_reveal_item` on cursor moves.
    results_list: ListState,
    preview_scroll: UniformListScrollHandle,
    preview_lines: Arc<Vec<HighlightedLine>>,
    status_message: Option<String>,
    text_field: Entity<TextField>,
    editor: String,
    dismiss_on_blur: Option<Subscription>,
    dismiss_on_window_blur: Option<Subscription>,
    responder: Option<ResponderArc>,
}

// Find byte ranges where query characters appear in order.
fn find_match_ranges(query: &str, text: &str) -> Vec<Range<usize>> {
    let query = query.trim();
    if query.is_empty() {
        return vec![];
    }

    let fuzzy_chars: Vec<char> = query.to_lowercase().chars().collect();
    let mut qi = 0;
    let mut ranges: Vec<Range<usize>> = Vec::new();
    let mut run_start: Option<usize> = None;
    let mut run_end: usize = 0;

    for (byte_idx, orig_ch) in text.char_indices() {
        if qi >= fuzzy_chars.len() {
            break;
        }
        let ch_lower = orig_ch.to_lowercase().next().unwrap_or(orig_ch);
        if ch_lower == fuzzy_chars[qi] {
            if run_start.is_none() {
                run_start = Some(byte_idx);
            }
            run_end = byte_idx + orig_ch.len_utf8();
            qi += 1;
        } else if let Some(start) = run_start.take() {
            ranges.push(start..run_end);
        }
    }
    if let Some(start) = run_start {
        ranges.push(start..run_end);
    }

    if qi >= fuzzy_chars.len() {
        ranges
    } else {
        vec![]
    }
}

// Clamp `[start, end)` to lie on char boundaries of `text` (start shrinks left,
// end grows right) and into `text`'s length. Returns `None` for an empty or
// inverted result. Mirrors the identically-named helper in `preview.rs`.
fn clamp_range_to_char_boundaries(text: &str, start: usize, end: usize) -> Option<Range<usize>> {
    let mut start = start.min(text.len());
    let mut end = end.min(text.len());

    while start > 0 && !text.is_char_boundary(start) {
        start -= 1;
    }
    while end < text.len() && !text.is_char_boundary(end) {
        end += 1;
    }

    (start < end).then_some(start..end)
}

// Split `text` into an ordered list of `(chunk, is_match)` segments given match
// byte ranges. Ranges are clamped to char boundaries, sorted, and any range that
// starts before the last consumed offset (out-of-order overlap / containment) is
// dropped; the gaps between and around matches become `is_match == false`
// chunks. With no usable ranges the whole text is returned as a single
// non-matched chunk (empty text yields an empty list). Pure — unit-tested below.
fn segment_matches<'a>(text: &'a str, ranges: &[Range<usize>]) -> Vec<(&'a str, bool)> {
    let mut clamped: Vec<Range<usize>> = ranges
        .iter()
        .filter_map(|range| clamp_range_to_char_boundaries(text, range.start, range.end))
        .collect();
    clamped.sort_by_key(|range| (range.start, range.end));

    let mut segments: Vec<(&str, bool)> = Vec::new();
    let mut last = 0;
    for range in clamped {
        if range.start < last {
            continue;
        }
        if range.start > last {
            segments.push((&text[last..range.start], false));
        }
        segments.push((&text[range.clone()], true));
        last = range.end;
    }
    if last < text.len() {
        segments.push((&text[last..], false));
    }
    segments
}

// Render text with fuzzy-matched character ranges tinted in the accent color
// (Files-view filename rows, Zed-style; no background). Grep match rows
// emphasize matches through `match_row_spans` instead.
fn render_highlighted(text: &str, ranges: &[Range<usize>], theme: &AppTheme) -> Div {
    let segments = segment_matches(text, ranges);
    if !segments.iter().any(|(_, is_match)| *is_match) {
        return div().flex().items_center().child(text.to_string());
    }

    let parts: Vec<Div> = segments
        .into_iter()
        .map(|(chunk, is_match)| {
            let color = if is_match {
                theme.text_accent
            } else {
                theme.text_primary
            };
            div().text_color(rgba(color)).child(chunk.to_string())
        })
        .collect();

    div().flex().items_center().children(parts)
}

// Shorten the directory segment shown in each result row.
fn shorten_dir_for_row(dir: &str, max_chars: usize) -> String {
    let trimmed = dir.trim_matches('/');
    if trimmed.is_empty() {
        return String::new();
    }

    PathShortenStrategy::MiddleNumber.shorten_path(std::path::Path::new(trimmed), max_chars)
}

// Keep file search on the fast fuzzy path unless the query looks like a
// filename or path filter.
//
// This avoids parsing code-shaped queries like `struct Data {` as glob-like
// constraints, while preserving filename/path searches such as `main.rs`,
// `src/foo`, `*.toml`, and `type:rust`.
fn should_parse_file_constraints(query: &str) -> bool {
    query.split_whitespace().any(|token| {
        token.contains('/') || token.contains(':') || token.starts_with('.') || token.contains('.')
    })
}

// Keep only tokens that can actually help fuzzy file-name matching.
//
// Punctuation-only crumbs like `{`, `}`, `(`, `)` are common in code-shaped
// queries but are useless for file-name fuzziness and can make the search
// require impossible extra matches.
fn is_useful_fuzzy_token(token: &str) -> bool {
    token.chars().any(|c| c.is_ascii_alphanumeric())
}

fn build_file_query<'a>(query: &'a str) -> FFFQuery<'a> {
    let query = query.trim();
    if query.is_empty() {
        return FFFQuery {
            raw_query: query,
            constraints: Vec::new(),
            fuzzy_query: FuzzyQuery::Empty,
            location: None,
        };
    }

    if should_parse_file_constraints(query) {
        let parser = QueryParser::new(FileSearchConfig);
        return parser.parse(query);
    }

    let fuzzy_parts: Vec<&str> = query
        .split_whitespace()
        .filter(|token| is_useful_fuzzy_token(token))
        .collect();

    let fuzzy_query = match fuzzy_parts.as_slice() {
        [] => FuzzyQuery::Empty,
        [single] => FuzzyQuery::Text(single),
        parts => FuzzyQuery::Parts(parts.to_vec()),
    };

    FFFQuery {
        raw_query: query,
        constraints: Vec::new(),
        fuzzy_query,
        location: None,
    }
}

// Resolve the git-status colour used for the row's left-edge bar from the
// theme's git tokens. `clean`/no-status rows draw no bar.
fn git_status_bar_color(status: Option<&str>, theme: &AppTheme) -> Option<u32> {
    match status {
        Some("modified") => Some(theme.git_modified),
        Some("staged_new") | Some("staged_modified") => Some(theme.git_created),
        Some("staged_deleted") | Some("deleted") => Some(theme.git_deleted),
        Some("renamed") => Some(theme.git_renamed),
        Some("untracked") => Some(theme.git_untracked),
        Some("ignored") => Some(theme.git_ignored),
        // fff-search never emits "conflict" today (conflicted files come
        // through as `None`), but map it defensively should that change.
        Some("conflict") => Some(theme.git_conflict),
        Some("clean") | None => None,
        Some(_) => Some(theme.git_ignored),
    }
}

// Preview gutter number color: the centered match line (the same row the
// active-line wash highlights) reads `editor_active_line_number`; every other
// row reads `editor_line_number`.
fn gutter_number_color(
    has_match: bool,
    line_number_color: u32,
    active_line_number_color: u32,
) -> u32 {
    if has_match {
        active_line_number_color
    } else {
        line_number_color
    }
}

fn render_file_icon(icon: Option<FileIconPath>, muted: u32) -> AnyElement {
    match icon {
        Some(FileIconPath::Embedded(path)) => svg()
            .path(path)
            .size(px(16.0))
            .flex_shrink_0()
            .text_color(rgba(muted))
            .into_any_element(),
        Some(FileIconPath::External(path)) => {
            img(path).size(px(16.0)).flex_shrink_0().into_any_element()
        }
        None => div()
            .w(px(16.0))
            .h(px(16.0))
            .flex_shrink_0()
            .into_any_element(),
    }
}

// Run a live grep query using the upstream parser and grep engine.
fn execute_grep_search(
    picker: &FilePicker,
    query: &str,
    base: &Path,
    excluded_dirs: &[PathBuf],
    abort_signal: Arc<AtomicBool>,
    grep_mode: GrepMode,
) -> (Vec<FileItemSnapshot>, usize, usize) {
    let query = query.trim();
    let fuzzy_query_text: String;
    let parsed = match grep_mode {
        GrepMode::Fuzzy => {
            fuzzy_query_text = query
                .split_whitespace()
                .filter(|token| is_useful_fuzzy_token(token))
                .collect::<Vec<_>>()
                .join(" ");
            let fuzzy_query = if fuzzy_query_text.is_empty() {
                FuzzyQuery::Empty
            } else {
                FuzzyQuery::Text(fuzzy_query_text.as_str())
            };
            FFFQuery {
                raw_query: query,
                constraints: Vec::new(),
                fuzzy_query,
                location: None,
            }
        }
        _ => {
            fuzzy_query_text = String::new();
            fff_search::grep::parse_grep_query(query)
        }
    };
    let primary_mode = grep_mode;

    let grep_started = Instant::now();
    let run = |mode| {
        picker.grep(
            &parsed,
            &GrepSearchOptions {
                mode,
                page_limit: 200,
                max_matches_per_file: 200,
                smart_case: true,
                time_budget_ms: GREP_TIME_BUDGET_MS,
                abort_signal: Some(abort_signal.clone()),
                ..Default::default()
            },
        )
    };

    let grep_result = run(primary_mode);
    let mut items: Vec<FileItemSnapshot> = Vec::new();
    let mut item_by_path = std::collections::HashMap::<PathBuf, usize>::new();
    for gm in &grep_result.matches {
        let Some(fi) = grep_result.files.get(gm.file_index) else {
            continue;
        };
        if fi.is_binary() {
            continue;
        }
        let absolute_path = fi.absolute_path(picker, base);
        if path_is_excluded(&absolute_path, excluded_dirs) {
            continue;
        }
        let file_name = fi.file_name(picker);
        let dir = fi.dir_str(picker);
        let grep_match = grep_match_line(
            &absolute_path,
            gm.line_number,
            gm.col,
            &gm.line_content,
            &gm.match_byte_offsets,
        );
        if let Some(&idx) = item_by_path.get(&absolute_path) {
            items[idx].grep_matches.push(grep_match);
        } else {
            item_by_path.insert(absolute_path.clone(), items.len());
            items.push(FileItemSnapshot {
                git_status: format_git_status_opt(fi.git_status).map(str::to_string),
                frecency_score: fi.access_frecency_score,
                match_ranges: find_match_ranges(query, &file_name),
                file_name,
                dir,
                absolute_path,
                grep_matches: vec![grep_match],
            });
        }
    }

    let total_files_seen = grep_result.total_files.max(grep_result.filtered_file_count);
    let total_matched = items.len();
    info!(
        query = %query,
        grep_mode = ?grep_mode,
        primary_mode = ?primary_mode,
        fuzzy_query = %fuzzy_query_text,
        total_files = total_files_seen,
        total_matched,
        returned = items.len(),
        elapsed = ?grep_started.elapsed(),
        "grep search completed"
    );
    (items, total_files_seen, total_matched)
}

// Map one engine grep match onto the picker's per-line snapshot. `col` is the
// engine's 0-based byte offset of the first match start within the line. Also
// computes the line's syntax spans here — `execute_grep_search` runs inside
// the spawned background search task, so the tree-sitter work stays off the
// main thread.
fn grep_match_line(
    path: &Path,
    line_number: u64,
    col: usize,
    line_content: &str,
    match_byte_offsets: &[(u32, u32)],
) -> GrepMatchLine {
    GrepMatchLine {
        line_number,
        line_content: line_content.to_string(),
        byte_ranges: match_byte_offsets.to_vec(),
        col: col as u32,
        syntax_spans: preview::highlight_single_line(path, line_content),
    }
}

// Recompute every grep match's syntax spans from its stored line content using
// the current theme output. `grep_match_line` bakes each line's colors in at
// search time, so a live theme change leaves match rows on the old palette
// until the next search; the render theme-version guard calls this to refresh
// them in place. Cheap — `preview::highlight_single_line` reuses the cached
// per-language highlight config and only re-parses each short line. Pure over
// its inputs (reads each item's path + line text, rewrites the spans) —
// unit-tested below.
fn refresh_syntax_spans(items: &mut [FileItemSnapshot]) {
    for item in items {
        let path = item.absolute_path.as_path();
        for m in &mut item.grep_matches {
            m.syntax_spans = preview::highlight_single_line(path, &m.line_content);
        }
    }
}

// Editor goto position for one grep match: its line plus the 1-based CHAR
// column of the first matched range. The stored `col` byte offset is match
// identity only — editors address characters, so the column is recomputed from
// the line text (falling back to column 1 when there is no range or the byte
// offset is not a char boundary). Pure — unit-tested below.
fn match_goto(m: &GrepMatchLine) -> (usize, usize) {
    let line = m.line_number as usize;
    let column = m
        .byte_ranges
        .first()
        .and_then(|(start, _)| {
            let start = *start as usize;
            m.line_content
                .get(..start)
                .map(|prefix| prefix.chars().count() + 1)
        })
        .unwrap_or(1);
    (line, column)
}

// Resolve one multiselect key to its open entry `(path, Option<(line, col)>)`.
// Grep keys open at their OWN match's line/char-col (looked up by the
// (line, col) identity triple in the current results); Files keys open the
// file with no goto. Defensive fallbacks for a selection that drifted from the
// results (normally the post-search prune removes such keys): the exact match
// gone but the file still listed → open at the key's line, column 1; the path
// gone from the results entirely → `None` (skip the entry). Pure —
// unit-tested below.
fn goto_for_key(
    key: &rows::SelectionKey,
    results: &[FileItemSnapshot],
) -> Option<(PathBuf, Option<(usize, usize)>)> {
    let (path, match_key) = key;
    let item = results.iter().find(|item| &item.absolute_path == path);
    match match_key {
        None => item.map(|_| (path.clone(), None)),
        Some((line, col)) => {
            let goto = item?
                .grep_matches
                .iter()
                .find(|m| m.line_number == *line && m.col == *col)
                .map(match_goto)
                .unwrap_or((*line as usize, 1));
            Some((path.clone(), Some(goto)))
        }
    }
}

// What Enter opens for the cursor row when the multiselect set is empty: a
// grep Match row opens its OWN match (line + 1-based char col), a Files-view
// row opens the file with no goto (its `m: 0` finds no grep match), and
// Header rows are a no-op (Zed parity: Enter on a collapsed header does
// nothing; expanded headers are never cursor-selectable in the first place —
// see `rows::can_select`). `None` = open nothing. Pure — unit-tested below.
fn cursor_open(
    rows: &[ResultRow],
    selected: usize,
    results: &[FileItemSnapshot],
) -> Option<(PathBuf, Option<(usize, usize)>)> {
    let (file, m) = rows::resolve_row(rows, selected)?;
    let m = m?; // Header row: no-op.
    let item = results.get(file)?;
    let goto = item.grep_matches.get(m).map(match_goto);
    Some((item.absolute_path.clone(), goto))
}

// The open list for a non-empty multiselect: one open per file — the FIRST
// selected match of each (`rows::dedupe_opens`) — each resolved through
// `goto_for_key`; entries whose path has vanished from the results are
// skipped. Pure — unit-tested below.
fn opens_for_selection(
    selection: &BTreeSet<rows::SelectionKey>,
    results: &[FileItemSnapshot],
) -> Vec<(PathBuf, Option<(usize, usize)>)> {
    rows::dedupe_opens(selection)
        .iter()
        .filter_map(|key| goto_for_key(key, results))
        .collect()
}

// What the preview pane shows for the cursor row: the row's file, the line to
// CENTER the window on, and ALL of the file's matches for the overlay (which
// match is centered never changes the highlighted ranges). A Match row centers
// its OWN match's line; a (collapsed) Header row centers the file's first
// match; a Files-view row has no matches — no center, empty overlay.
// Separator rows and out-of-range indices preview nothing. Pure — unit-tested
// below.
fn preview_target(
    rows: &[ResultRow],
    selected: usize,
    results: &[FileItemSnapshot],
) -> Option<(PathBuf, Option<usize>, Vec<GrepMatchLine>)> {
    let (file, m) = rows::resolve_row(rows, selected)?;
    let item = results.get(file)?;
    let center = m
        .and_then(|m| item.grep_matches.get(m))
        .map(|gm| gm.line_number as usize)
        .or_else(|| {
            item.grep_matches
                .iter()
                .map(|gm| gm.line_number as usize)
                .min()
        });
    Some((
        item.absolute_path.clone(),
        center,
        item.grep_matches.clone(),
    ))
}

// Scroll row for serving a preview request from the ALREADY-loaded window
// instead of reloading — Some only when the cursor moved between matches of
// the same file and the loaded window can serve the new center exactly: same
// path, no load in flight, the window holds the whole file (start line 1 and
// under the `MAX_PREVIEW_LINES` cap — files at or over the cap may be
// truncated and re-window around the new center, so they take the full reload
// path), and the target line is inside it. The caller keeps the loaded path
// honest: `preview_path` is cleared whenever the loaded spans may be stale
// (search reapply, view switch, theme change). Pure — unit-tested below.
fn recenter_scroll_row(
    loaded_path: Option<&Path>,
    loading: bool,
    start_line: usize,
    loaded_lines: usize,
    path: &Path,
    center_line: Option<usize>,
) -> Option<usize> {
    let center = center_line?;
    let whole_file_loaded = start_line == 1 && loaded_lines < preview::MAX_PREVIEW_LINES;
    (!loading
        && loaded_path == Some(path)
        && whole_file_loaded
        && (1..=loaded_lines).contains(&center))
    .then(|| center - 1)
}

// Singular/plural word choice for status-bar counts.
fn plural<'a>(n: usize, one: &'a str, many: &'a str) -> &'a str {
    if n == 1 { one } else { many }
}

// Left status-bar text. Precedence: a set `status_message` wins outright, then
// the indexing-in-progress line while the scan runs, then the counts — Files
// view keeps its flat "shown/selected/matches/indexed" counts, Grep view shows
// "N matches in M files" where N is the total of VISIBLE matches summed fresh
// from the results (`total_matched` is NOT reusable there: in grep view
// `execute_grep_search` sets it to the deduped FILE count). Grep view always
// appends the submode hint. Pure — unit-tested below.
#[allow(clippy::too_many_arguments)] // plain snapshot of picker state, by design
fn status_left_text(
    view: SearchView,
    grep_mode: GrepMode,
    status_message: Option<&str>,
    scan_done: bool,
    indexed_count: usize,
    total_files: usize,
    total_matched: usize,
    selected_count: usize,
    results: &[FileItemSnapshot],
) -> String {
    let mut text = if let Some(message) = status_message {
        message.to_string()
    } else if !scan_done {
        if indexed_count > 0 {
            format!("indexing. {indexed_count} files")
        } else {
            String::new()
        }
    } else {
        let indexed = if total_files > 0 {
            total_files
        } else {
            indexed_count
        };
        match view {
            SearchView::Files => format!(
                "{} shown  {selected_count} selected  {total_matched} matches  {indexed} indexed",
                results.len()
            ),
            SearchView::Grep => {
                let match_total: usize = results.iter().map(|item| item.grep_matches.len()).sum();
                let file_count = results.len();
                format!(
                    "{match_total} {} in {file_count} {}  {selected_count} selected  {indexed} indexed",
                    plural(match_total, "match", "matches"),
                    plural(file_count, "file", "files"),
                )
            }
        }
    };
    if view == SearchView::Grep {
        let mode = match grep_mode {
            GrepMode::PlainText => "plain",
            GrepMode::Regex => "regex",
            GrepMode::Fuzzy => "fuzzy",
        };
        if !text.is_empty() {
            text.push_str("  \u{2022}  ");
        }
        text.push_str(&format!("mode: {mode}  \u{21E7}tab mode"));
    }
    text
}

// Right status-bar key hints. Grep view adds the fold hint; both views keep
// the mode-switch hint. Pure — unit-tested below.
fn status_right_hints(view: SearchView) -> &'static str {
    match view {
        SearchView::Grep => {
            "\u{2191}\u{2193} nav  \u{21E5} mark  \u{2318}\u{21E7}S multi  \u{2325}Z fold  cmd-f files  \u{23CE} open  esc quit"
        }
        SearchView::Files => {
            "\u{2191}\u{2193} nav  \u{21E5} mark  \u{2318}\u{21E7}S multi  cmd-g grep  \u{23CE} open  esc quit"
        }
    }
}

// Cut the byte window `[start, end)` out of a span list, preserving styling.
// Window edges falling inside a multi-byte char are clamped per-span (start
// shrinks left, end grows right — `clamp_range_to_char_boundaries`), so a
// misaligned window never panics. Pure — unit-tested below.
fn slice_spans(
    spans: &[preview::HighlightedSpan],
    start: usize,
    end: usize,
) -> Vec<preview::HighlightedSpan> {
    let mut out = Vec::new();
    let mut span_start = 0usize;
    for span in spans {
        let span_end = span_start + span.text.len();
        let s = start.max(span_start);
        let e = end.min(span_end);
        if s < e
            && let Some(range) =
                clamp_range_to_char_boundaries(&span.text, s - span_start, e - span_start)
        {
            out.push(preview::HighlightedSpan {
                text: span.text[range].to_string(),
                ..span.clone()
            });
        }
        span_start = span_end;
    }
    out
}

// Assemble the display spans for one grep match row: trim the line's
// leading/trailing whitespace out of its syntax spans (shifting the match
// byte ranges to compensate), then overlay the match ranges so matched chunks
// carry the search-match background + bold. Empty `syntax_spans` (plain
// fallback) become a single uncolored span over the line first;
// whitespace-only lines yield no spans. Pure — unit-tested below.
fn match_row_spans(
    syntax_spans: &[preview::HighlightedSpan],
    line_content: &str,
    byte_ranges: &[(u32, u32)],
    fallback_color: u32,
    match_bg: u32,
) -> Vec<preview::HighlightedSpan> {
    let start = line_content.len() - line_content.trim_start().len();
    let end = start + line_content.trim().len();
    if start >= end {
        return Vec::new();
    }

    let plain_fallback;
    let spans = if syntax_spans.is_empty() {
        plain_fallback = vec![preview::HighlightedSpan {
            color: fallback_color,
            bg: None,
            italic: false,
            bold: false,
            underline: false,
            strikethrough: false,
            matched: false,
            text: line_content.to_string(),
        }];
        plain_fallback.as_slice()
    } else {
        syntax_spans
    };

    let trimmed = slice_spans(spans, start, end);
    let trimmed_len = (end - start) as u32;
    let shifted: Vec<(u32, u32)> = byte_ranges
        .iter()
        .filter_map(|&(s, e)| {
            let s = s.saturating_sub(start as u32).min(trimmed_len);
            let e = e.saturating_sub(start as u32).min(trimmed_len);
            (s < e).then_some((s, e))
        })
        .collect();
    preview::overlay_match_ranges(&trimmed, &shifted, Some(match_bg))
}

// Left-truncate `text` to at most `max_chars` characters (ellipsis included),
// keeping the tail — Zed's `.truncate_start()` equivalent for header
// directory paths. Char-based, so multi-byte text never splits mid-char.
fn truncate_start(text: &str, max_chars: usize) -> String {
    let char_count = text.chars().count();
    if char_count <= max_chars {
        return text.to_string();
    }
    if max_chars == 0 {
        return String::new();
    }
    let keep = max_chars - 1;
    let mut out = String::from("\u{2026}");
    out.extend(text.chars().skip(char_count - keep));
    out
}

fn path_is_excluded(path: &Path, excluded_dirs: &[PathBuf]) -> bool {
    excluded_dirs
        .iter()
        .any(|excluded| path.starts_with(excluded))
}

impl FffPicker {
    // Create a new picker rooted at `base_path` and start the background file scan.
    #[allow(clippy::too_many_arguments)] // wiring constructor; each arg is a distinct dependency
    pub fn new(
        base_path: PathBuf,
        shared: PickerSharedState,
        enable_content_indexing: bool,
        follow_symlinks: bool,
        start_in_grep: bool,
        excluded_dirs: Vec<PathBuf>,
        print_stdout: bool,
        editor: String,
        responder: Option<ResponderArc>,
        cx: &mut Context<Self>,
    ) -> Self {
        let text_field = cx.new(|cx| TextField::new("Search files...", cx));

        cx.observe(&text_field, |this, _entity, cx| {
            let new_query = this.text_field.read(cx).text();
            if new_query != this.query {
                // Anything we did not inject ourselves is a real edit, which
                // drops the cursor so the next shift-up starts from the newest
                // entry again.
                let injected_by_history = this
                    .history_nav
                    .as_ref()
                    .is_some_and(|nav| nav.injected == new_query);
                if !injected_by_history {
                    this.history_nav = None;
                }
                this.query = new_query;
                this.status_message = None;
                Arc::make_mut(&mut this.selection).clear();
                this.preview_scroll_row = 0;
                this.run_search(cx);
            }
        })
        .detach();

        let mut instance = Self {
            shared_picker: shared.shared_picker,
            shared_frecency: shared.shared_frecency,
            shared_query_tracker: shared.shared_query_tracker,
            view: if start_in_grep {
                SearchView::Grep
            } else {
                SearchView::Files
            },
            excluded_dirs,
            print_stdout,
            grep_mode: GrepMode::PlainText,
            query: String::new(),
            history_nav: None,
            results: Arc::new(Vec::new()),
            total_files: 0,
            total_matched: 0,
            indexed_count: 0,
            selected: 0,
            multi_select_mode: false,
            selection: Arc::new(BTreeSet::new()),
            rows: Vec::new(),
            collapsed: HashSet::new(),
            max_match_line: 0,
            scan_done: false,
            search_epoch: 0,
            search_in_flight: false,
            search_abort: None,
            preview_epoch: 0,
            preview_loading: false,
            preview_loading_visible: false,
            preview_scroll_row: 0,
            preview_start_line: 1,
            preview_path: None,
            session_results_width: load_session_results_width(),
            theme_version: theme::version(),
            focus_handle: cx.focus_handle(),
            results_list: ListState::new(0, ListAlignment::Top, px(512.0)),
            preview_scroll: UniformListScrollHandle::new(),
            preview_lines: Arc::new(Vec::new()),
            status_message: None,
            text_field,
            editor,
            dismiss_on_blur: None,
            dismiss_on_window_blur: None,
            responder,
        };

        instance.start_scan(base_path, enable_content_indexing, follow_symlinks, cx);
        instance
    }

    // Close the popup when the window loses focus, matching Raycast-style dismissal.
    pub fn install_focus_lost_dismiss(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.dismiss_on_window_blur =
            Some(cx.observe_window_activation(window, |_this, window, _cx| {
                if !window.is_window_active() {
                    window.remove_window();
                }
            }));
        self.dismiss_on_blur =
            Some(
                cx.on_blur(&self.focus_handle, window, |_this, window, _cx| {
                    window.remove_window();
                }),
            );
    }

    // Start the file indexer and trigger the initial search when indexing is ready.
    #[tracing::instrument(skip(self, cx, base_path), fields(base_path = %base_path.display(), enable_content_indexing, follow_symlinks))]
    fn start_scan(
        &mut self,
        base_path: PathBuf,
        enable_content_indexing: bool,
        follow_symlinks: bool,
        cx: &mut Context<Self>,
    ) {
        let sp = self.shared_picker.clone();
        let sf = self.shared_frecency.clone();
        let sq = self.shared_query_tracker.clone();

        self.spawn_index_progress_poll(cx);

        let existing_picker = self.shared_picker.read().ok().and_then(|guard| {
            guard.as_ref().map(|picker| {
                (
                    picker.base_path().to_path_buf(),
                    picker.get_scan_progress().is_scanning,
                )
            })
        });

        if let Some((existing_base_path, is_scanning)) = existing_picker
            && existing_base_path == base_path
        {
            info!(
                base_path = %base_path.display(),
                is_scanning,
                "reusing resident file index"
            );
            if is_scanning {
                cx.spawn(
                    async move |this: WeakEntity<FffPicker>, cx: &mut AsyncApp| {
                        let scan_done =
                            smol::unblock(move || sp.wait_for_scan(Duration::from_secs(60))).await;
                        if !scan_done {
                            warn!(base_path = %base_path.display(), "resident file scan timed out");
                        }

                        let update_result = this.update(cx, |this, cx| {
                            this.scan_done = true;
                            cx.notify();
                            this.run_search(cx);
                            info!(
                                scan_done = this.scan_done,
                                results = this.results.len(),
                                "resident scan state applied to picker"
                            );
                        });

                        if let Err(err) = update_result {
                            warn!(
                                error = %err,
                                "failed to apply resident scan state to picker"
                            );
                        }
                    },
                )
                .detach();
            } else {
                self.scan_done = true;
                self.run_search(cx);
                info!(
                    scan_done = self.scan_done,
                    results = self.results.len(),
                    "resident scan state applied to picker"
                );
            }
            return;
        }

        info!("starting file index");

        cx.spawn(
            async move |this: WeakEntity<FffPicker>, cx: &mut AsyncApp| {
                smol::unblock(move || {
                    preview::warm_highlighter();

                    trace!(home = ?std::env::var("HOME").ok(), "initializing shared trackers");
                    if let Ok(home) = std::env::var("HOME") {
                        let data_dir = PathBuf::from(home).join(".local/share/fff");
                        let _ = std::fs::create_dir_all(&data_dir);
                        if let Ok(tracker) =
                            FrecencyTracker::open(data_dir.join("frecency.lmdb"))
                        {
                            let _ = sf.init(tracker);
                        }
                        if let Ok(tracker) = QueryTracker::open(
                            data_dir.join("queries.lmdb").to_string_lossy().as_ref(),
                        ) {
                            let _ = sq.init(tracker);
                        }
                    }
                    if let Err(err) = FilePicker::new_with_shared_state(
                        sp.clone(),
                        sf,
                        FilePickerOptions {
                            base_path: base_path.to_string_lossy().to_string(),
                            enable_mmap_cache: false,
                            // Disable the persistent grep content cache so
                            // grep falls back to a per-search reusable buffer
                            // instead of mmap-pinning every searched file
                            // (default allows ~512 MB and never frees in a
                            // daemon-resident picker). Keep max_file_size at
                            // its default (10 MB) — zeroing it would make
                            // get_content_for_search reject every file and
                            // grep would return no matches.
                            cache_budget: Some(ContentCacheBudget {
                                max_files: 0,
                                max_bytes: 0,
                                ..ContentCacheBudget::default()
                            }),
                            enable_content_indexing,
                            follow_symlinks,
                            mode: FFFMode::Neovim,
                            // Keep the resident index live: fff-search's
                            // background watcher applies create/modify/delete
                            // events to the in-memory file list in real time,
                            // so files created after the daemon started are
                            // searchable without a restart (matching fff.nvim).
                            // The watcher installs after the initial snapshot
                            // and fails soft (logs, static index) for home/root
                            // base paths, so this never blocks or breaks a scan.
                            watch: true,
                            ..Default::default()
                        },
                    ) {
                        error!(error = %err, base_path = %base_path.display(), "failed to initialize file picker");
                    }

                    let scan_completed = sp.wait_for_scan(Duration::from_secs(60));
                    if scan_completed {
                        info!(base_path = %base_path.display(), "initial file scan completed");
                    } else {
                        warn!(base_path = %base_path.display(), "initial file scan timed out");
                    }
                })
                .await;

                let update_result = this.update(cx, |this, cx| {
                    this.scan_done = true;
                    cx.notify();
                    this.run_search(cx);
                    info!(
                        scan_done = this.scan_done,
                        results = this.results.len(),
                        "scan completion applied to picker state"
                    );
                });

                if let Err(err) = update_result {
                    warn!(error = %err, "failed to apply scan completion to picker state");
                }
            },
        )
        .detach();
    }

    // Poll the shared picker's scan progress at ~150 ms while scanning is
    // active and publish `scanned_files_count` into `indexed_count` so the
    // UI can render a live counter. The loop exits as soon as the scan
    // reports `is_scanning == false`, or when the entity is dropped.
    fn spawn_index_progress_poll(&self, cx: &mut Context<Self>) {
        let shared_picker = self.shared_picker.clone();
        cx.spawn(
            async move |this: WeakEntity<FffPicker>, cx: &mut AsyncApp| {
                loop {
                    let Some(progress) = shared_picker
                        .read()
                        .ok()
                        .and_then(|guard| guard.as_ref().map(|p| p.get_scan_progress()))
                    else {
                        smol::Timer::after(Duration::from_millis(150)).await;
                        if this.upgrade().is_none() {
                            return;
                        }
                        continue;
                    };

                    let count = progress.scanned_files_count;
                    let scanning = progress.is_scanning;
                    let update = this.update(cx, |this, cx| {
                        if this.indexed_count != count {
                            this.indexed_count = count;
                            cx.notify();
                        }
                    });

                    if update.is_err() {
                        return;
                    }

                    if !scanning {
                        return;
                    }

                    smol::Timer::after(Duration::from_millis(150)).await;
                }
            },
        )
        .detach();
    }

    // Run the active search view and render the corresponding result set.
    fn run_search(&mut self, cx: &mut Context<Self>) {
        if !self.scan_done {
            return;
        }

        if let Some(abort) = &self.search_abort {
            abort.store(true, Ordering::Release);
        }
        self.search_epoch = self.search_epoch.wrapping_add(1);
        self.search_in_flight = true;
        let abort_signal = Arc::new(AtomicBool::new(false));
        self.search_abort = Some(abort_signal.clone());
        let epoch = self.search_epoch;
        let shared_picker = self.shared_picker.clone();
        let shared_query_tracker = self.shared_query_tracker.clone();
        let query_str = self.query.clone();
        let view = self.view;
        let grep_mode = self.grep_mode;
        let excluded_dirs = self.excluded_dirs.clone();
        info!(
            epoch,
            query = %query_str.trim(),
            view = ?view,
            grep_mode = ?grep_mode,
            "starting search"
        );

        cx.spawn(
            async move |this: WeakEntity<FffPicker>, cx: &mut AsyncApp| {
                let (items, total_files, total_matched) = smol::unblock(move || {
                    let Ok(guard) = shared_picker.read() else {
                        return (Vec::new(), 0, 0);
                    };
                    let Some(picker) = guard.as_ref() else {
                        return (Vec::new(), 0, 0);
                    };

                    let base = picker.base_path().to_path_buf();
                    let query = query_str.trim().to_string();

                    match view {
                        SearchView::Files => {
                            let file_search_started = Instant::now();
                            let parse_started = Instant::now();
                            let parsed = build_file_query(&query);
                            let parse_elapsed = parse_started.elapsed();
                            let query_tracker = shared_query_tracker.read().ok();
                            let search = picker.fuzzy_search(
                                &parsed,
                                query_tracker
                                    .as_deref()
                                    .and_then(|tracker| tracker.as_ref()),
                                FuzzySearchOptions {
                                    max_threads: search_threads(),
                                    project_path: Some(picker.base_path()),
                                    combo_boost_score_multiplier: 100,
                                    min_combo_count: 3,
                                    pagination: PaginationArgs {
                                        offset: 0,
                                        limit: 200,
                                    },
                                    ..Default::default()
                                },
                            );
                            let fuzzy_items = search
                                .items
                                .iter()
                                .filter_map(|fi| {
                                    if fi.is_binary() {
                                        return None;
                                    }
                                    let absolute_path = fi.absolute_path(picker, &base);
                                    if path_is_excluded(&absolute_path, &excluded_dirs) {
                                        return None;
                                    }
                                    let file_name = fi.file_name(picker);
                                    let dir = fi.dir_str(picker);
                                    Some(FileItemSnapshot {
                                        git_status: format_git_status_opt(fi.git_status)
                                            .map(str::to_string),
                                        frecency_score: fi.access_frecency_score,
                                        match_ranges: find_match_ranges(&query, &file_name),
                                        file_name,
                                        dir,
                                        absolute_path,
                                        grep_matches: vec![],
                                    })
                                })
                                .collect::<Vec<_>>();
                            let visible_results = fuzzy_items.len();
                            info!(
                                epoch,
                                query = %query,
                                query_mode = if parsed.constraints.is_empty() { "plain_fuzzy" } else { "file_filter" },
                                constraints = parsed.constraints.len(),
                                parse_elapsed = ?parse_elapsed,
                                search_elapsed = ?file_search_started.elapsed(),
                                total_files = search.total_files,
                                total_matched = search.total_matched,
                                visible_results,
                                "file search completed"
                            );

                            (fuzzy_items, search.total_files, visible_results)
                        }
                        SearchView::Grep => {
                            if query.is_empty() {
                                return (Vec::new(), 0, 0);
                            }

                            execute_grep_search(
                                picker,
                                &query_str,
                                &base,
                                &excluded_dirs,
                                abort_signal,
                                grep_mode,
                            )
                        }
                    }
                })
                .await;

                let update_result = this.update(cx, |this, cx| {
                    if this.search_epoch != epoch {
                        trace!(epoch, "discarding stale search result");
                        this.finish_search(epoch, cx);
                        return;
                    }
                    debug!(
                        epoch,
                        results = items.len(),
                        total_files,
                        total_matched,
                        "applying search result"
                    );
                    this.results = Arc::new(items);
                    // New results can change the same file's overlay ranges —
                    // force `load_preview` down the full reload path.
                    this.preview_path = None;
                    this.total_files = total_files;
                    this.total_matched = total_matched;
                    this.collapsed.clear();
                    this.rebuild_rows();
                    this.selected =
                        rows::first_selectable(&this.rows, &this.collapsed, &this.results)
                            .unwrap_or(0);
                    this.preview_scroll_row = 0;
                    Arc::make_mut(&mut this.selection)
                        .retain(|key| rows::key_survives(key, &this.results));
                    this.results_list.scroll_to(ListOffset::default());
                    this.load_preview(cx);
                    cx.notify();
                    info!(
                        epoch,
                        view = ?this.view,
                        query = %this.query,
                        visible_results = this.results.len(),
                        selected = this.selected,
                        scan_done = this.scan_done,
                        "search results applied"
                    );
                    this.finish_search(epoch, cx);
                    trace!(
                        epoch,
                        scan_done = this.scan_done,
                        results = this.results.len(),
                        "search result applied to picker state"
                    );
                });

                if let Err(err) = update_result {
                    warn!(error = %err, epoch, "failed to apply search result to picker state");
                }
            },
        )
        .detach();
    }

    // Finish the active search and schedule any query that arrived while it was running.
    fn finish_search(&mut self, epoch: u64, _cx: &mut Context<Self>) {
        if self.search_epoch != epoch {
            return;
        }
        self.search_in_flight = false;
        self.search_abort = None;
    }

    // True while the grep view owns the results pane (rows are grouped).
    fn is_grep_view(&self) -> bool {
        self.view == SearchView::Grep
    }

    // Rebuild the derived row projection after `results`/`collapsed`/view
    // changes and resize the list state to match. Callers re-seed or
    // re-anchor `selected` themselves.
    fn rebuild_rows(&mut self) {
        self.rows = rows::build_rows(&self.results, &self.collapsed, self.is_grep_view());
        self.max_match_line = rows::max_line_number(&self.results);
        self.results_list.reset(self.rows.len());
    }

    // Resolve the cursor row to its file snapshot. Header and Match rows both
    // map to a file; Separator rows and out-of-range indices resolve to None.
    // Every consumer of "the selected result" goes through this — `selected`
    // indexes `rows`, and using it against `results` would hit the wrong file.
    fn selected_row_snapshot(&self) -> Option<&FileItemSnapshot> {
        let (file, _m) = rows::resolve_row(&self.rows, self.selected)?;
        self.results.get(file)
    }

    // Clear results and repaint immediately, then kick off a fresh search on
    // the next frame. This lets GPUI flush the mode-change render before the
    // search work starts, avoiding a visible hang on the stale result list.
    fn switch_mode(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(abort) = &self.search_abort {
            abort.store(true, Ordering::Release);
        }
        // Bump both epochs so any search/preview task started before the switch
        // is discarded by the existing staleness checks even if it lands in the
        // window between here and the deferred `run_search` below — otherwise a
        // stale task could complete with the epoch unchanged and paint results
        // on top of the state we just cleared.
        self.search_epoch = self.search_epoch.wrapping_add(1);
        self.preview_epoch = self.preview_epoch.wrapping_add(1);
        Arc::make_mut(&mut self.results).clear();
        self.total_files = 0;
        self.total_matched = 0;
        // Files and Grep read separate history stacks, so an offset carried
        // across the switch would point into the wrong one.
        self.history_nav = None;
        self.collapsed.clear();
        self.rebuild_rows();
        self.selected = 0;
        Arc::make_mut(&mut self.selection).clear();
        Arc::make_mut(&mut self.preview_lines).clear();
        self.preview_path = None;
        self.preview_loading = false;
        self.preview_loading_visible = false;
        self.status_message = None;
        cx.notify();
        cx.defer_in(window, |this, _window, cx| {
            this.run_search(cx);
        });
    }

    // Load and syntax-highlight the selected file preview in the background.
    // The window is centered on the cursor row's target line
    // (`preview_target`: a Match row's own line, a collapsed header's first
    // match) with ALL of the file's matches overlaid. Cursor moves between
    // matches of the SAME fully-loaded file skip the reload and only re-center
    // (`recenter_scroll_row`).
    fn load_preview(&mut self, cx: &mut Context<Self>) {
        self.preview_epoch = self.preview_epoch.wrapping_add(1);
        let preview_epoch = self.preview_epoch;
        let Some((path, center_line, grep_matches)) =
            preview_target(&self.rows, self.selected, &self.results)
        else {
            self.preview_lines = Arc::new(vec![]);
            self.preview_path = None;
            self.preview_loading = false;
            self.preview_loading_visible = false;
            self.preview_scroll_row = 0;
            self.preview_start_line = 1;
            return;
        };

        if let Some(scroll_row) = recenter_scroll_row(
            self.preview_path.as_deref(),
            self.preview_loading,
            self.preview_start_line,
            self.preview_lines.len(),
            &path,
            center_line,
        ) {
            self.preview_scroll_row = scroll_row;
            self.recenter_preview();
            cx.notify();
            return;
        }

        self.preview_loading = true;
        self.preview_loading_visible = false;
        trace!(
            preview_epoch,
            path = %path.display(),
            grep_matches = grep_matches.len(),
            "loading preview"
        );
        let theme = cx.global::<AppTheme>();
        let match_bg = theme.match_highlight_bg;

        cx.spawn(
            async move |this: WeakEntity<FffPicker>, cx: &mut AsyncApp| {
                smol::Timer::after(Duration::from_millis(120)).await;
                this.update(cx, |this, cx| {
                    if this.preview_epoch == preview_epoch
                        && this.preview_loading
                        && this.preview_lines.is_empty()
                    {
                        this.preview_loading_visible = true;
                        cx.notify();
                    }
                })
                .ok();
            },
        )
        .detach();

        let loaded_path = path.clone();
        cx.spawn(
            async move |this: WeakEntity<FffPicker>, cx: &mut AsyncApp| {
                let (start_line, lines) = smol::unblock(move || {
                    let (start_line, mut lines) =
                        preview::highlight_file_window(&path, center_line);
                    for gm in &grep_matches {
                        // Skip matches whose line falls before the loaded window;
                        // otherwise a saturating subtraction would clamp them onto
                        // row 0 and stack a bogus highlight on the first visible
                        // line. `get_mut` discards matches past the window's end.
                        let Some(idx) =
                            preview::overlay_row_index(gm.line_number as usize, start_line)
                        else {
                            continue;
                        };
                        if let Some(line) = lines.get_mut(idx) {
                            line.spans = preview::overlay_match_ranges(
                                &line.spans,
                                &gm.byte_ranges,
                                Some(match_bg),
                            );
                        }
                    }
                    (start_line, lines)
                })
                .await;

                this.update(cx, |this, cx| {
                    if this.preview_epoch != preview_epoch {
                        trace!(preview_epoch, "discarding stale preview result");
                        return;
                    }
                    this.preview_lines = Arc::new(lines);
                    this.preview_path = Some(loaded_path);
                    this.preview_loading = false;
                    this.preview_loading_visible = false;
                    this.preview_start_line = start_line;
                    this.preview_scroll_row = center_line
                        .map(|line| line.saturating_sub(start_line))
                        .unwrap_or(0);
                    this.recenter_preview();
                    cx.notify();
                })
                .ok();
            },
        )
        .detach();
    }

    // Vertically center the current match row in the visible preview pane.
    //
    // Uses a strict Center scroll: it always positions the row (even when the
    // row is already visible), and the deferred scroll resolves against the
    // pane's real bounds at the next layout — so no visible-row estimate is
    // needed and it stays correct after pane resizes. Re-issue this after
    // anything that changes the preview pane size, e.g. divider drags.
    fn recenter_preview(&mut self) {
        self.preview_scroll
            .scroll_to_item_strict(self.preview_scroll_row, ScrollStrategy::Center);
    }

    // Update frecency / query trackers for the selected paths.
    fn track_open(&self, paths: &[PathBuf]) {
        if self.view == SearchView::Grep
            && let Ok(guard) = self.shared_picker.read()
            && let Some(picker) = guard.as_ref()
            && let Ok(mut tracker_guard) = self.shared_query_tracker.write()
            && let Some(tracker) = tracker_guard.as_mut()
        {
            let _ = tracker.track_grep_query(&self.query, picker.base_path());
        }

        for path in paths {
            if self.view == SearchView::Files
                && let Ok(guard) = self.shared_picker.read()
                && let Some(picker) = guard.as_ref()
                && let Ok(mut tracker_guard) = self.shared_query_tracker.write()
                && let Some(tracker) = tracker_guard.as_mut()
            {
                let _ = tracker.track_query_completion(&self.query, picker.base_path(), path);
            }

            if let Ok(guard) = self.shared_frecency.read()
                && let Some(tracker) = guard.as_ref()
            {
                let _ = tracker.track_access(path);
            }
        }
    }

    // Open the selected file(s). For client-forwarded sessions writes paths back over the IPC
    // socket (the client process spawns the editor). For daemon-side sessions (menubar /
    // hotkey / daemon-startup --open) spawns the editor inline.
    fn on_open_selected(&mut self, _: &OpenSelected, window: &mut Window, cx: &mut Context<Self>) {
        // Non-empty selection: one open per file at its first selected match.
        // Empty selection: the cursor row's own match (Header rows: no-op).
        let opens: Vec<(PathBuf, Option<(usize, usize)>)> = if !self.selection.is_empty() {
            opens_for_selection(&self.selection, &self.results)
        } else {
            cursor_open(&self.rows, self.selected, &self.results)
                .into_iter()
                .collect()
        };
        if opens.is_empty() {
            return;
        }

        let paths_to_open: Vec<PathBuf> = opens.iter().map(|(path, _)| path.clone()).collect();
        self.track_open(&paths_to_open);

        let entries: Vec<PickEntry> = opens
            .into_iter()
            .map(|(path, goto)| PickEntry {
                path,
                line: goto.map(|g| g.0),
                column: goto.map(|g| g.1),
            })
            .collect();

        if self.responder.is_some() {
            send_pick_response(self.responder.take(), &entries);
            window.remove_window();
            return;
        }

        let mut opened = 0usize;
        let mut last_error: Option<String> = None;
        for entry in &entries {
            let goto = entry.line.zip(entry.column);
            if self.print_stdout {
                println!("{}", entry.path.display());
                opened += 1;
                continue;
            }
            match editor::open_in_editor(
                &entry.path,
                goto,
                &self.editor,
                editor::EditorLaunchMode::Detached,
            ) {
                Ok(child) => {
                    info!(pid = child.id(), path = %entry.path.display(), "spawned editor");
                    opened += 1;
                }
                Err(err) => {
                    error!(error = %err, path = %entry.path.display(), "open failed");
                    last_error = Some(err.to_string());
                }
            }
        }

        if opened > 0 {
            self.status_message = Some(if entries.len() == 1 {
                format!("Opened {}", entries[0].path.display())
            } else {
                format!("Opened {opened} files")
            });
            cx.notify();
            window.remove_window();
        } else if let Some(err) = last_error {
            self.status_message = Some(format!(
                "Open failed: {err}  (log: {})",
                log::path_for_display()
            ));
            cx.notify();
        }
    }

    // Close the current picker window without terminating the resident service.
    fn on_quit(&mut self, _: &Quit, window: &mut Window, _cx: &mut Context<Self>) {
        window.remove_window();
    }
}

// Send an empty PickResponse to the client when the picker is dismissed without a selection
// (Esc, focus-loss, window-close, session replacement). For non-client sessions
// (responder=None) this is a no-op. on_open_selected calls take() first so the inner stream is
// already gone by the time Drop runs after a successful pick.
impl Drop for FffPicker {
    fn drop(&mut self) {
        if self.responder.is_some() {
            send_pick_response(self.responder.take(), &[]);
        }
    }
}

impl FffPicker {
    // Move selection down the list — toward a worse-ranked result. Steps over
    // unselectable rows (expanded headers, separators) and clamps at the end.
    fn on_select_next(&mut self, _: &SelectNext, _window: &mut Window, cx: &mut Context<Self>) {
        if let Some(next) = rows::step_selectable(
            &self.rows,
            self.selected,
            rows::Direction::Next,
            &self.collapsed,
            &self.results,
        ) {
            self.selected = next;
            self.results_list.scroll_to_reveal_item(self.selected);
            self.load_preview(cx);
            cx.notify();
        }
    }

    // Move selection up the list — toward a better-ranked result. Steps over
    // unselectable rows (expanded headers, separators) and clamps at the top.
    fn on_select_prev(&mut self, _: &SelectPrev, _window: &mut Window, cx: &mut Context<Self>) {
        if let Some(prev) = rows::step_selectable(
            &self.rows,
            self.selected,
            rows::Direction::Prev,
            &self.collapsed,
            &self.results,
        ) {
            self.selected = prev;
            self.results_list.scroll_to_reveal_item(self.selected);
            self.load_preview(cx);
            cx.notify();
        }
    }

    // Select the clicked row and refresh the preview. `index` is a ROW index;
    // clicks on unselectable rows (expanded headers, separators) are ignored.
    fn on_select_row(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        if !rows::can_select(&self.rows, index, &self.collapsed, &self.results) {
            return;
        }

        if self.selected == index {
            self.on_open_selected(&OpenSelected, window, cx);
            return;
        }

        self.selected = index;
        self.results_list.scroll_to_reveal_item(self.selected);
        self.load_preview(cx);
        let focus = self.text_field_focus_handle(cx);
        window.focus(&focus, cx);
        cx.notify();
    }

    // Fold the current row's file group (alt-z): collapse re-anchors the
    // cursor to the group header, expand to the group's first match. Grep
    // view only — Files rows have no groups, so this no-ops there.
    fn on_toggle_fold(&mut self, _: &ToggleFold, _window: &mut Window, cx: &mut Context<Self>) {
        if let Some((file, _m)) = rows::resolve_row(&self.rows, self.selected) {
            self.toggle_group_fold(file, cx);
        }
    }

    // Toggle-all (alt-shift-z): any collapsed group → expand all, otherwise
    // collapse all. Grep view only.
    fn on_toggle_fold_all(
        &mut self,
        _: &ToggleFoldAll,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.toggle_all_folds(cx);
    }

    // Toggle one file group's fold state, splicing the changed row range into
    // the list state so scroll is preserved for rows outside the group. The
    // cursor is re-anchored via `rows::anchor_selection`: onto the header when
    // its own group collapsed, onto the first match when it expanded, and back
    // onto its own (index-shifted) row when a different group was toggled.
    fn toggle_group_fold(&mut self, file: usize, cx: &mut Context<Self>) {
        if !self.is_grep_view() {
            return;
        }
        let Some(path) = self
            .results
            .get(file)
            .map(|item| item.absolute_path.clone())
        else {
            return;
        };
        let old_key = self.selected_fold_key();
        let old_range = rows::group_match_range(&self.rows, file);
        if !self.collapsed.remove(&path) {
            self.collapsed.insert(path);
        }
        self.rows = rows::build_rows(&self.results, &self.collapsed, self.is_grep_view());
        let new_range = rows::group_match_range(&self.rows, file);
        match (old_range, new_range) {
            (Some(old), Some(new)) => self.results_list.splice(old, new.len()),
            // Header not found (should not happen in grep view) — fall back
            // to a full reset rather than desync the list's row count.
            _ => self.results_list.reset(self.rows.len()),
        }
        self.reanchor_after_fold(old_key, cx);
    }

    // Toggle-all fold state. Every group's row count changes here, so the
    // list state takes a full `reset` instead of many splices (the plan's
    // accepted simpler path); scroll is restored by revealing the re-anchored
    // cursor row.
    fn toggle_all_folds(&mut self, cx: &mut Context<Self>) {
        if !self.is_grep_view() || self.results.is_empty() {
            return;
        }
        let old_key = self.selected_fold_key();
        self.collapsed = rows::toggle_all_collapsed(&self.collapsed, &self.results);
        self.rebuild_rows();
        self.reanchor_after_fold(old_key, cx);
    }

    // The cursor's (path, match) identity captured before a fold rebuild.
    // Header rows key as match 0 so expanding a collapsed group lands the
    // cursor on the group's first match row.
    fn selected_fold_key(&self) -> Option<(PathBuf, usize)> {
        let (file, m) = rows::resolve_row(&self.rows, self.selected)?;
        let path = self.results.get(file)?.absolute_path.clone();
        Some((path, m.unwrap_or(0)))
    }

    // Re-anchor the cursor after a fold rebuild and keep it visible. The
    // previewed FILE cannot change across a fold (the anchor stays within the
    // old key's file, which is still present), so the preview is left alone.
    // Accepted nuance: collapsing from a match row lands on the header, whose
    // preview target centers the file's FIRST match — the window keeps the
    // old match's center until the next cursor move (same file, same overlay
    // either way; no reload/scroll churn on fold).
    fn reanchor_after_fold(&mut self, old_key: Option<(PathBuf, usize)>, cx: &mut Context<Self>) {
        self.selected = rows::anchor_selection(
            old_key.as_ref().map(|(path, m)| (path.as_path(), *m)),
            &self.rows,
            &self.collapsed,
            &self.results,
        );
        self.results_list.scroll_to_reveal_item(self.selected);
        cx.notify();
    }

    // Toggle the multiselect key for `row_ix` in place. Header and Separator
    // rows carry no key (`rows::selection_key_for_row`), so toggling there is
    // a no-op — returns whether a key was actually toggled.
    fn toggle_key_at(&mut self, row_ix: usize) -> bool {
        let Some(key) =
            rows::selection_key_for_row(&self.rows, row_ix, &self.results, self.is_grep_view())
        else {
            return false;
        };
        let selection = Arc::make_mut(&mut self.selection);
        if !selection.insert(key.clone()) {
            selection.remove(&key);
        }
        true
    }

    // Tab (Zed's toggle+advance): enter multiselect mode, toggle the current
    // row's key, and move the cursor to the next selectable row. On a
    // collapsed header (no key) it only enters mode and advances.
    fn on_toggle_selected(
        &mut self,
        _: &ToggleSelected,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.multi_select_mode = true;
        self.toggle_key_at(self.selected);
        if let Some(next) = rows::step_selectable(
            &self.rows,
            self.selected,
            rows::Direction::Next,
            &self.collapsed,
            &self.results,
        ) {
            self.selected = next;
            self.results_list.scroll_to_reveal_item(self.selected);
            self.load_preview(cx);
        }
        cx.notify();
    }

    // Ctrl-a: enter multiselect mode and toggle-all — any key selected clears
    // the whole selection, otherwise every visible match key becomes selected
    // (per-match triples in grep view, per-file keys in Files view).
    fn on_toggle_select_all(
        &mut self,
        _: &ToggleSelectAll,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.multi_select_mode = true;
        self.selection = Arc::new(rows::toggle_all_selection(
            &self.selection,
            &self.results,
            self.is_grep_view(),
        ));
        cx.notify();
    }

    // Cmd-shift-s: explicit multiselect mode toggle. Leaving the mode clears
    // the selection — the checkboxes vanish and nothing stays secretly marked.
    fn on_toggle_multi_select_mode(
        &mut self,
        _: &ToggleMultiSelectMode,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (mode, selection) =
            rows::toggle_multi_select_mode(self.multi_select_mode, &self.selection);
        self.multi_select_mode = mode;
        self.selection = Arc::new(selection);
        cx.notify();
    }

    // Route a row click: cmd-click (the platform modifier) toggles the row's
    // multiselect key and enters multiselect mode, leaving the cursor where it
    // is; a plain click keeps the cursor-move / open-on-second-click behavior.
    fn on_row_click(
        &mut self,
        row_ix: usize,
        event: &ClickEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if event.modifiers().platform {
            if self.toggle_key_at(row_ix) {
                self.multi_select_mode = true;
                cx.notify();
            }
            return;
        }
        self.on_select_row(row_ix, window, cx);
    }

    // Shift-tab: cycle grep mode in grep view, otherwise move selection up.
    fn on_shift_tab(&mut self, _: &ShiftTab, window: &mut Window, cx: &mut Context<Self>) {
        match self.view {
            SearchView::Grep => self.on_cycle_grep_mode(&CycleGrepMode, window, cx),
            SearchView::Files => self.on_select_prev(&SelectPrev, window, cx),
        }
    }

    // Cycle through the available grep modes.
    fn on_cycle_grep_mode(
        &mut self,
        _: &CycleGrepMode,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.grep_mode = match self.grep_mode {
            GrepMode::PlainText => GrepMode::Regex,
            GrepMode::Regex => GrepMode::Fuzzy,
            GrepMode::Fuzzy => GrepMode::PlainText,
        };
        self.switch_mode(window, cx);
    }

    // Shift-up: step to an older query in the local search history.
    fn on_history_prev(&mut self, _: &HistoryPrev, _window: &mut Window, cx: &mut Context<Self>) {
        self.navigate_history(history::Direction::Older, cx);
    }

    // Shift-down: step back toward the newest query, then to the draft.
    fn on_history_next(&mut self, _: &HistoryNext, _window: &mut Window, cx: &mut Context<Self>) {
        self.navigate_history(history::Direction::Newer, cx);
    }

    // Walk the query history one entry and put the result in the field.
    // Files and Grep read separate stacks, so the view decides which one.
    fn navigate_history(&mut self, direction: history::Direction, cx: &mut Context<Self>) {
        let current = self.history_nav.as_ref().map(|nav| nav.offset);
        // Copied out so the fetch closure below doesn't borrow `self` while
        // `navigate_history` holds it mutably.
        let view = self.view;
        let shown = self.query.clone();

        let outcome = (|| {
            let guard = self.shared_query_tracker.read().ok()?;
            let tracker = guard.as_ref()?;
            let picker_guard = self.shared_picker.read().ok()?;
            let picker = picker_guard.as_ref()?;
            let project_path = picker.base_path();
            Some(history::step(
                |offset| match view {
                    SearchView::Files => tracker
                        .get_historical_query(project_path, offset)
                        .ok()
                        .flatten(),
                    SearchView::Grep => tracker
                        .get_historical_grep_query(project_path, offset)
                        .ok()
                        .flatten(),
                },
                current,
                direction,
                &shown,
            ))
        })();

        // A poisoned lock or an unopened tracker reads as an empty history.
        let outcome = outcome.unwrap_or(history::Step::Edge);

        match outcome {
            history::Step::Move { offset, query } => {
                // Entering history for the first time banks the draft so
                // shift-down can hand it back; later steps carry it along.
                let draft = match self.history_nav.take() {
                    Some(nav) => nav.draft,
                    None => shown,
                };
                // Set before `set_text` so the observer sees the cursor it
                // needs to recognize this write as ours.
                self.history_nav = Some(HistoryNav {
                    offset,
                    draft,
                    injected: query.clone(),
                });
                self.status_message = None;
                self.text_field
                    .update(cx, |field, cx| field.set_text(query, cx));
            }
            history::Step::Draft => {
                let Some(nav) = self.history_nav.take() else {
                    return;
                };
                self.text_field
                    .update(cx, |field, cx| field.set_text(nav.draft, cx));
            }
            // shift-down outside history is a no-op — the draft is already up.
            history::Step::Edge if direction == history::Direction::Newer => {}
            history::Step::Edge => {
                self.status_message = Some(match current {
                    Some(_) => "Oldest query".to_string(),
                    None => "No query history".to_string(),
                });
                cx.notify();
            }
        }
    }

    // Scroll the preview pane toward the top.
    fn on_preview_scroll_up(
        &mut self,
        _: &PreviewScrollUp,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.preview_scroll_row = self.preview_scroll_row.saturating_sub(6);
        self.preview_scroll
            .scroll_to_item(self.preview_scroll_row, ScrollStrategy::Top);
        cx.notify();
    }

    // Scroll the preview pane toward the bottom.
    fn on_preview_scroll_down(
        &mut self,
        _: &PreviewScrollDown,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.preview_lines.is_empty() {
            self.preview_scroll_row =
                (self.preview_scroll_row + 6).min(self.preview_lines.len() - 1);
            self.preview_scroll
                .scroll_to_item(self.preview_scroll_row, ScrollStrategy::Top);
            cx.notify();
        }
    }

    // Switch back to file search mode.
    fn on_switch_files(&mut self, _: &SwitchFiles, window: &mut Window, cx: &mut Context<Self>) {
        if self.view != SearchView::Files {
            self.view = SearchView::Files;
            self.switch_mode(window, cx);
        }
    }

    // Switch to live grep mode.
    fn on_switch_grep(&mut self, _: &SwitchGrep, window: &mut Window, cx: &mut Context<Self>) {
        if self.view != SearchView::Grep {
            self.view = SearchView::Grep;
            self.grep_mode = GrepMode::PlainText;
            self.switch_mode(window, cx);
        }
    }

    // Return the text field focus handle so the window can focus it on startup.
    pub fn text_field_focus_handle(&self, cx: &App) -> FocusHandle {
        self.text_field.focus_handle(cx)
    }

    // Render one results-pane row by `ResultRow` dispatch. Grep view gets
    // Zed-style visuals (header groups, per-match rows, padded separators);
    // the Files view keeps its flat file-row layout (files-mode rows are
    // `Match { m: 0 }` with no grep matches).
    fn render_result_row(
        &mut self,
        row_ix: usize,
        results_pane_width: f32,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        match self.rows.get(row_ix) {
            Some(ResultRow::Separator) => {
                // Zed-style group divider: 1px border line with 8px vertical
                // padding; no git edge bar and never selectable.
                let border_variant = cx.global::<AppTheme>().border_variant;
                div()
                    .w_full()
                    .py(px(8.0))
                    .child(div().w_full().h(px(1.0)).bg(rgba(border_variant)))
                    .into_any_element()
            }
            Some(ResultRow::Header(file)) => {
                self.render_header_row(row_ix, *file, results_pane_width, cx)
            }
            Some(ResultRow::Match { file, m }) if self.is_grep_view() => {
                self.render_grep_match_row(row_ix, *file, *m, cx)
            }
            Some(ResultRow::Match { file, .. }) => {
                self.render_file_row(row_ix, *file, results_pane_width, cx)
            }
            None => div().w_full().into_any_element(),
        }
    }

    // Multiselect checkbox for a result row: clicking toggles the row's
    // `SelectionKey` and (re)enters multiselect mode. Shared by the grep-match
    // and file row renderers, which drop it into their leading slot while
    // multiselect is active. `stop_propagation` keeps the row's own click
    // (cursor move / open) from also firing.
    fn checkbox_slot(
        &self,
        row_ix: usize,
        is_checked: bool,
        theme: &AppTheme,
        cx: &Context<Self>,
    ) -> Stateful<Div> {
        ui::checkbox(("checkbox", row_ix), is_checked, theme).on_click(cx.listener(
            move |this, _: &ClickEvent, _window, cx| {
                cx.stop_propagation();
                if this.toggle_key_at(row_ix) {
                    this.multi_select_mode = true;
                    cx.notify();
                }
            },
        ))
    }

    // Grep file-group header (~30px): chevron, 16px file icon, filename,
    // left-truncated directory. The git edge bar matches the group's match
    // rows so the colored edge reads as one continuous strip per file.
    // Background: `selected_row` only while collapsed AND cursor-selected
    // (expanded headers are not selectable).
    fn render_header_row(
        &mut self,
        row_ix: usize,
        file: usize,
        results_pane_width: f32,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        // Borrow the theme rather than cloning it per visible row — the clone
        // would deep-copy `syntax_styles` (a `Vec<(String, SyntaxStyle)>`) that
        // this row never reads. Only scalar color/size fields are touched.
        let theme = cx.global::<AppTheme>();
        let Some(item) = self.results.get(file) else {
            return div().w_full().into_any_element();
        };
        let is_collapsed = self.collapsed.contains(&item.absolute_path);
        let show_selected_bg = is_collapsed && row_ix == self.selected;
        let bar_color = git_status_bar_color(item.git_status.as_deref(), theme);
        let file_icon = theme::file_icon_for_path(&item.absolute_path);
        // Monospace approximation: a char is roughly 0.6 of the em. Header rows
        // always lead with a chevron slot on top of the base row chrome; the
        // left-truncated dir has no ellipsis fallback, so account for it or the
        // dir overflows the row.
        let char_px = theme.ui_font_size * 0.6;
        let dir_max_chars = layout::dir_max_chars(
            results_pane_width,
            char_px,
            item.file_name.chars().count(),
            layout::ROW_LEADING_SLOT,
        );
        let display_dir = truncate_start(item.dir.trim_matches('/'), dir_max_chars);

        div()
            .id(("row", row_ix))
            .w_full()
            .h(px(30.0))
            .flex()
            .items_center()
            // Unselected rows stay transparent over the root surface; only
            // the selected row paints (hover paints via the closure below).
            .when(show_selected_bg, |d| d.bg(rgba(theme.selected_row)))
            .hover(|s| s.bg(rgba(theme.hover_row)))
            .cursor_pointer()
            .on_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
                this.on_row_click(row_ix, event, window, cx);
            }))
            .child(ui::git_edge_bar(bar_color))
            .child(
                div()
                    .pl(px(10.0))
                    .pr(px(12.0))
                    .flex_1()
                    .min_w(px(0.0))
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .child(
                        // Disclosure chevron: down expanded / right collapsed.
                        // Click toggles this group; alt-click toggles all
                        // groups. The click stops propagation so the row's own
                        // click handler (cursor move / open) does not also fire.
                        ui::disclosure(("chevron", row_ix), !is_collapsed, theme).on_click(
                            cx.listener(move |this, event: &ClickEvent, _window, cx| {
                                cx.stop_propagation();
                                if event.modifiers().alt {
                                    this.toggle_all_folds(cx);
                                } else {
                                    this.toggle_group_fold(file, cx);
                                }
                            }),
                        ),
                    )
                    .child(render_file_icon(file_icon, theme.icon_muted))
                    .child(
                        div()
                            .text_xs()
                            .text_color(rgba(theme.text_primary))
                            .flex_shrink_0()
                            .child(item.file_name.clone()),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(rgba(theme.text_secondary))
                            .min_w(px(0.0))
                            .overflow_hidden()
                            .child(display_dir),
                    ),
            )
            .into_any_element()
    }

    // One grep match line (28px): right-aligned dimmed line-number gutter
    // sized for the widest visible line number, then the line's syntax spans
    // with the match ranges overlaid (match bg + bold), single line and
    // truncated. No count/✨ pills and no filename prefix — the group header
    // carries the file identity. While multiselect mode is active a checkbox
    // slot leads the row, checked from the row's per-match `SelectionKey`.
    fn render_grep_match_row(
        &mut self,
        row_ix: usize,
        file: usize,
        m: usize,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        // Borrow the theme rather than cloning it per visible row — the clone
        // would deep-copy `syntax_styles` (a `Vec<(String, SyntaxStyle)>`) that
        // this row never reads. Only scalar color/size fields are touched.
        let theme = cx.global::<AppTheme>();
        let Some(gm) = self
            .results
            .get(file)
            .and_then(|item| item.grep_matches.get(m))
        else {
            return div().w_full().into_any_element();
        };
        let item = &self.results[file];
        let is_selected = row_ix == self.selected;
        let show_checkbox = self.multi_select_mode;
        let is_checked = show_checkbox
            && rows::selection_key_for_row(&self.rows, row_ix, &self.results, self.is_grep_view())
                .is_some_and(|key| self.selection.contains(&key));
        let bar_color = git_status_bar_color(item.git_status.as_deref(), theme);
        // Gutter sized for the widest line number across the visible results
        // (cached by `rebuild_rows`), same 0.6-em char-width heuristic as the
        // preview gutter; `layout::gutter_width` includes the 8px gap.
        let char_px = theme.ui_font_size * 0.6;
        let gutter_w = px(layout::gutter_width(self.max_match_line as usize, char_px));
        // Muted gutter numbers: `text_secondary` (Zed's `text_muted`, the token
        // Zed uses for picker row line numbers) at ~50% opacity.
        let gutter_color = rgba(theme::with_alpha(theme.text_secondary, 0x80));
        let spans = match_row_spans(
            &gm.syntax_spans,
            &gm.line_content,
            &gm.byte_ranges,
            theme.text_primary,
            theme.match_highlight_bg,
        );
        let line_number = gm.line_number;

        div()
            .id(("row", row_ix))
            .w_full()
            .h(px(28.0))
            .flex()
            .items_center()
            // Unselected rows stay transparent over the root surface; only
            // the selected row paints (hover paints via the closure below).
            .when(is_selected, |d| d.bg(rgba(theme.selected_row)))
            .hover(|s| s.bg(rgba(theme.hover_row)))
            .cursor_pointer()
            .on_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
                this.on_row_click(row_ix, event, window, cx);
            }))
            // Same 3px edge strip as the header so the group's git color reads
            // as one continuous bar; transparent when clean.
            .child(ui::git_edge_bar(bar_color))
            .child(
                div()
                    .pl(px(10.0))
                    .pr(px(12.0))
                    .flex_1()
                    .min_w(px(0.0))
                    .flex()
                    .items_center()
                    .text_sm()
                    .when(show_checkbox, |d| {
                        // 6px matches Zed's ListItem start_slot gap
                        // (DynamicSpacing::Base06 at default density).
                        d.child(
                            self.checkbox_slot(row_ix, is_checked, theme, cx)
                                .mr(px(6.0)),
                        )
                    })
                    .child(
                        // Right-aligned line number; the 8px gap before the
                        // line text lives inside `gutter_w`.
                        div()
                            .w(gutter_w)
                            .flex_shrink_0()
                            .pr(px(layout::GUTTER_GAP))
                            .flex()
                            .justify_end()
                            .text_color(gutter_color)
                            .child(line_number.to_string()),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.0))
                            .overflow_hidden()
                            .flex()
                            .items_center()
                            .children(spans.into_iter().map(|span| {
                                div()
                                    .text_color(rgba(span.color))
                                    .when(span.bold, |d| d.font_weight(FontWeight::BOLD))
                                    .when(span.italic, |d| d.italic())
                                    .when(span.underline, |d| d.underline())
                                    .when(span.strikethrough, |d| d.line_through())
                                    .when_some(span.bg, |d, bg| d.bg(rgba(bg)))
                                    .child(span.text)
                            })),
                    ),
            )
            .into_any_element()
    }

    // Files-view row (28px): icon, fuzzy-tinted filename, shortened dir, ✨
    // frecency pill. While multiselect mode is active a checkbox slot leads
    // the row, checked from the row's per-file `SelectionKey`.
    fn render_file_row(
        &mut self,
        row_ix: usize,
        file: usize,
        results_pane_width: f32,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        // Borrow the theme rather than cloning it per visible row — the clone
        // would deep-copy `syntax_styles` (a `Vec<(String, SyntaxStyle)>`) that
        // this row never reads. Only scalar color/size fields are touched.
        let theme = cx.global::<AppTheme>();
        let Some(item) = self.results.get(file) else {
            return div().w_full().into_any_element();
        };
        let is_selected = row_ix == self.selected;
        let show_checkbox = self.multi_select_mode;
        let is_checked = show_checkbox
            && rows::selection_key_for_row(&self.rows, row_ix, &self.results, self.is_grep_view())
                .is_some_and(|key| self.selection.contains(&key));
        let badge_color = if is_selected {
            theme.text_primary
        } else {
            theme.text_secondary
        };
        // Monospace approximation: a char is roughly 0.6 of the em. The
        // multiselect checkbox slot leads the row only while multiselect is
        // active, so add its width to the chrome budget only then.
        let char_px = theme.ui_font_size * 0.6;
        let extra_chrome = if show_checkbox {
            layout::ROW_LEADING_SLOT
        } else {
            0.0
        };
        let path_max_chars = layout::dir_max_chars(
            results_pane_width,
            char_px,
            item.file_name.chars().count(),
            extra_chrome,
        );
        let display_dir = shorten_dir_for_row(&item.dir, path_max_chars);
        let bar_color = git_status_bar_color(item.git_status.as_deref(), theme);
        let file_icon = theme::file_icon_for_path(&item.absolute_path);

        div()
            .id(("row", row_ix))
            .w_full()
            .h(px(28.0))
            .flex()
            .items_center()
            // Unselected rows stay transparent over the root surface; only
            // the selected row paints (hover paints via the closure below).
            .when(is_selected, |d| d.bg(rgba(theme.selected_row)))
            .hover(|s| s.bg(rgba(theme.hover_row)))
            .cursor_pointer()
            .on_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
                this.on_row_click(row_ix, event, window, cx);
            }))
            // Full-row-height edge bar flush against the row's left edge; rows
            // without a git status keep the transparent 3px strip so text
            // alignment stays consistent.
            .child(ui::git_edge_bar(bar_color))
            .child(
                div()
                    .pl(px(10.0))
                    .pr(px(12.0))
                    .flex_1()
                    .min_w(px(0.0))
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .when(show_checkbox, |d| {
                        d.child(self.checkbox_slot(row_ix, is_checked, theme, cx))
                    })
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.0))
                            .overflow_hidden()
                            .flex()
                            .items_center()
                            .gap(px(8.0))
                            .text_sm()
                            .child(render_file_icon(file_icon, theme.icon_muted))
                            .child(div().flex_shrink_0().child(render_highlighted(
                                &item.file_name,
                                &item.match_ranges,
                                theme,
                            )))
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(rgba(theme.text_secondary))
                                    .min_w(px(0.0))
                                    .overflow_hidden()
                                    .child(display_dir),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_shrink_0()
                            .items_center()
                            .gap(px(6.0))
                            .when(item.frecency_score > 0, |this| {
                                this.child(
                                    div()
                                        .text_xs()
                                        .text_color(rgba(badge_color))
                                        .flex_shrink_0()
                                        .child(format!("\u{2728} {}", item.frecency_score)),
                                )
                            }),
                    ),
            )
            .into_any_element()
    }
}

impl Render for FffPicker {
    // Render the picker layout.
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let current_theme_version = theme::version();
        if self.theme_version != current_theme_version {
            self.theme_version = current_theme_version;
            // Match-row spans were highlighted under the theme active at search
            // time; recompute them in place so grep rows follow the new palette
            // without waiting for a fresh search (cheap — configs are cached).
            if self.is_grep_view() && !self.results.is_empty() {
                refresh_syntax_spans(Arc::make_mut(&mut self.results).as_mut_slice());
            }
            if self.selected_row_snapshot().is_some() {
                // The loaded spans bake the old theme's colors — force a full
                // re-highlight instead of the same-file re-center shortcut.
                self.preview_path = None;
                self.load_preview(cx);
            }
        }
        let theme = cx.global::<AppTheme>().clone();
        // Viewport-relative pane split: the preview gets a fixed width and the
        // results pane flexes into the rest. The window IS the modal, so the
        // viewport width is the modal width. A session drag position wins over
        // the `picker_pane_width` config override, which wins over the
        // default 50/50 results share.
        let modal_width = f32::from(window.viewport_size().width);
        // `layout::split` reserves the 1px divider, so results_w + divider +
        // preview_w == modal_width. The results panel flexes into the space the
        // preview and divider leave, which is exactly `results_w`.
        let split = effective_split(
            modal_width,
            self.session_results_width,
            theme.picker_pane_width,
        );
        let preview_pane_width = split.preview_w;
        // Computed once per frame and threaded into the row renderers so each
        // row's dir-truncation budget shares one pane width instead of
        // recomputing the split per row.
        let results_pane_width = split.results_w;
        let ui_font_family = theme.ui_font_family.clone();
        let buffer_font_family = theme.buffer_font_family.clone();
        let ui_font_size = px(theme.ui_font_size);
        let buffer_font_size = px(theme.buffer_font_size);
        let preview_line_height = px(theme.buffer_font_size);
        let results = self.results.clone();
        let preview_lines = self.preview_lines.clone();
        // Line-number gutter: sized for the largest visible line number
        // (1-based `preview_start_line` + window length), using the monospace
        // 0.6-em advance-width heuristic on the buffer font. The 8px gap
        // before the code is included in `layout::gutter_width`.
        let preview_start_line = self.preview_start_line;
        let gutter_char_px = theme.buffer_font_size * 0.6;
        // The largest rendered line number is `preview_start_line + len - 1`;
        // passing `+ len` over-provisions the gutter by one digit-width at most.
        // Intentional — it never clips at a digit rollover, so keep the `+ len`.
        let gutter_width = px(layout::gutter_width(
            preview_start_line + preview_lines.len(),
            gutter_char_px,
        ));
        // Zed gutter tokens: per-row cell background plus the normal vs
        // match-line number colors (`gutter_number_color` decides per row).
        let gutter_bg = theme.editor_gutter_bg;
        let line_number_color = theme.editor_line_number;
        let active_line_number_color = theme.editor_active_line_number;
        let active_line_bg = theme.active_line_bg;
        let selected = self.selected;
        let scan_done = self.scan_done;
        let total_files = self.total_files;
        let total_matched = self.total_matched;
        let indexed_count = self.indexed_count;
        let selected_count = self.selection.len();
        let preview_scroll = self.preview_scroll.clone();
        // Built up-front: the results-list `.when` closure below consumes
        // `cx`, and the divider (which needs this listener) renders after it.
        // gpui consumes the pending click once a drag starts (>2px movement),
        // so drag and double-click reset coexist safely on the hit strip.
        let divider_double_click = cx.listener(|this, event: &ClickEvent, window, cx| {
            if event.click_count() == 2 {
                // Reset to the 50/50 default (not the config override).
                let modal_w = f32::from(window.viewport_size().width);
                this.session_results_width = Some(layout::reset_split(modal_w).results_w);
                store_session_results_width(this.session_results_width);
                this.recenter_preview();
                cx.notify();
            }
        });
        let selected_path = self
            .selected_row_snapshot()
            .map(|item| item.absolute_path.clone());
        trace!(
            scan_done,
            results = results.len(),
            selected,
            preview_lines = preview_lines.len(),
            selected_count,
            view = ?self.view,
            query = %self.query,
            status_message = ?self.status_message,
            "rendering picker"
        );
        let preview_placeholder = if !scan_done {
            ""
        } else if self.preview_loading_visible {
            "Loading\u{2026}"
        } else if selected_path.is_some() && preview_lines.is_empty() {
            "No preview"
        } else if self.view == SearchView::Grep && self.query.trim().is_empty() {
            "Type to grep"
        } else {
            "No preview"
        };

        let status_text = status_left_text(
            self.view,
            self.grep_mode,
            self.status_message.as_deref(),
            scan_done,
            indexed_count,
            total_files,
            total_matched,
            selected_count,
            &results,
        );

        div()
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(Self::on_quit))
            .on_action(cx.listener(Self::on_open_selected))
            .on_action(cx.listener(Self::on_select_next))
            .on_action(cx.listener(Self::on_select_prev))
            .on_action(cx.listener(Self::on_toggle_selected))
            .on_action(cx.listener(Self::on_toggle_select_all))
            .on_action(cx.listener(Self::on_toggle_multi_select_mode))
            .on_action(cx.listener(Self::on_toggle_fold))
            .on_action(cx.listener(Self::on_toggle_fold_all))
            .on_action(cx.listener(Self::on_shift_tab))
            .on_action(cx.listener(Self::on_cycle_grep_mode))
            .on_action(cx.listener(Self::on_history_prev))
            .on_action(cx.listener(Self::on_history_next))
            .on_action(cx.listener(Self::on_preview_scroll_up))
            .on_action(cx.listener(Self::on_preview_scroll_down))
            .on_action(cx.listener(Self::on_switch_files))
            .on_action(cx.listener(Self::on_switch_grep))
            .size_full()
            .flex()
            .flex_row()
            .overflow_hidden()
            // Divider drag: fires capture-phase for every mouse move
            // window-wide while a DividerDrag is active (mouse-up ends it
            // automatically). `e.bounds` is the root's own bounds — the full
            // window from x=0 — so the desired results width is just the
            // cursor x relative to the window's left edge and `modal_w` is
            // the window width; no stored drag-start state needed.
            .on_drag_move(
                cx.listener(|this, e: &DragMoveEvent<DividerDrag>, _window, cx| {
                    let results_w = f32::from(e.event.position.x - e.bounds.origin.x);
                    let modal_w = f32::from(e.bounds.size.width);
                    this.session_results_width =
                        Some(layout::clamp_drag(results_w, modal_w).results_w);
                    store_session_results_width(this.session_results_width);
                    this.recenter_preview();
                    cx.notify();
                }),
            )
            .bg(rgba(theme.bg))
            .text_color(rgba(theme.text_primary))
            .text_size(ui_font_size)
            .when_some(ui_font_family.clone(), |this, family| {
                this.font_family(family)
            })
            .child(
                // Left column (Zed `render_with_preview_right` parity): the
                // search row, results area, and status bar stack vertically;
                // the divider and full-height preview sit to the right as
                // root children. `flex_1` (rather than an explicit
                // `w(results_pane_width)`) lets the flex engine absorb any
                // sub-pixel rounding — the divider and preview widths are
                // fixed, so this column resolves to exactly
                // `results_pane_width` per the `layout::split` invariant
                // (`results_w + 1 + preview_w == modal_w`).
                div()
                    .h_full()
                    .flex_1()
                    .min_w(px(0.0))
                    .flex()
                    .flex_col()
                    .overflow_hidden()
                    .child(
                        div()
                            .w_full()
                            .h(px(36.0))
                            .flex_shrink_0()
                            .px(px(10.0))
                            .flex()
                            .items_center()
                            .gap(px(8.0))
                            .border_b_1()
                            .border_color(rgba(theme.border_variant))
                            .text_size(buffer_font_size)
                            .when_some(buffer_font_family.clone(), |this, family| {
                                this.font_family(family)
                            })
                            .child(
                                div()
                                    .text_color(rgba(theme.match_highlight))
                                    .text_sm()
                                    .child("🪿"),
                            )
                            .child(
                                // Fill the 36px row height so the field's `h_full` root
                                // resolves against a definite height and its mouse
                                // handlers cover the whole row, not just the 18px text.
                                div()
                                    .flex_1()
                                    .w_full()
                                    .h_full()
                                    .min_w(px(0.0))
                                    .child(self.text_field.clone()),
                            )
                            .child(
                                // Multiselect-mode toggle (Zed parity): always
                                // visible in both views, accent-tinted while the
                                // mode is on. Same path as cmd-shift-s. The click
                                // stops propagation so root handlers don't fire;
                                // a Stateful<Div> is non-focusable, so the query
                                // input keeps focus (no focus-lost dismiss).
                                ui::icon_button(
                                    "multi-select-toggle",
                                    "icons/file_multiple.svg",
                                    self.multi_select_mode,
                                    &theme,
                                )
                                .on_click(cx.listener(
                                    |this, _: &ClickEvent, window, cx| {
                                        cx.stop_propagation();
                                        this.on_toggle_multi_select_mode(
                                            &ToggleMultiSelectMode,
                                            window,
                                            cx,
                                        );
                                    },
                                )),
                            ),
                    )
                    .child(
                        // Results area: fills the left column between the
                        // fixed-height search row and status bar.
                        div()
                            .flex_1()
                            .w_full()
                            .min_h(px(0.0))
                            .flex()
                            .flex_col()
                            .overflow_hidden()
                            .when(!scan_done, |this| {
                                let label = if indexed_count > 0 {
                                    format!("Indexing {indexed_count} files")
                                } else {
                                    "Indexing".to_string()
                                };

                                this.child(
                                    div()
                                        .flex_1()
                                        .size_full()
                                        .flex()
                                        .items_center()
                                        .justify_center()
                                        .text_sm()
                                        .text_color(rgba(theme.text_dim))
                                        .child(label),
                                )
                            })
                            .when(scan_done && results.is_empty(), |this| {
                                if self.view == SearchView::Grep && self.query.trim().is_empty() {
                                    let hint_row = |key: &'static str, desc: &'static str| {
                                        div()
                                            .flex()
                                            .gap(px(8.0))
                                            .text_xs()
                                            .text_color(rgba(theme.text_dim))
                                            .child(div().w(px(140.0)).child(key))
                                            .child(div().child(desc))
                                    };
                                    this.child(
                                        div()
                                            .flex_1()
                                            .size_full()
                                            .px(px(20.0))
                                            .pt(px(20.0))
                                            .flex()
                                            .flex_col()
                                            .gap(px(4.0))
                                            .child(
                                                div()
                                                    .text_sm()
                                                    .text_color(rgba(theme.text_dim))
                                                    .child(
                                                        "Start typing to search file contents...",
                                                    ),
                                            )
                                            .child(div().h(px(8.0)))
                                            .child(
                                                div()
                                                    .text_xs()
                                                    .text_color(rgba(theme.text_secondary))
                                                    .child("Tips:"),
                                            )
                                            .child(hint_row(
                                                "\"pattern *.rs\"",
                                                "search only in Rust files",
                                            ))
                                            .child(hint_row(
                                                "\"pattern /src/\"",
                                                "limit search to src/ directory",
                                            ))
                                            .child(hint_row(
                                                "\"!test pattern\"",
                                                "exclude test files",
                                            )),
                                    )
                                } else {
                                    this.child(
                                        div()
                                            .flex_1()
                                            .size_full()
                                            .flex()
                                            .items_center()
                                            .justify_center()
                                            .text_sm()
                                            .text_color(rgba(theme.text_dim))
                                            .child("No files matched"),
                                    )
                                }
                            })
                            .when(scan_done && !results.is_empty(), |this| {
                                this.child(
                                    div()
                                        .w_full()
                                        .h_full()
                                        .flex()
                                        .flex_col()
                                        .overflow_hidden()
                                        .child(
                                            list(
                                                self.results_list.clone(),
                                                cx.processor(
                                                    move |this, ix: usize, _window, cx| {
                                                        this.render_result_row(
                                                            ix,
                                                            results_pane_width,
                                                            cx,
                                                        )
                                                    },
                                                ),
                                            )
                                            .flex_1()
                                            .w_full(),
                                        ),
                                )
                            }),
                    )
                    .child(
                        div()
                            .w_full()
                            .h(px(28.0))
                            .flex_shrink_0()
                            .px(px(12.0))
                            .flex()
                            .items_center()
                            .justify_between()
                            // No bg: the status bar is transparent over the
                            // root surface (Zed-footer style). `min_w(0)` +
                            // `truncate()` on both children let narrow drags
                            // degrade by clipping text instead of forcing the
                            // column wider.
                            .border_t_1()
                            .border_color(rgba(theme.border_variant))
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(rgba(theme.text_dim))
                                    .min_w(px(0.0))
                                    .truncate()
                                    .child(status_text),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(rgba(theme.text_dim))
                                    .min_w(px(0.0))
                                    .truncate()
                                    .child(status_right_hints(self.view)),
                            ),
                    ),
            )
            .child(
                // 1px visible divider line; the invisible 6px hit
                // strip is absolutely positioned over it (centered, so
                // it overhangs ~2.5px into each pane) to make the
                // divider grabbable without widening the line.
                div()
                    .w(px(1.0))
                    .h_full()
                    .bg(rgba(theme.border_variant))
                    .flex_shrink_0()
                    .relative()
                    .child(
                        div()
                            .id("divider-hit")
                            .absolute()
                            .left(px(-2.5))
                            .top_0()
                            .w(px(6.0))
                            .h_full()
                            .cursor_col_resize()
                            // EmptyView keeps the drag ghost invisible;
                            // the cursor stays col-resize during the
                            // drag via this element's own style.
                            .on_drag(DividerDrag, |_, _, _, cx| cx.new(|_| EmptyView))
                            // Double-click resets the split; the
                            // listener is prebuilt above because `cx`
                            // is consumed by the results-list closure.
                            .on_click(divider_double_click),
                    ),
            )
            .child(
                div()
                    .w(px(preview_pane_width))
                    .h_full()
                    .flex_shrink_0()
                    .flex()
                    .flex_col()
                    .bg(rgba(theme.preview_bg))
                    .text_size(buffer_font_size)
                    .when_some(buffer_font_family.clone(), |this, family| {
                        this.font_family(family)
                    })
                    .overflow_hidden()
                    .when(preview_lines.is_empty(), |this| {
                        this.child(
                            div()
                                .size_full()
                                .flex()
                                .items_center()
                                .justify_center()
                                .text_xs()
                                .text_color(rgba(theme.text_dim))
                                .child(preview_placeholder),
                        )
                    })
                    .when(!preview_lines.is_empty(), |this| {
                        this.child(
                            uniform_list(
                                "preview",
                                preview_lines.len(),
                                move |range, _window, _cx| {
                                    range
                                        .map(|i| {
                                            let line = &preview_lines[i];
                                            // Zed-style match emphasis: the gutter
                                            // keeps its own background while the
                                            // active-line tint covers only the
                                            // text area.
                                            let has_match = preview::line_has_match(line);
                                            div()
                                                .id(("pl", i))
                                                .h(preview_line_height)
                                                .flex()
                                                .items_center()
                                                .child(
                                                    // Per-row gutter cell painted
                                                    // `editor_gutter_bg` (reads as a
                                                    // full-height column across the
                                                    // virtualized rows). Absorbs the
                                                    // pane's 8px left padding so the
                                                    // column reaches the pane edge;
                                                    // the 8px gap before the code
                                                    // lives inside `gutter_width`.
                                                    div()
                                                        .w(gutter_width + px(8.0))
                                                        .h_full()
                                                        .flex_shrink_0()
                                                        .bg(rgba(gutter_bg))
                                                        .pl(px(8.0))
                                                        .pr(px(layout::GUTTER_GAP))
                                                        .flex()
                                                        .items_center()
                                                        .justify_end()
                                                        .text_xs()
                                                        .line_height(preview_line_height)
                                                        .text_color(rgba(gutter_number_color(
                                                            has_match,
                                                            line_number_color,
                                                            active_line_number_color,
                                                        )))
                                                        .child(
                                                            (preview_start_line + i).to_string(),
                                                        ),
                                                )
                                                .child(
                                                    // Text area: the active-line
                                                    // wash blends with real alpha
                                                    // over `preview_bg` here only,
                                                    // never over the gutter cell.
                                                    div()
                                                        .flex_1()
                                                        .h_full()
                                                        .pr(px(8.0))
                                                        .flex()
                                                        .items_center()
                                                        .when(has_match, |d| {
                                                            d.bg(rgba(active_line_bg))
                                                        })
                                                        .children(line.spans.iter().map(|span| {
                                                            // Matched substrings
                                                            // carry a flat match
                                                            // background (bold
                                                            // comes from
                                                            // `span.bold`, set by
                                                            // `overlay_match_ranges`).
                                                            div()
                                                                .text_xs()
                                                                .line_height(preview_line_height)
                                                                .text_color(rgba(span.color))
                                                                .when(span.bold, |d| {
                                                                    d.font_weight(FontWeight::BOLD)
                                                                })
                                                                .when(span.italic, |d| d.italic())
                                                                .when(span.underline, |d| {
                                                                    d.underline()
                                                                })
                                                                .when(span.strikethrough, |d| {
                                                                    d.line_through()
                                                                })
                                                                .when_some(
                                                                    span.bg,
                                                                    |d, bg_color| {
                                                                        d.bg(rgba(bg_color))
                                                                    },
                                                                )
                                                                .child(span.text.clone())
                                                        })),
                                                )
                                        })
                                        .collect()
                                },
                            )
                            .flex_1()
                            .w_full()
                            .track_scroll(&preview_scroll),
                        )
                    }),
            )
    }
}

#[cfg(test)]
mod tests {
    // The `segment_matches` tests pass single-range slice literals like `&[1..3]`
    // as intentional match-range inputs; that is not a mistaken range-vec init.
    #![allow(clippy::single_range_in_vec_init)]
    // Deliberately not `use super::*`: picker.rs glob-imports `gpui::*`, whose
    // `test` attribute macro would shadow the built-in `#[test]`.
    use super::{
        FileItemSnapshot, FuzzyQuery, GrepMatchLine, GrepMode, SESSION_WIDTH_UNSET, SearchView,
        build_file_query, cursor_open, decode_session_width, effective_split, encode_session_width,
        find_match_ranges, git_status_bar_color, goto_for_key, grep_match_line,
        gutter_number_color, is_useful_fuzzy_token, match_goto, match_row_spans,
        opens_for_selection, path_is_excluded, preview_target, recenter_scroll_row,
        refresh_syntax_spans, segment_matches, should_parse_file_constraints, slice_spans,
        status_left_text, status_right_hints, truncate_start,
    };
    use crate::layout;
    use crate::preview::{HighlightedSpan, MAX_PREVIEW_LINES};
    use crate::rows;
    use crate::theme::AppTheme;
    use std::collections::{BTreeSet, HashSet};
    use std::ops::Range;
    use std::path::{Path, PathBuf};

    // A theme whose git tokens are all distinct so the mapping is observable.
    fn theme_with_distinct_git_tokens() -> AppTheme {
        AppTheme {
            git_created: 0x000001,
            git_modified: 0x000002,
            git_deleted: 0x000003,
            git_conflict: 0x000004,
            git_renamed: 0x000005,
            git_untracked: 0x000006,
            git_ignored: 0x000007,
            ..AppTheme::default()
        }
    }

    #[test]
    fn git_status_modified_maps_to_git_modified() {
        let theme = theme_with_distinct_git_tokens();
        assert_eq!(
            git_status_bar_color(Some("modified"), &theme),
            Some(theme.git_modified)
        );
    }

    #[test]
    fn git_status_staged_new_and_staged_modified_map_to_git_created() {
        let theme = theme_with_distinct_git_tokens();
        assert_eq!(
            git_status_bar_color(Some("staged_new"), &theme),
            Some(theme.git_created)
        );
        assert_eq!(
            git_status_bar_color(Some("staged_modified"), &theme),
            Some(theme.git_created)
        );
    }

    #[test]
    fn git_status_deleted_and_staged_deleted_map_to_git_deleted() {
        let theme = theme_with_distinct_git_tokens();
        assert_eq!(
            git_status_bar_color(Some("deleted"), &theme),
            Some(theme.git_deleted)
        );
        assert_eq!(
            git_status_bar_color(Some("staged_deleted"), &theme),
            Some(theme.git_deleted)
        );
    }

    #[test]
    fn git_status_renamed_maps_to_git_renamed() {
        let theme = theme_with_distinct_git_tokens();
        assert_eq!(
            git_status_bar_color(Some("renamed"), &theme),
            Some(theme.git_renamed)
        );
    }

    #[test]
    fn git_status_untracked_maps_to_distinct_git_untracked() {
        let theme = theme_with_distinct_git_tokens();
        assert_eq!(
            git_status_bar_color(Some("untracked"), &theme),
            Some(theme.git_untracked)
        );
        // Untracked must stay distinct from created.
        assert_ne!(theme.git_untracked, theme.git_created);
    }

    #[test]
    fn git_status_ignored_maps_to_git_ignored() {
        let theme = theme_with_distinct_git_tokens();
        assert_eq!(
            git_status_bar_color(Some("ignored"), &theme),
            Some(theme.git_ignored)
        );
    }

    // fff-search does not emit "conflict" today; the mapping handles it
    // defensively should the crate start doing so.
    #[test]
    fn git_status_conflict_maps_to_git_conflict() {
        let theme = theme_with_distinct_git_tokens();
        assert_eq!(
            git_status_bar_color(Some("conflict"), &theme),
            Some(theme.git_conflict)
        );
    }

    #[test]
    fn git_status_clean_and_none_render_no_bar() {
        let theme = theme_with_distinct_git_tokens();
        assert_eq!(git_status_bar_color(Some("clean"), &theme), None);
        assert_eq!(git_status_bar_color(None, &theme), None);
    }

    #[test]
    fn git_status_unknown_string_falls_back_to_git_ignored() {
        let theme = theme_with_distinct_git_tokens();
        assert_eq!(
            git_status_bar_color(Some("something_new"), &theme),
            Some(theme.git_ignored)
        );
    }

    // effective_split: session drag value -> config override -> 50/50 default.

    #[test]
    fn effective_split_session_value_wins_over_config() {
        // The 1px divider is reserved, so preview == 1000 - 1 - results.
        let s = effective_split(1000.0, Some(600.0), Some(400.0));
        assert!((s.results_w - 600.0).abs() < 0.01);
        assert!((s.preview_w - 399.0).abs() < 0.01);
    }

    #[test]
    fn effective_split_config_wins_over_default_without_session() {
        let s = effective_split(1000.0, None, Some(400.0));
        assert!((s.results_w - 400.0).abs() < 0.01);
        assert!((s.preview_w - 599.0).abs() < 0.01);
    }

    #[test]
    fn effective_split_defaults_to_fifty_fifty() {
        // 50/50 of the 999px that remains after reserving the 1px divider.
        let s = effective_split(1000.0, None, None);
        assert!((s.results_w - 499.5).abs() < 0.01);
        assert!((s.preview_w - 499.5).abs() < 0.01);
    }

    #[test]
    fn effective_split_clamps_session_value_to_pane_minimums() {
        // Dragged too far left: results floor at 280px.
        let low = effective_split(1000.0, Some(0.0), None);
        assert!((low.results_w - layout::MIN_RESULTS_WIDTH).abs() < 0.01);
        // Dragged too far right: preview floor at 128px.
        let high = effective_split(1000.0, Some(10_000.0), None);
        assert!((high.preview_w - layout::MIN_PREVIEW_WIDTH).abs() < 0.01);
    }

    // Double-click stores `reset_split(modal_w).results_w` as the session
    // value; that must land on the 50/50 default even when a config override
    // is present (reset goes to the default, not the config value).
    #[test]
    fn double_click_reset_value_restores_default_over_config() {
        let reset_w = layout::reset_split(1000.0).results_w;
        let s = effective_split(1000.0, Some(reset_w), Some(400.0));
        assert!((s.results_w - 499.5).abs() < 0.01);
        assert!((s.preview_w - 499.5).abs() < 0.01);
    }

    // Session divider-width store: encode/decode round trip. The atomic global
    // is exercised indirectly; these cover the pure seed/writeback codec so a
    // drag value survives the round trip and None maps to the unset sentinel.

    #[test]
    fn session_width_round_trips_finite_value() {
        assert_eq!(
            decode_session_width(encode_session_width(Some(512.0))),
            Some(512.0)
        );
        assert_eq!(
            decode_session_width(encode_session_width(Some(280.5))),
            Some(280.5)
        );
    }

    #[test]
    fn session_width_none_maps_to_unset_sentinel() {
        assert_eq!(encode_session_width(None), SESSION_WIDTH_UNSET);
        assert_eq!(decode_session_width(SESSION_WIDTH_UNSET), None);
        assert_eq!(decode_session_width(encode_session_width(None)), None);
    }

    #[test]
    fn session_width_non_finite_decodes_to_none() {
        // A stored non-finite value (e.g. from a corrupt bit pattern) is unset.
        assert_eq!(encode_session_width(Some(f32::NAN)), SESSION_WIDTH_UNSET);
        assert_eq!(
            encode_session_width(Some(f32::INFINITY)),
            SESSION_WIDTH_UNSET
        );
        assert_eq!(decode_session_width(f32::INFINITY.to_bits()), None);
    }

    // segment_matches: text + ranges -> ordered (chunk, is_match) segments.

    #[test]
    fn segment_matches_basic_case() {
        assert_eq!(
            segment_matches("hello", &[1..3]),
            vec![("h", false), ("el", true), ("lo", false)],
        );
    }

    #[test]
    fn segment_matches_orders_out_of_order_ranges() {
        assert_eq!(
            segment_matches("hello", &[3..5, 1..3]),
            vec![("h", false), ("el", true), ("lo", true)],
        );
    }

    #[test]
    fn segment_matches_drops_overlapping_ranges() {
        // 2..4 starts inside the already-consumed 1..3, so it is dropped.
        assert_eq!(
            segment_matches("hello", &[1..3, 2..4]),
            vec![("h", false), ("el", true), ("lo", false)],
        );
    }

    #[test]
    fn segment_matches_clamps_to_char_boundaries() {
        // "aé": a=byte 0, é=bytes 1..3. A range ending mid-'é' grows to include
        // the whole char instead of slicing it (which would panic).
        assert_eq!(
            segment_matches("aé", &[1..2]),
            vec![("a", false), ("é", true)],
        );
        // A range starting mid-char shrinks its start left to the boundary.
        assert_eq!(segment_matches("é", &[1..2]), vec![("é", true)]);
    }

    #[test]
    fn segment_matches_empty_ranges_yield_single_chunk() {
        assert_eq!(segment_matches("hello", &[]), vec![("hello", false)]);
        // A zero-length range clamps away and leaves the text unmatched.
        assert_eq!(segment_matches("hello", &[2..2]), vec![("hello", false)]);
    }

    #[test]
    fn segment_matches_range_past_end_is_clamped() {
        assert_eq!(
            segment_matches("hi", &[1..10]),
            vec![("h", false), ("i", true)],
        );
        // A range entirely past the end clamps away to nothing.
        assert_eq!(segment_matches("hi", &[5..10]), vec![("hi", false)]);
    }

    #[test]
    fn segment_matches_empty_text_yields_no_segments() {
        assert_eq!(segment_matches("", &[0..3]), Vec::<(&str, bool)>::new());
    }

    // find_match_ranges: case-insensitive subsequence match returning the byte
    // ranges of each consecutive run of matched query chars (Files-view fuzzy
    // highlight). No full subsequence match yields no ranges.

    #[test]
    fn find_match_ranges_contiguous_prefix() {
        assert_eq!(find_match_ranges("main", "main.rs"), vec![0..4]);
    }

    #[test]
    fn find_match_ranges_subsequence_splits_into_runs() {
        // "mrs" in "main.rs": 'm' at 0, then 'r','s' contiguously at 5..7.
        assert_eq!(find_match_ranges("mrs", "main.rs"), vec![0..1, 5..7]);
    }

    #[test]
    fn find_match_ranges_is_case_insensitive() {
        assert_eq!(find_match_ranges("MAIN", "main.rs"), vec![0..4]);
        assert_eq!(find_match_ranges("main", "MAIN.RS"), vec![0..4]);
    }

    #[test]
    fn find_match_ranges_no_match_yields_empty() {
        // 'z' never appears, so the query is not a subsequence: no ranges.
        assert_eq!(
            find_match_ranges("maz", "main.rs"),
            Vec::<Range<usize>>::new()
        );
    }

    #[test]
    fn find_match_ranges_empty_query_yields_empty() {
        assert_eq!(find_match_ranges("", "main.rs"), Vec::<Range<usize>>::new());
        // Whitespace-only trims to empty.
        assert_eq!(
            find_match_ranges("   ", "main.rs"),
            Vec::<Range<usize>>::new()
        );
    }

    #[test]
    fn find_match_ranges_multibyte_lands_on_char_boundaries() {
        // 'é' occupies bytes 3..5 in "café"; the range must cover the whole
        // char so downstream slicing can never split it mid-byte.
        assert_eq!(find_match_ranges("é", "café"), vec![3..5]);
        // Uppercase query still matches the lowercase multibyte char.
        assert_eq!(find_match_ranges("É", "café"), vec![3..5]);
        for range in find_match_ranges("cé", "café") {
            assert!("café".is_char_boundary(range.start));
            assert!("café".is_char_boundary(range.end));
        }
    }

    // should_parse_file_constraints: route filename/path-shaped queries to the
    // constraint parser, keep code-shaped queries on the fast fuzzy path.

    #[test]
    fn should_parse_file_constraints_keeps_code_shaped_queries_fuzzy() {
        assert!(!should_parse_file_constraints("struct Data {"));
        assert!(!should_parse_file_constraints("fn main"));
        assert!(!should_parse_file_constraints(""));
    }

    #[test]
    fn should_parse_file_constraints_detects_filename_and_path_filters() {
        assert!(should_parse_file_constraints("main.rs")); // has '.'
        assert!(should_parse_file_constraints("src/foo")); // has '/'
        assert!(should_parse_file_constraints("*.toml")); // has '.'
        assert!(should_parse_file_constraints("type:rust")); // has ':'
        assert!(should_parse_file_constraints(".hidden")); // starts with '.'
        // A single filename-shaped token among plain words still trips it.
        assert!(should_parse_file_constraints("find main.rs"));
    }

    // is_useful_fuzzy_token: drop punctuation-only crumbs, keep tokens with any
    // alphanumeric so code-shaped queries don't demand impossible extra matches.

    #[test]
    fn is_useful_fuzzy_token_filters_punctuation_only() {
        assert!(is_useful_fuzzy_token("main"));
        assert!(is_useful_fuzzy_token("a1"));
        assert!(!is_useful_fuzzy_token("{"));
        assert!(!is_useful_fuzzy_token("}"));
        assert!(!is_useful_fuzzy_token("()"));
        assert!(!is_useful_fuzzy_token(""));
        // Non-ASCII letters are not ASCII-alphanumeric, so a bare accented word
        // is dropped (documented limitation of the ASCII check).
        assert!(!is_useful_fuzzy_token("é"));
    }

    // build_file_query: empty -> Empty; code-shaped -> plain fuzzy with
    // punctuation crumbs dropped and no constraints; filename/path-shaped ->
    // constraint parser.

    #[test]
    fn build_file_query_empty_input_is_empty_fuzzy() {
        let q = build_file_query("");
        assert_eq!(q.fuzzy_query, FuzzyQuery::Empty);
        assert!(q.constraints.is_empty());
        assert_eq!(q.raw_query, "");
        // Whitespace-only trims to the same empty query.
        let q = build_file_query("   ");
        assert_eq!(q.fuzzy_query, FuzzyQuery::Empty);
        assert_eq!(q.raw_query, "");
    }

    #[test]
    fn build_file_query_single_token_is_fuzzy_text() {
        let q = build_file_query("main");
        assert_eq!(q.fuzzy_query, FuzzyQuery::Text("main"));
        assert!(q.constraints.is_empty());
    }

    #[test]
    fn build_file_query_code_shaped_drops_punctuation_crumbs() {
        // "struct Data {" stays fuzzy (no '.'/'/'/'/':'); the lone "{" is dropped.
        let q = build_file_query("struct Data {");
        assert_eq!(q.fuzzy_query, FuzzyQuery::Parts(vec!["struct", "Data"]));
        assert!(q.constraints.is_empty());
    }

    #[test]
    fn build_file_query_punctuation_only_code_is_empty_fuzzy() {
        // Every token is punctuation-only, so all are filtered and the fuzzy
        // query collapses to Empty rather than demanding impossible matches.
        let q = build_file_query("{ } ( )");
        assert_eq!(q.fuzzy_query, FuzzyQuery::Empty);
        assert!(q.constraints.is_empty());
    }

    #[test]
    fn build_file_query_filename_shaped_routes_to_constraint_parser() {
        // The constraint parser produces at least one constraint for these,
        // proving they took the parser path rather than the plain fuzzy path.
        let ext = build_file_query("*.toml");
        assert_eq!(ext.raw_query, "*.toml");
        assert!(!ext.constraints.is_empty());

        let typ = build_file_query("type:rust");
        assert_eq!(typ.raw_query, "type:rust");
        assert!(!typ.constraints.is_empty());
    }

    // path_is_excluded: privacy-relevant exclusion — component-wise prefix so a
    // nested path under an excluded dir is excluded but a shared-prefix sibling
    // is not.

    #[test]
    fn path_is_excluded_matches_nested_and_the_dir_itself() {
        let excluded = vec![PathBuf::from("/proj/target")];
        assert!(path_is_excluded(Path::new("/proj/target"), &excluded));
        assert!(path_is_excluded(
            Path::new("/proj/target/debug/app"),
            &excluded
        ));
    }

    #[test]
    fn path_is_excluded_ignores_unrelated_and_shared_prefix_siblings() {
        let excluded = vec![PathBuf::from("/proj/target")];
        assert!(!path_is_excluded(Path::new("/proj/src/main.rs"), &excluded));
        // Component-wise: "target-foo" is not the "target" component.
        assert!(!path_is_excluded(
            Path::new("/proj/target-foo/x"),
            &excluded
        ));
    }

    #[test]
    fn path_is_excluded_empty_list_never_excludes() {
        assert!(!path_is_excluded(Path::new("/proj/target"), &[]));
    }

    #[test]
    fn path_is_excluded_matches_relative_entries() {
        let excluded = vec![PathBuf::from("node_modules"), PathBuf::from(".git")];
        assert!(path_is_excluded(
            Path::new("node_modules/pkg/index.js"),
            &excluded
        ));
        assert!(path_is_excluded(Path::new(".git/HEAD"), &excluded));
        assert!(!path_is_excluded(Path::new("src/lib.rs"), &excluded));
    }

    // grep_match_line: engine grep match data -> per-line snapshot with col
    // (0-based byte offset, identity only) and per-line syntax spans.

    #[test]
    fn grep_match_line_carries_line_col_and_byte_ranges() {
        let m = grep_match_line(
            Path::new("src/main.rs"),
            42,
            4,
            "let x = 1;",
            &[(4, 5), (8, 9)],
        );
        assert_eq!(m.line_number, 42);
        assert_eq!(m.col, 4);
        assert_eq!(m.line_content, "let x = 1;");
        assert_eq!(m.byte_ranges, vec![(4, 5), (8, 9)]);
    }

    #[test]
    fn grep_match_line_syntax_spans_concat_back_to_line() {
        let line = "let x = \"s\";";
        let m = grep_match_line(Path::new("src/main.rs"), 1, 0, line, &[(0, 3)]);
        let joined: String = m.syntax_spans.iter().map(|s| s.text.as_str()).collect();
        assert_eq!(joined, line);
    }

    #[test]
    fn grep_match_line_unknown_extension_yields_one_plain_span() {
        let line = "some plain text";
        let m = grep_match_line(Path::new("notes.xyz"), 7, 5, line, &[(5, 10)]);
        assert_eq!(m.syntax_spans.len(), 1);
        assert_eq!(m.syntax_spans[0].text, line);
    }

    #[test]
    fn grep_match_line_empty_line_yields_no_syntax_spans() {
        let m = grep_match_line(Path::new("src/main.rs"), 3, 0, "", &[]);
        assert!(m.syntax_spans.is_empty());
        assert!(m.byte_ranges.is_empty());
        assert_eq!(m.line_content, "");
    }

    // refresh_syntax_spans: after a live theme change the render guard
    // recomputes each grep match's baked-in spans from its stored line content.

    #[test]
    fn refresh_syntax_spans_recomputes_stale_match_rows() {
        // `gm` leaves `syntax_spans` empty, standing in for spans that went
        // stale after a theme change. Refreshing repopulates them from the
        // current resolver output for the item's own path + line text.
        let line = "let x = 1;";
        let mut items = vec![snap("/proj/main.rs", vec![gm(1, 0, line, &[(0, 3)])])];
        assert!(items[0].grep_matches[0].syntax_spans.is_empty());

        refresh_syntax_spans(&mut items);

        let expected = crate::preview::highlight_single_line(Path::new("/proj/main.rs"), line);
        let got = &items[0].grep_matches[0].syntax_spans;
        assert!(!got.is_empty(), "stale empty spans should be refreshed");
        assert_eq!(got.len(), expected.len());
        let joined: String = got.iter().map(|s| s.text.as_str()).collect();
        assert_eq!(joined, line, "refreshed spans must reconstruct the line");
    }

    #[test]
    fn refresh_syntax_spans_is_idempotent_when_output_unchanged() {
        // Spans already matching the resolver output stay identical across a
        // refresh (theme unchanged between passes), so a spurious refresh is a
        // no-op rather than churning colors.
        let path = Path::new("/proj/main.rs");
        let line = "let x = \"s\";";
        let mut items = vec![snap(
            "/proj/main.rs",
            vec![grep_match_line(path, 1, 0, line, &[(0, 3)])],
        )];
        let key = |spans: &[HighlightedSpan]| -> Vec<(u32, bool, String)> {
            spans
                .iter()
                .map(|s| (s.color, s.bold, s.text.clone()))
                .collect()
        };
        let before = key(&items[0].grep_matches[0].syntax_spans);
        assert!(!before.is_empty());

        refresh_syntax_spans(&mut items);

        let after = key(&items[0].grep_matches[0].syntax_spans);
        assert_eq!(before, after);
    }

    // match_goto / cursor_open / goto_for_key / opens_for_selection: Enter
    // open resolution — per-match gotos (1-based CHAR columns) plus the
    // once-per-file multiselect dedupe.

    fn gm(line: u64, col: u32, content: &str, ranges: &[(u32, u32)]) -> GrepMatchLine {
        GrepMatchLine {
            line_number: line,
            line_content: content.to_string(),
            byte_ranges: ranges.to_vec(),
            col,
            syntax_spans: Vec::new(),
        }
    }

    fn snap(path: &str, matches: Vec<GrepMatchLine>) -> FileItemSnapshot {
        FileItemSnapshot {
            file_name: path.rsplit('/').next().unwrap_or(path).to_string(),
            dir: String::new(),
            absolute_path: PathBuf::from(path),
            git_status: None,
            frecency_score: 0,
            match_ranges: Vec::new(),
            grep_matches: matches,
        }
    }

    // Two grep files: /a with matches on lines 3 and 10, /b with one on line 1.
    // The line-10 match sits after 4 spaces (byte col 4 → char col 5).
    fn goto_fixture() -> Vec<FileItemSnapshot> {
        vec![
            snap(
                "/a",
                vec![
                    gm(3, 0, "foo bar", &[(0, 3)]),
                    gm(10, 4, "    foo", &[(4, 7)]),
                ],
            ),
            snap("/b", vec![gm(1, 0, "foo", &[(0, 3)])]),
        ]
    }

    #[test]
    fn match_goto_line_and_one_based_char_column() {
        assert_eq!(match_goto(&gm(42, 4, "let x = 1;", &[(4, 5)])), (42, 5));
    }

    #[test]
    fn match_goto_counts_chars_not_bytes_before_match() {
        // "αβ x": α and β are 2 bytes each, so 'x' sits at BYTE 5 but is the
        // 4th CHAR — the goto column must be 4, never the byte offset.
        assert_eq!(match_goto(&gm(3, 5, "αβ x", &[(5, 6)])), (3, 4));
    }

    #[test]
    fn match_goto_no_ranges_or_misaligned_offset_fall_back_to_column_one() {
        assert_eq!(match_goto(&gm(7, 0, "text", &[])), (7, 1));
        // Byte 1 splits α in two — not a char boundary, so column 1.
        assert_eq!(match_goto(&gm(9, 1, "αx", &[(1, 3)])), (9, 1));
    }

    #[test]
    fn cursor_open_match_row_opens_its_own_match() {
        let results = goto_fixture();
        let rows = rows::build_rows(&results, &HashSet::new(), true);
        // rows: [Header(0), Match{0,0}, Match{0,1}, Separator, Header(1), Match{1,0}]
        // Row 2 is /a's SECOND match (line 10) — its own goto, not the file's first.
        assert_eq!(
            cursor_open(&rows, 2, &results),
            Some((PathBuf::from("/a"), Some((10, 5))))
        );
        assert_eq!(
            cursor_open(&rows, 5, &results),
            Some((PathBuf::from("/b"), Some((1, 1))))
        );
    }

    #[test]
    fn cursor_open_files_row_opens_with_no_goto() {
        let results = vec![snap("/a", Vec::new()), snap("/b", Vec::new())];
        let rows = rows::build_rows(&results, &HashSet::new(), false);
        assert_eq!(
            cursor_open(&rows, 1, &results),
            Some((PathBuf::from("/b"), None))
        );
    }

    #[test]
    fn cursor_open_header_row_is_a_no_op() {
        let results = goto_fixture();
        // Expanded header (unreachable by cursor, but shape-driven no-op).
        let rows = rows::build_rows(&results, &HashSet::new(), true);
        assert_eq!(cursor_open(&rows, 0, &results), None);
        // Collapsed header — the reachable case: Enter does nothing (Zed parity).
        let collapsed: HashSet<PathBuf> = [PathBuf::from("/a")].into();
        let rows = rows::build_rows(&results, &collapsed, true);
        assert_eq!(cursor_open(&rows, 0, &results), None);
    }

    #[test]
    fn cursor_open_separator_and_out_of_range_are_no_ops() {
        let results = goto_fixture();
        let rows = rows::build_rows(&results, &HashSet::new(), true);
        assert_eq!(cursor_open(&rows, 3, &results), None); // separator
        assert_eq!(cursor_open(&rows, rows.len(), &results), None);
        assert_eq!(cursor_open(&[], 0, &results), None);
    }

    #[test]
    fn goto_for_key_grep_key_resolves_to_its_own_matchs_char_col() {
        let results = goto_fixture();
        // Key col is the 0-based BYTE offset (identity); the goto column is
        // the recomputed 1-based CHAR column (byte 4 → col 5 here).
        assert_eq!(
            goto_for_key(&(PathBuf::from("/a"), Some((10, 4))), &results),
            Some((PathBuf::from("/a"), Some((10, 5))))
        );
    }

    #[test]
    fn goto_for_key_files_key_opens_with_no_goto() {
        let results = vec![snap("/a", Vec::new())];
        assert_eq!(
            goto_for_key(&(PathBuf::from("/a"), None), &results),
            Some((PathBuf::from("/a"), None))
        );
    }

    #[test]
    fn goto_for_key_match_gone_falls_back_to_line_column_one() {
        // Path still listed but the exact (line, col) match no longer exists
        // (results drifted between selection and Enter): open at the key's
        // line, column 1.
        let results = goto_fixture();
        assert_eq!(
            goto_for_key(&(PathBuf::from("/a"), Some((99, 3))), &results),
            Some((PathBuf::from("/a"), Some((99, 1))))
        );
    }

    #[test]
    fn goto_for_key_path_gone_skips_entry() {
        let results = goto_fixture();
        assert_eq!(
            goto_for_key(&(PathBuf::from("/gone"), Some((3, 0))), &results),
            None
        );
        assert_eq!(
            goto_for_key(&(PathBuf::from("/gone"), None), &results),
            None
        );
    }

    #[test]
    fn opens_for_selection_same_file_two_matches_open_once_at_first() {
        let results = goto_fixture();
        // Both of /a's matches selected (inserted out of order) plus /b's:
        // /a opens ONCE at its first selected match (line 3), path-sorted.
        let selection: BTreeSet<rows::SelectionKey> = [
            (PathBuf::from("/a"), Some((10, 4))),
            (PathBuf::from("/a"), Some((3, 0))),
            (PathBuf::from("/b"), Some((1, 0))),
        ]
        .into();
        assert_eq!(
            opens_for_selection(&selection, &results),
            vec![
                (PathBuf::from("/a"), Some((3, 1))),
                (PathBuf::from("/b"), Some((1, 1))),
            ]
        );
    }

    #[test]
    fn opens_for_selection_skips_vanished_paths_keeps_live_ones() {
        let results = goto_fixture();
        let selection: BTreeSet<rows::SelectionKey> = [
            (PathBuf::from("/gone"), Some((5, 0))),
            (PathBuf::from("/b"), Some((1, 0))),
        ]
        .into();
        assert_eq!(
            opens_for_selection(&selection, &results),
            vec![(PathBuf::from("/b"), Some((1, 1)))]
        );
    }

    #[test]
    fn opens_for_selection_empty_selection_opens_nothing() {
        assert!(opens_for_selection(&BTreeSet::new(), &goto_fixture()).is_empty());
    }

    // Acceptance edge case: keys marked while a group was expanded survive
    // that group's collapse — selection identity is (path, line, col), never a
    // row index, and fold toggles rebuild only `rows` (the picker prunes the
    // selection on search apply, not on fold). Enter afterwards still opens
    // each file once at its first selected match.
    #[test]
    fn selection_survives_collapsing_marked_group_and_still_opens_once_per_file() {
        let results = goto_fixture();
        let expanded = rows::build_rows(&results, &HashSet::new(), true);
        // Tab-mark both of /a's match rows and /b's while everything is
        // expanded. Layout: 0:H0 1:M00 2:M01 3:Sep 4:H1 5:M10
        let selection: BTreeSet<rows::SelectionKey> = [1, 2, 5]
            .into_iter()
            .filter_map(|ix| rows::selection_key_for_row(&expanded, ix, &results, true))
            .collect();
        assert_eq!(selection.len(), 3);

        // Collapse /a: its match rows vanish from the row projection...
        let collapsed: HashSet<PathBuf> = [PathBuf::from("/a")].into();
        let folded = rows::build_rows(&results, &collapsed, true);
        assert!(
            !folded
                .iter()
                .any(|row| matches!(row, rows::ResultRow::Match { file: 0, .. }))
        );
        // ...but every key still resolves against `results` unchanged.
        assert!(
            selection
                .iter()
                .all(|key| rows::key_survives(key, &results))
        );
        // Enter: one open per file, /a at its FIRST selected match (line 3).
        assert_eq!(
            opens_for_selection(&selection, &results),
            vec![
                (PathBuf::from("/a"), Some((3, 1))),
                (PathBuf::from("/b"), Some((1, 1))),
            ]
        );
    }

    // preview_target: cursor row -> (file, center line, overlay matches) for
    // the preview pane.

    #[test]
    fn preview_target_match_row_centers_its_own_line_with_all_matches_overlaid() {
        let results = goto_fixture();
        let rows = rows::build_rows(&results, &HashSet::new(), true);
        // rows: [Header(0), Match{0,0}, Match{0,1}, Separator, Header(1), Match{1,0}]
        // Row 2 is /a's SECOND match (line 10) — the center follows it, while
        // the overlay still carries ALL of /a's matches, not just the first.
        let (path, center, overlay) = preview_target(&rows, 2, &results).unwrap();
        assert_eq!(path, PathBuf::from("/a"));
        assert_eq!(center, Some(10));
        let overlay_lines: Vec<u64> = overlay.iter().map(|m| m.line_number).collect();
        assert_eq!(overlay_lines, vec![3, 10]);
    }

    #[test]
    fn preview_target_collapsed_header_centers_first_match() {
        let results = goto_fixture();
        let collapsed: HashSet<PathBuf> = [PathBuf::from("/a")].into();
        let rows = rows::build_rows(&results, &collapsed, true);
        // Row 0 is /a's collapsed header: center the file's first match.
        let (path, center, overlay) = preview_target(&rows, 0, &results).unwrap();
        assert_eq!(path, PathBuf::from("/a"));
        assert_eq!(center, Some(3));
        assert_eq!(overlay.len(), 2);
    }

    #[test]
    fn preview_target_files_row_has_no_center_and_no_overlay() {
        let results = vec![snap("/a", Vec::new()), snap("/b", Vec::new())];
        let rows = rows::build_rows(&results, &HashSet::new(), false);
        let (path, center, overlay) = preview_target(&rows, 1, &results).unwrap();
        assert_eq!(path, PathBuf::from("/b"));
        assert_eq!(center, None);
        assert!(overlay.is_empty());
    }

    #[test]
    fn preview_target_separator_and_out_of_range_are_none() {
        let results = goto_fixture();
        let rows = rows::build_rows(&results, &HashSet::new(), true);
        assert!(preview_target(&rows, 3, &results).is_none()); // separator
        assert!(preview_target(&rows, rows.len(), &results).is_none());
        assert!(preview_target(&[], 0, &results).is_none());
    }

    // recenter_scroll_row: serve a same-file cursor move from the loaded
    // window (Some(0-based scroll row)) vs take the full reload path (None).

    #[test]
    fn recenter_same_whole_file_yields_zero_based_scroll_row() {
        let a = Path::new("/a");
        assert_eq!(
            recenter_scroll_row(Some(a), false, 1, 40, a, Some(10)),
            Some(9)
        );
        // First and last loaded lines are still servable.
        assert_eq!(
            recenter_scroll_row(Some(a), false, 1, 40, a, Some(1)),
            Some(0)
        );
        assert_eq!(
            recenter_scroll_row(Some(a), false, 1, 40, a, Some(40)),
            Some(39)
        );
    }

    #[test]
    fn recenter_reloads_on_other_file_no_loaded_file_or_load_in_flight() {
        let a = Path::new("/a");
        assert_eq!(
            recenter_scroll_row(Some(Path::new("/other")), false, 1, 40, a, Some(10)),
            None
        );
        assert_eq!(recenter_scroll_row(None, false, 1, 40, a, Some(10)), None);
        assert_eq!(recenter_scroll_row(Some(a), true, 1, 40, a, Some(10)), None);
    }

    #[test]
    fn recenter_reloads_when_window_may_be_truncated_or_line_outside() {
        let a = Path::new("/a");
        // A window starting past line 1 is centered deep in a big file.
        assert_eq!(
            recenter_scroll_row(Some(a), false, 100, 40, a, Some(120)),
            None
        );
        // A full-cap window may be truncated — re-window around the new center.
        assert_eq!(
            recenter_scroll_row(Some(a), false, 1, MAX_PREVIEW_LINES, a, Some(10)),
            None
        );
        // Target line beyond the loaded lines (e.g. file changed on disk).
        assert_eq!(
            recenter_scroll_row(Some(a), false, 1, 40, a, Some(41)),
            None
        );
        // No center line to serve (Files-view target).
        assert_eq!(recenter_scroll_row(Some(a), false, 1, 40, a, None), None);
    }

    // status_left_text / status_right_hints: bottom status-bar counts and key
    // hints.

    // A grep-view snapshot with `n` matches (lines 1..=n).
    fn snap_n(path: &str, n: u64) -> FileItemSnapshot {
        snap(
            path,
            (1..=n).map(|line| gm(line, 0, "x", &[(0, 1)])).collect(),
        )
    }

    #[test]
    fn status_left_grep_sums_visible_matches_not_total_matched() {
        // 3 + 2 = 5 visible matches across 2 files; `total_matched` carries a
        // deliberately different value (in grep view it is the deduped FILE
        // count, and a bogus 99 here proves the helper never reads it).
        let results = vec![snap_n("/a", 3), snap_n("/b", 2)];
        let text = status_left_text(
            SearchView::Grep,
            GrepMode::PlainText,
            None,
            true,
            0,
            10,
            99,
            0,
            &results,
        );
        assert_eq!(
            text,
            "5 matches in 2 files  0 selected  10 indexed  \u{2022}  mode: plain  \u{21E7}tab mode"
        );
    }

    #[test]
    fn status_left_grep_singular_match_and_file() {
        let results = vec![snap_n("/a", 1)];
        let text = status_left_text(
            SearchView::Grep,
            GrepMode::PlainText,
            None,
            true,
            0,
            4,
            1,
            0,
            &results,
        );
        assert!(text.starts_with("1 match in 1 file  "), "got: {text}");
    }

    #[test]
    fn status_left_grep_zero_matches() {
        let text = status_left_text(
            SearchView::Grep,
            GrepMode::PlainText,
            None,
            true,
            0,
            8,
            0,
            0,
            &[],
        );
        assert!(
            text.starts_with("0 matches in 0 files  0 selected"),
            "got: {text}"
        );
    }

    #[test]
    fn status_left_grep_selected_count_and_submode() {
        let results = vec![snap_n("/a", 2)];
        let text = status_left_text(
            SearchView::Grep,
            GrepMode::Regex,
            None,
            true,
            0,
            5,
            1,
            3,
            &results,
        );
        assert!(text.contains("  3 selected  "), "got: {text}");
        assert!(
            text.ends_with("mode: regex  \u{21E7}tab mode"),
            "got: {text}"
        );
        let fuzzy = status_left_text(
            SearchView::Grep,
            GrepMode::Fuzzy,
            None,
            true,
            0,
            5,
            1,
            3,
            &results,
        );
        assert!(fuzzy.contains("mode: fuzzy"), "got: {fuzzy}");
    }

    #[test]
    fn status_left_files_keeps_flat_counts_no_submode_hint() {
        // Files view: unchanged "shown/selected/matches/indexed" counts —
        // there `total_matched` IS the real match total — and no grep hint.
        let results = vec![
            snap("/a", Vec::new()),
            snap("/b", Vec::new()),
            snap("/c", Vec::new()),
        ];
        let text = status_left_text(
            SearchView::Files,
            GrepMode::PlainText,
            None,
            true,
            0,
            50,
            7,
            2,
            &results,
        );
        assert_eq!(text, "3 shown  2 selected  7 matches  50 indexed");
    }

    #[test]
    fn status_left_status_message_wins() {
        let results = vec![snap_n("/a", 2)];
        let files = status_left_text(
            SearchView::Files,
            GrepMode::PlainText,
            Some("Opened 2 files"),
            true,
            9,
            9,
            9,
            9,
            &results,
        );
        assert_eq!(files, "Opened 2 files");
        // Grep view keeps appending the submode hint after the message.
        let grep = status_left_text(
            SearchView::Grep,
            GrepMode::PlainText,
            Some("Opened 2 files"),
            true,
            9,
            9,
            9,
            9,
            &results,
        );
        assert_eq!(
            grep,
            "Opened 2 files  \u{2022}  mode: plain  \u{21E7}tab mode"
        );
    }

    #[test]
    fn status_left_indexing_in_progress() {
        let files = status_left_text(
            SearchView::Files,
            GrepMode::PlainText,
            None,
            false,
            42,
            0,
            0,
            0,
            &[],
        );
        assert_eq!(files, "indexing. 42 files");
        // Nothing indexed yet: empty in Files view; the grep hint stands
        // alone (no leading bullet separator) in Grep view.
        let files_zero = status_left_text(
            SearchView::Files,
            GrepMode::PlainText,
            None,
            false,
            0,
            0,
            0,
            0,
            &[],
        );
        assert_eq!(files_zero, "");
        let grep_zero = status_left_text(
            SearchView::Grep,
            GrepMode::PlainText,
            None,
            false,
            0,
            0,
            0,
            0,
            &[],
        );
        assert_eq!(grep_zero, "mode: plain  \u{21E7}tab mode");
    }

    #[test]
    fn status_left_indexed_falls_back_to_indexed_count() {
        // total_files == 0 (e.g. content indexing off) -> show indexed_count.
        let text = status_left_text(
            SearchView::Files,
            GrepMode::PlainText,
            None,
            true,
            7,
            0,
            0,
            0,
            &[],
        );
        assert_eq!(text, "0 shown  0 selected  0 matches  7 indexed");
    }

    #[test]
    fn status_right_hints_grep_includes_fold_and_mode_switch() {
        assert_eq!(
            status_right_hints(SearchView::Grep),
            "\u{2191}\u{2193} nav  \u{21E5} mark  \u{2318}\u{21E7}S multi  \u{2325}Z fold  cmd-f files  \u{23CE} open  esc quit"
        );
    }

    #[test]
    fn status_right_hints_files_omits_fold_keeps_mode_switch() {
        let hints = status_right_hints(SearchView::Files);
        assert_eq!(
            hints,
            "\u{2191}\u{2193} nav  \u{21E5} mark  \u{2318}\u{21E7}S multi  cmd-g grep  \u{23CE} open  esc quit"
        );
        assert!(!hints.contains("fold"));
    }

    // match_row_spans / slice_spans / truncate_start: Zed-style match-row
    // span assembly (syntax spans + match byte ranges -> display spans).

    const BG: u32 = 0xAABBCC;

    fn syn_span(text: &str, color: u32) -> HighlightedSpan {
        HighlightedSpan {
            color,
            bg: None,
            italic: false,
            bold: false,
            underline: false,
            strikethrough: false,
            matched: false,
            text: text.to_string(),
        }
    }

    fn concat_spans(spans: &[HighlightedSpan]) -> String {
        spans.iter().map(|s| s.text.as_str()).collect()
    }

    fn marked_text(spans: &[HighlightedSpan]) -> String {
        spans
            .iter()
            .filter(|s| s.bg.is_some())
            .map(|s| s.text.as_str())
            .collect()
    }

    #[test]
    fn match_row_spans_overlays_ranges_across_syntax_span_boundaries() {
        let spans = vec![
            syn_span("let ", 0x111111),
            syn_span("x", 0x222222),
            syn_span(" = 1;", 0x333333),
        ];
        // "let x = 1;": range 2..6 = "t x " straddles all three spans.
        let out = match_row_spans(&spans, "let x = 1;", &[(2, 6)], 0x999999, BG);
        let got: Vec<(String, u32, Option<u32>, bool)> = out
            .iter()
            .map(|s| (s.text.clone(), s.color, s.bg, s.bold))
            .collect();
        assert_eq!(
            got,
            vec![
                ("le".to_string(), 0x111111, None, false),
                ("t ".to_string(), 0x111111, Some(BG), true),
                ("x".to_string(), 0x222222, Some(BG), true),
                (" ".to_string(), 0x333333, Some(BG), true),
                ("= 1;".to_string(), 0x333333, None, false),
            ]
        );
        assert_eq!(concat_spans(&out), "let x = 1;");
    }

    #[test]
    fn match_row_spans_trims_whitespace_and_shifts_ranges() {
        let line = "    foo bar  ";
        let spans = vec![syn_span("    ", 0x111111), syn_span("foo bar  ", 0x222222)];
        // "foo" = bytes 4..7 of the raw line -> 0..3 of the trimmed text.
        let out = match_row_spans(&spans, line, &[(4, 7)], 0x999999, BG);
        assert_eq!(concat_spans(&out), "foo bar");
        assert_eq!(marked_text(&out), "foo");
        assert!(out.iter().all(|s| s.color == 0x222222));
        // A range entirely inside the stripped leading whitespace vanishes.
        let out = match_row_spans(&spans, line, &[(0, 2)], 0x999999, BG);
        assert_eq!(concat_spans(&out), "foo bar");
        assert_eq!(marked_text(&out), "");
    }

    #[test]
    fn match_row_spans_plain_fallback_without_syntax_spans() {
        let out = match_row_spans(&[], "hello world", &[(0, 5)], 0x999999, BG);
        assert_eq!(concat_spans(&out), "hello world");
        assert_eq!(marked_text(&out), "hello");
        assert!(out.iter().all(|s| s.color == 0x999999));
        assert!(out.iter().filter(|s| s.bg.is_some()).all(|s| s.bold));
    }

    #[test]
    fn match_row_spans_empty_and_whitespace_lines_yield_nothing() {
        assert!(match_row_spans(&[], "", &[], 0x9, BG).is_empty());
        let ws = vec![syn_span("   ", 0x111111)];
        assert!(match_row_spans(&ws, "   ", &[(0, 1)], 0x9, BG).is_empty());
    }

    #[test]
    fn match_row_spans_no_ranges_returns_trimmed_spans_unmarked() {
        let spans = vec![syn_span("  a", 0x111111), syn_span("b ", 0x222222)];
        let out = match_row_spans(&spans, "  ab ", &[], 0x999999, BG);
        let got: Vec<(String, u32)> = out.iter().map(|s| (s.text.clone(), s.color)).collect();
        assert_eq!(
            got,
            vec![("a".to_string(), 0x111111), ("b".to_string(), 0x222222)]
        );
        assert!(out.iter().all(|s| s.bg.is_none() && !s.bold));
    }

    #[test]
    fn match_row_spans_multibyte_trim_and_ranges_stay_on_char_boundaries() {
        // "  héllo wörld": strip = 2, "héllo" = raw bytes 2..8.
        let line = "  héllo wörld";
        let spans = vec![syn_span(line, 0x111111)];
        let out = match_row_spans(&spans, line, &[(2, 8)], 0x999999, BG);
        assert_eq!(concat_spans(&out), "héllo wörld");
        assert_eq!(marked_text(&out), "héllo");
    }

    #[test]
    fn slice_spans_clamps_window_edges_to_char_boundaries() {
        // Spans "aé" (bytes 0..3) and "βx" (bytes 3..6). Window 2..4 falls
        // mid-'é' (start shrinks left) and mid-'β' (end grows right) without
        // panicking on either edge.
        let spans = vec![syn_span("aé", 0x111111), syn_span("βx", 0x222222)];
        let out = slice_spans(&spans, 2, 4);
        let got: Vec<(String, u32)> = out.iter().map(|s| (s.text.clone(), s.color)).collect();
        assert_eq!(
            got,
            vec![("é".to_string(), 0x111111), ("β".to_string(), 0x222222)]
        );
        // Full window copies everything; empty/out-of-range windows yield
        // nothing.
        assert_eq!(concat_spans(&slice_spans(&spans, 0, 6)), "aéβx");
        assert!(slice_spans(&spans, 3, 3).is_empty());
        assert!(slice_spans(&spans, 6, 10).is_empty());
    }

    #[test]
    fn truncate_start_keeps_tail_with_leading_ellipsis() {
        assert_eq!(truncate_start("src/deep/nested/dir", 8), "…ted/dir");
        assert_eq!(truncate_start("short", 8), "short");
        assert_eq!(truncate_start("exact", 5), "exact");
    }

    #[test]
    fn truncate_start_is_multibyte_safe_and_handles_tiny_budgets() {
        assert_eq!(truncate_start("ééééé", 3), "…éé");
        assert_eq!(truncate_start("ééé", 3), "ééé");
        assert_eq!(truncate_start("abc", 1), "…");
        assert_eq!(truncate_start("abc", 0), "");
    }

    #[test]
    fn gutter_number_color_uses_active_token_on_match_line() {
        // The centered match line's number reads `editor_active_line_number`,
        // alpha byte included.
        assert_eq!(
            gutter_number_color(true, 0x4E_5A_6F_80, 0xAB_B2_BF_FF),
            0xAB_B2_BF_FF
        );
    }

    #[test]
    fn gutter_number_color_uses_line_number_token_otherwise() {
        // Every other row reads the translucent `editor_line_number` token.
        assert_eq!(
            gutter_number_color(false, 0x4E_5A_6F_80, 0xAB_B2_BF_FF),
            0x4E_5A_6F_80
        );
    }
}
