// Derived row projection for the results pane: pure row building, selection
// stepping, re-anchoring, and multiselect open-dedupe over `FileItemSnapshot`
// slices. No gpui imports — everything here stays unit-testable (project
// policy; see layout.rs for the same pattern).

use std::collections::{BTreeSet, HashSet};
use std::ops::Range;
use std::path::{Path, PathBuf};

use crate::picker::FileItemSnapshot;

/// Multiselect identity: path plus `Some((line, col))` for grep match rows,
/// `None` for Files-mode whole-file rows. The `col` is a 0-based BYTE offset
/// used for match identity ONLY — editor goto columns come from the picker's
/// `match_goto` 1-based char-column computation.
pub type SelectionKey = (PathBuf, Option<(u64, u32)>);

/// One rendered row of the results list, derived from the per-file snapshots.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResultRow {
    /// Grep file-group header; the payload indexes `results`.
    Header(usize),
    /// One match line: `results[file].grep_matches[m]`. The Files view uses
    /// `m: 0` as its whole-file row.
    Match { file: usize, m: usize },
    /// Divider between grep file groups (never before the first group).
    Separator,
}

/// Direction for `step_selectable`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    Next,
    Prev,
}

// Project the per-file snapshots into list rows. Files view: one flat
// `Match { file, m: 0 }` row per file (no headers/separators; `collapsed` is
// ignored). Grep view: `Separator` (except before the first group) +
// `Header(file)` + one `Match` row per grep match, with the match rows omitted
// while the file's path is in `collapsed`.
pub fn build_rows(
    results: &[FileItemSnapshot],
    collapsed: &HashSet<PathBuf>,
    is_grep: bool,
) -> Vec<ResultRow> {
    if !is_grep {
        return (0..results.len())
            .map(|file| ResultRow::Match { file, m: 0 })
            .collect();
    }
    let mut rows = Vec::new();
    for (file, snapshot) in results.iter().enumerate() {
        if file > 0 {
            rows.push(ResultRow::Separator);
        }
        rows.push(ResultRow::Header(file));
        if !collapsed.contains(&snapshot.absolute_path) {
            for m in 0..snapshot.grep_matches.len() {
                rows.push(ResultRow::Match { file, m });
            }
        }
    }
    rows
}

// Whether the cursor may rest on `rows[ix]`: Match rows always, Header rows
// only while their file is collapsed (the header stands in for its hidden
// matches), Separators and out-of-range indices never.
pub fn can_select(
    rows: &[ResultRow],
    ix: usize,
    collapsed: &HashSet<PathBuf>,
    results: &[FileItemSnapshot],
) -> bool {
    match rows.get(ix) {
        Some(ResultRow::Match { .. }) => true,
        Some(ResultRow::Header(file)) => results
            .get(*file)
            .is_some_and(|snapshot| collapsed.contains(&snapshot.absolute_path)),
        _ => false,
    }
}

// Walk from `from` (exclusive) in `dir` until a selectable row; no wrap.
// Returns None when no selectable row exists in that direction, so callers
// clamp by keeping the current selection.
pub fn step_selectable(
    rows: &[ResultRow],
    from: usize,
    dir: Direction,
    collapsed: &HashSet<PathBuf>,
    results: &[FileItemSnapshot],
) -> Option<usize> {
    match dir {
        Direction::Next => (from.saturating_add(1)..rows.len())
            .find(|&ix| can_select(rows, ix, collapsed, results)),
        Direction::Prev => (0..from.min(rows.len()))
            .rev()
            .find(|&ix| can_select(rows, ix, collapsed, results)),
    }
}

// Resolve a row index to its (file, match) coordinates in `results`:
// Match rows yield `(file, Some(m))`, Header rows `(file, None)`, and
// Separators / out-of-range indices yield `None`. Every picker site that used
// to index `results` with the selection must go through this instead —
// `selected` indexes rows, not results.
pub fn resolve_row(rows: &[ResultRow], ix: usize) -> Option<(usize, Option<usize>)> {
    match rows.get(ix)? {
        ResultRow::Header(file) => Some((*file, None)),
        ResultRow::Match { file, m } => Some((*file, Some(*m))),
        ResultRow::Separator => None,
    }
}

// First selectable row in `rows`, scanning from the top.
pub fn first_selectable(
    rows: &[ResultRow],
    collapsed: &HashSet<PathBuf>,
    results: &[FileItemSnapshot],
) -> Option<usize> {
    (0..rows.len()).find(|&ix| can_select(rows, ix, collapsed, results))
}

// Re-anchor the cursor after a rebuild. `old` is the previously selected
// (path, match index) — Files-mode rows use match index 0. Resolution order:
// the same path + match index if that row still exists; the file's header if
// the file is now collapsed; the file's last match row if the match index is
// gone (nearest within the file); otherwise (file removed / no old selection)
// the first selectable row. Falls back to 0 when nothing is selectable.
pub fn anchor_selection(
    old: Option<(&Path, usize)>,
    rows: &[ResultRow],
    collapsed: &HashSet<PathBuf>,
    results: &[FileItemSnapshot],
) -> usize {
    if let Some((path, m)) = old
        && let Some(file) = results
            .iter()
            .position(|snapshot| snapshot.absolute_path == path)
    {
        if collapsed.contains(path) {
            if let Some(ix) = rows
                .iter()
                .position(|row| matches!(row, ResultRow::Header(f) if *f == file))
            {
                return ix;
            }
        } else {
            let mut last_match_row = None;
            for (ix, row) in rows.iter().enumerate() {
                if let ResultRow::Match { file: f, m: mm } = row
                    && *f == file
                {
                    if *mm == m {
                        return ix;
                    }
                    last_match_row = Some(ix);
                }
            }
            // Match index gone (fewer matches now): nearest within the file.
            if let Some(ix) = last_match_row {
                return ix;
            }
        }
    }
    first_selectable(rows, collapsed, results).unwrap_or(0)
}

// Row range occupied by `file`'s match rows: starts right after the group's
// header and spans the contiguous `Match` rows that follow (empty when the
// group is collapsed). `None` when the header is absent (Files-view flat rows
// or a file index not in `rows`). Fold toggles splice this range: the old
// range comes from the pre-toggle rows and the new count from the post-toggle
// rows — the start index is identical because rows before the header are
// untouched — so `ListState::splice(old, new.len())` preserves scroll for
// everything outside the group.
pub fn group_match_range(rows: &[ResultRow], file: usize) -> Option<Range<usize>> {
    let header = rows
        .iter()
        .position(|row| matches!(row, ResultRow::Header(f) if *f == file))?;
    let start = header + 1;
    let count = rows[start..]
        .iter()
        .take_while(|row| matches!(row, ResultRow::Match { file: f, .. } if *f == file))
        .count();
    Some(start..start + count)
}

// Toggle-all fold semantics: any group collapsed → expand all (empty set);
// nothing collapsed → collapse every result file. `collapsed` only ever holds
// paths from `results` (it is cleared on search apply and mode switch), so
// the is_empty check is exact.
pub fn toggle_all_collapsed(
    collapsed: &HashSet<PathBuf>,
    results: &[FileItemSnapshot],
) -> HashSet<PathBuf> {
    if collapsed.is_empty() {
        results
            .iter()
            .map(|snapshot| snapshot.absolute_path.clone())
            .collect()
    } else {
        HashSet::new()
    }
}

// The multiselect key for `rows[ix]`: grep Match rows key their exact match
// `(path, Some((line, col)))`, Files-view rows key the whole file
// `(path, None)`. Header rows (collapsed or not), Separators, and
// out-of-range indices carry no key — toggling there is a no-op.
pub fn selection_key_for_row(
    rows: &[ResultRow],
    ix: usize,
    results: &[FileItemSnapshot],
    is_grep: bool,
) -> Option<SelectionKey> {
    let (file, m) = resolve_row(rows, ix)?;
    let m = m?;
    let item = results.get(file)?;
    if is_grep {
        let gm = item.grep_matches.get(m)?;
        Some((item.absolute_path.clone(), Some((gm.line_number, gm.col))))
    } else {
        Some((item.absolute_path.clone(), None))
    }
}

// Prune predicate applied to the selection after each search apply: a grep
// match key survives only while its exact (path, line, col) triple still
// exists in the visible results; a Files-mode key survives while its path
// does.
pub fn key_survives(key: &SelectionKey, results: &[FileItemSnapshot]) -> bool {
    let (path, match_key) = key;
    match match_key {
        None => results.iter().any(|item| &item.absolute_path == path),
        Some((line, col)) => results.iter().any(|item| {
            &item.absolute_path == path
                && item
                    .grep_matches
                    .iter()
                    .any(|m| m.line_number == *line && m.col == *col)
        }),
    }
}

// Toggle-all multiselect semantics (ctrl-a): any key selected → clear the
// whole selection; nothing selected → select every visible match key (one per
// (path, line, col) triple in grep view, one per-file key in Files view).
pub fn toggle_all_selection(
    selection: &BTreeSet<SelectionKey>,
    results: &[FileItemSnapshot],
    is_grep: bool,
) -> BTreeSet<SelectionKey> {
    if !selection.is_empty() {
        return BTreeSet::new();
    }
    if is_grep {
        results
            .iter()
            .flat_map(|item| {
                item.grep_matches
                    .iter()
                    .map(|m| (item.absolute_path.clone(), Some((m.line_number, m.col))))
            })
            .collect()
    } else {
        results
            .iter()
            .map(|item| (item.absolute_path.clone(), None))
            .collect()
    }
}

// Explicit multiselect-mode toggle (cmd-shift-s): the new mode flag plus the
// selection that goes with it — leaving the mode clears the selection,
// entering it keeps whatever is already there.
pub fn toggle_multi_select_mode(
    mode: bool,
    selection: &BTreeSet<SelectionKey>,
) -> (bool, BTreeSet<SelectionKey>) {
    let entering = !mode;
    let selection = if entering {
        selection.clone()
    } else {
        BTreeSet::new()
    };
    (entering, selection)
}

// Reduce a selection to one open per file: the FIRST selected match of each
// file. BTreeSet iteration is already (path, then line/col) ordered, so the
// first key seen for a path is its lowest match; output stays path-sorted.
pub fn dedupe_opens(selection: &BTreeSet<SelectionKey>) -> Vec<SelectionKey> {
    let mut opens: Vec<SelectionKey> = Vec::new();
    for key in selection {
        if opens.last().is_none_or(|(path, _)| path != &key.0) {
            opens.push(key.clone());
        }
    }
    opens
}

// Widest line number across all grep matches in the results (0 when there are
// none) — feeds `layout::gutter_width` for the match-row gutter.
pub fn max_line_number(results: &[FileItemSnapshot]) -> u64 {
    results
        .iter()
        .flat_map(|snapshot| snapshot.grep_matches.iter())
        .map(|m| m.line_number)
        .max()
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::picker::GrepMatchLine;

    fn snap(path: &str, lines: &[u64]) -> FileItemSnapshot {
        let matches: Vec<(u64, u32)> = lines.iter().map(|&line| (line, 0)).collect();
        snap_with_cols(path, &matches)
    }

    fn snap_with_cols(path: &str, matches: &[(u64, u32)]) -> FileItemSnapshot {
        FileItemSnapshot {
            file_name: path.rsplit('/').next().unwrap_or(path).to_string(),
            dir: String::new(),
            absolute_path: PathBuf::from(path),
            git_status: None,
            frecency_score: 0,
            match_ranges: Vec::new(),
            grep_matches: matches
                .iter()
                .map(|&(line_number, col)| GrepMatchLine {
                    line_number,
                    line_content: String::new(),
                    byte_ranges: Vec::new(),
                    col,
                    syntax_spans: Vec::new(),
                })
                .collect(),
        }
    }

    fn collapsed_set(paths: &[&str]) -> HashSet<PathBuf> {
        paths.iter().map(PathBuf::from).collect()
    }

    // Three grep files: a (2 matches), b (1 match), c (2 matches).
    fn fixture() -> Vec<FileItemSnapshot> {
        vec![
            snap("/a.rs", &[1, 5]),
            snap("/b.rs", &[2]),
            snap("/c.rs", &[7, 9]),
        ]
    }

    // build_rows

    #[test]
    fn build_rows_grep_groups_with_separators_between_groups_only() {
        let results = fixture();
        let rows = build_rows(&results, &HashSet::new(), true);
        assert_eq!(
            rows,
            vec![
                ResultRow::Header(0),
                ResultRow::Match { file: 0, m: 0 },
                ResultRow::Match { file: 0, m: 1 },
                ResultRow::Separator,
                ResultRow::Header(1),
                ResultRow::Match { file: 1, m: 0 },
                ResultRow::Separator,
                ResultRow::Header(2),
                ResultRow::Match { file: 2, m: 0 },
                ResultRow::Match { file: 2, m: 1 },
            ]
        );
    }

    #[test]
    fn build_rows_collapsed_file_keeps_header_omits_matches() {
        let results = fixture();
        let rows = build_rows(&results, &collapsed_set(&["/b.rs"]), true);
        assert_eq!(
            rows,
            vec![
                ResultRow::Header(0),
                ResultRow::Match { file: 0, m: 0 },
                ResultRow::Match { file: 0, m: 1 },
                ResultRow::Separator,
                ResultRow::Header(1),
                ResultRow::Separator,
                ResultRow::Header(2),
                ResultRow::Match { file: 2, m: 0 },
                ResultRow::Match { file: 2, m: 1 },
            ]
        );
    }

    #[test]
    fn build_rows_files_mode_is_flat_and_ignores_collapsed() {
        let results = fixture();
        let rows = build_rows(&results, &collapsed_set(&["/a.rs", "/b.rs"]), false);
        assert_eq!(
            rows,
            vec![
                ResultRow::Match { file: 0, m: 0 },
                ResultRow::Match { file: 1, m: 0 },
                ResultRow::Match { file: 2, m: 0 },
            ]
        );
    }

    #[test]
    fn build_rows_empty_results_yield_no_rows() {
        assert!(build_rows(&[], &HashSet::new(), true).is_empty());
        assert!(build_rows(&[], &HashSet::new(), false).is_empty());
    }

    #[test]
    fn build_rows_single_file_has_no_separators() {
        let results = vec![snap("/a.rs", &[1, 5])];
        let rows = build_rows(&results, &HashSet::new(), true);
        assert_eq!(
            rows,
            vec![
                ResultRow::Header(0),
                ResultRow::Match { file: 0, m: 0 },
                ResultRow::Match { file: 0, m: 1 },
            ]
        );
        // Collapsed single file: just its header.
        let rows = build_rows(&results, &collapsed_set(&["/a.rs"]), true);
        assert_eq!(rows, vec![ResultRow::Header(0)]);
    }

    // can_select

    #[test]
    fn can_select_match_rows_always() {
        let results = fixture();
        let none = HashSet::new();
        let rows = build_rows(&results, &none, true);
        for (ix, row) in rows.iter().enumerate() {
            if matches!(row, ResultRow::Match { .. }) {
                assert!(can_select(&rows, ix, &none, &results), "row {ix}");
            }
        }
    }

    #[test]
    fn can_select_header_only_when_its_file_is_collapsed() {
        let results = fixture();
        let collapsed = collapsed_set(&["/b.rs"]);
        let rows = build_rows(&results, &collapsed, true);
        // Fixture B layout: 0:H0 1:M00 2:M01 3:Sep 4:H1 5:Sep 6:H2 7:M20 8:M21
        assert!(!can_select(&rows, 0, &collapsed, &results)); // expanded header
        assert!(can_select(&rows, 4, &collapsed, &results)); // collapsed header
        assert!(!can_select(&rows, 6, &collapsed, &results)); // expanded header
    }

    #[test]
    fn can_select_separators_and_out_of_range_never() {
        let results = fixture();
        let none = HashSet::new();
        let rows = build_rows(&results, &none, true);
        assert!(!can_select(&rows, 3, &none, &results)); // separator
        assert!(!can_select(&rows, 6, &none, &results)); // separator
        assert!(!can_select(&rows, rows.len(), &none, &results));
        assert!(!can_select(&rows, usize::MAX, &none, &results));
    }

    // step_selectable

    #[test]
    fn step_selectable_skips_separator_and_expanded_header() {
        let results = fixture();
        let none = HashSet::new();
        let rows = build_rows(&results, &none, true);
        // Fixture A layout: 0:H0 1:M00 2:M01 3:Sep 4:H1 5:M10 6:Sep 7:H2 8:M20 9:M21
        assert_eq!(
            step_selectable(&rows, 2, Direction::Next, &none, &results),
            Some(5)
        );
        assert_eq!(
            step_selectable(&rows, 5, Direction::Prev, &none, &results),
            Some(2)
        );
    }

    #[test]
    fn step_selectable_lands_on_collapsed_header() {
        let results = fixture();
        let collapsed = collapsed_set(&["/b.rs"]);
        let rows = build_rows(&results, &collapsed, true);
        // Fixture B layout: 0:H0 1:M00 2:M01 3:Sep 4:H1 5:Sep 6:H2 7:M20 8:M21
        assert_eq!(
            step_selectable(&rows, 2, Direction::Next, &collapsed, &results),
            Some(4)
        );
        assert_eq!(
            step_selectable(&rows, 4, Direction::Next, &collapsed, &results),
            Some(7)
        );
        assert_eq!(
            step_selectable(&rows, 7, Direction::Prev, &collapsed, &results),
            Some(4)
        );
    }

    #[test]
    fn step_selectable_clamps_at_ends_without_wrapping() {
        let results = fixture();
        let none = HashSet::new();
        let rows = build_rows(&results, &none, true);
        // Last selectable row is 9 (M21), first is 1 (M00).
        assert_eq!(
            step_selectable(&rows, 9, Direction::Next, &none, &results),
            None
        );
        assert_eq!(
            step_selectable(&rows, 1, Direction::Prev, &none, &results),
            None
        );
        assert_eq!(
            step_selectable(&rows, 0, Direction::Prev, &none, &results),
            None
        );
        // Out-of-range starting points do not panic.
        assert_eq!(
            step_selectable(&rows, usize::MAX, Direction::Next, &none, &results),
            None
        );
        assert_eq!(
            step_selectable(&rows, usize::MAX, Direction::Prev, &none, &results),
            Some(9)
        );
    }

    #[test]
    fn step_selectable_navigates_headers_when_all_collapsed() {
        let results = fixture();
        let collapsed = collapsed_set(&["/a.rs", "/b.rs", "/c.rs"]);
        let rows = build_rows(&results, &collapsed, true);
        // Fixture C layout: 0:H0 1:Sep 2:H1 3:Sep 4:H2
        assert_eq!(
            step_selectable(&rows, 0, Direction::Next, &collapsed, &results),
            Some(2)
        );
        assert_eq!(
            step_selectable(&rows, 2, Direction::Next, &collapsed, &results),
            Some(4)
        );
        assert_eq!(
            step_selectable(&rows, 4, Direction::Next, &collapsed, &results),
            None
        );
        assert_eq!(
            step_selectable(&rows, 2, Direction::Prev, &collapsed, &results),
            Some(0)
        );
        assert_eq!(
            step_selectable(&rows, 0, Direction::Prev, &collapsed, &results),
            None
        );
    }

    #[test]
    fn step_selectable_empty_rows_return_none() {
        assert_eq!(
            step_selectable(&[], 0, Direction::Next, &HashSet::new(), &[]),
            None
        );
        assert_eq!(
            step_selectable(&[], 0, Direction::Prev, &HashSet::new(), &[]),
            None
        );
    }

    // resolve_row

    #[test]
    fn resolve_row_maps_match_and_header_rows_to_file_coords() {
        let results = fixture();
        let rows = build_rows(&results, &HashSet::new(), true);
        // Fixture A layout: 0:H0 1:M00 2:M01 3:Sep 4:H1 5:M10 6:Sep 7:H2 8:M20 9:M21
        assert_eq!(resolve_row(&rows, 0), Some((0, None)));
        assert_eq!(resolve_row(&rows, 2), Some((0, Some(1))));
        assert_eq!(resolve_row(&rows, 5), Some((1, Some(0))));
        assert_eq!(resolve_row(&rows, 9), Some((2, Some(1))));
    }

    #[test]
    fn resolve_row_separator_and_out_of_range_resolve_to_none() {
        let results = fixture();
        let rows = build_rows(&results, &HashSet::new(), true);
        assert_eq!(resolve_row(&rows, 3), None); // separator
        assert_eq!(resolve_row(&rows, rows.len()), None);
        assert_eq!(resolve_row(&rows, usize::MAX), None);
        assert_eq!(resolve_row(&[], 0), None);
    }

    #[test]
    fn resolve_row_files_mode_rows_resolve_to_match_zero() {
        let results = fixture();
        let rows = build_rows(&results, &HashSet::new(), false);
        assert_eq!(resolve_row(&rows, 0), Some((0, Some(0))));
        assert_eq!(resolve_row(&rows, 2), Some((2, Some(0))));
    }

    // first_selectable

    #[test]
    fn first_selectable_skips_leading_header_when_expanded() {
        let results = fixture();
        let none = HashSet::new();
        let rows = build_rows(&results, &none, true);
        assert_eq!(first_selectable(&rows, &none, &results), Some(1));
        let all = collapsed_set(&["/a.rs", "/b.rs", "/c.rs"]);
        let rows = build_rows(&results, &all, true);
        assert_eq!(first_selectable(&rows, &all, &results), Some(0));
        assert_eq!(first_selectable(&[], &none, &[]), None);
    }

    // Seeding contract after every rebuild: `selected` becomes
    // `first_selectable(..).unwrap_or(0)` — row 0 in Files view (flat rows),
    // row 1 in grep view (row 0 is an expanded, unselectable Header), and the
    // 0 fallback when nothing is selectable.
    #[test]
    fn first_selectable_seed_files_mode_is_row_zero_grep_is_first_match() {
        let results = fixture();
        let none = HashSet::new();
        let flat = build_rows(&results, &none, false);
        assert_eq!(first_selectable(&flat, &none, &results).unwrap_or(0), 0);
        let grouped = build_rows(&results, &none, true);
        assert_eq!(first_selectable(&grouped, &none, &results).unwrap_or(0), 1);
        assert_eq!(first_selectable(&[], &none, &[]).unwrap_or(0), 0);
    }

    // anchor_selection

    #[test]
    fn anchor_selection_match_survives_rebuild() {
        let results = fixture();
        let none = HashSet::new();
        let rows = build_rows(&results, &none, true);
        // (b, match 0) is row 5 in fixture A.
        assert_eq!(
            anchor_selection(Some((Path::new("/b.rs"), 0)), &rows, &none, &results),
            5
        );
        // Files mode: (c, 0) is the third flat row.
        let flat = build_rows(&results, &none, false);
        assert_eq!(
            anchor_selection(Some((Path::new("/c.rs"), 0)), &flat, &none, &results),
            2
        );
    }

    #[test]
    fn anchor_selection_collapsed_file_anchors_to_header() {
        let results = fixture();
        let collapsed = collapsed_set(&["/a.rs"]);
        let rows = build_rows(&results, &collapsed, true);
        // Layout: 0:H0 1:Sep 2:H1 3:M10 4:Sep 5:H2 6:M20 7:M21
        assert_eq!(
            anchor_selection(Some((Path::new("/a.rs"), 1)), &rows, &collapsed, &results),
            0
        );
    }

    #[test]
    fn anchor_selection_gone_match_clamps_to_files_last_match() {
        let results = fixture();
        let none = HashSet::new();
        let rows = build_rows(&results, &none, true);
        // a now has 2 matches; old match index 5 is gone -> last match row (2).
        assert_eq!(
            anchor_selection(Some((Path::new("/a.rs"), 5)), &rows, &none, &results),
            2
        );
    }

    #[test]
    fn anchor_selection_removed_file_falls_back_to_nearest_selectable() {
        // a was removed by the rebuild; nearest = first selectable row.
        let results = vec![snap("/b.rs", &[2]), snap("/c.rs", &[7, 9])];
        let none = HashSet::new();
        let rows = build_rows(&results, &none, true);
        // Layout: 0:H0 1:M00 2:Sep 3:H1 4:M10 5:M11
        assert_eq!(
            anchor_selection(Some((Path::new("/a.rs"), 0)), &rows, &none, &results),
            1
        );
    }

    #[test]
    fn anchor_selection_without_old_key_picks_first_selectable() {
        let results = fixture();
        let none = HashSet::new();
        let rows = build_rows(&results, &none, true);
        assert_eq!(anchor_selection(None, &rows, &none, &results), 1);
        // Nothing selectable at all falls back to 0.
        assert_eq!(anchor_selection(None, &[], &none, &[]), 0);
    }

    // group_match_range

    #[test]
    fn group_match_range_expanded_covers_match_rows() {
        let results = fixture();
        let rows = build_rows(&results, &HashSet::new(), true);
        // Fixture A layout: 0:H0 1:M00 2:M01 3:Sep 4:H1 5:M10 6:Sep 7:H2 8:M20 9:M21
        assert_eq!(group_match_range(&rows, 0), Some(1..3));
        assert_eq!(group_match_range(&rows, 1), Some(5..6));
        assert_eq!(group_match_range(&rows, 2), Some(8..10));
    }

    #[test]
    fn group_match_range_collapsed_group_is_empty_after_header() {
        let results = fixture();
        let rows = build_rows(&results, &collapsed_set(&["/b.rs"]), true);
        // Fixture B layout: 0:H0 1:M00 2:M01 3:Sep 4:H1 5:Sep 6:H2 7:M20 8:M21
        assert_eq!(group_match_range(&rows, 1), Some(5..5));
        assert_eq!(group_match_range(&rows, 0), Some(1..3));
        assert_eq!(group_match_range(&rows, 2), Some(7..9));
    }

    #[test]
    fn group_match_range_missing_header_is_none() {
        let results = fixture();
        // Files-mode flat rows have no headers.
        let flat = build_rows(&results, &HashSet::new(), false);
        assert_eq!(group_match_range(&flat, 0), None);
        // File index beyond the results.
        let rows = build_rows(&results, &HashSet::new(), true);
        assert_eq!(group_match_range(&rows, 3), None);
        assert_eq!(group_match_range(&[], 0), None);
    }

    // Splice contract for a single-group toggle: the old range comes from the
    // pre-toggle rows, the new count from the post-toggle rows; the start is
    // identical in both because rows before the header are untouched.
    #[test]
    fn group_match_range_collapse_and_expand_splice_math() {
        let results = fixture();
        let expanded = build_rows(&results, &HashSet::new(), true);
        let folded = build_rows(&results, &collapsed_set(&["/a.rs"]), true);
        // Collapse a: splice(1..3, 0).
        let old = group_match_range(&expanded, 0).unwrap();
        let new = group_match_range(&folded, 0).unwrap();
        assert_eq!((old, new.len()), (1..3, 0));
        // Expand a again: splice(1..1, 2).
        let old = group_match_range(&folded, 0).unwrap();
        let new = group_match_range(&expanded, 0).unwrap();
        assert_eq!((old, new.len()), (1..1, 2));
    }

    // toggle_all_collapsed

    #[test]
    fn toggle_all_collapses_everything_when_nothing_is_collapsed() {
        let results = fixture();
        assert_eq!(
            toggle_all_collapsed(&HashSet::new(), &results),
            collapsed_set(&["/a.rs", "/b.rs", "/c.rs"])
        );
    }

    #[test]
    fn toggle_all_expands_everything_when_any_group_is_collapsed() {
        let results = fixture();
        assert!(toggle_all_collapsed(&collapsed_set(&["/b.rs"]), &results).is_empty());
        assert!(
            toggle_all_collapsed(&collapsed_set(&["/a.rs", "/b.rs", "/c.rs"]), &results).is_empty()
        );
    }

    #[test]
    fn toggle_all_empty_results_stays_empty() {
        assert!(toggle_all_collapsed(&HashSet::new(), &[]).is_empty());
    }

    // Fold re-anchor scenarios: the picker captures the cursor's (path, match)
    // key before a toggle (header rows key as match 0) and feeds it through
    // anchor_selection after rebuilding.

    #[test]
    fn fold_toggle_all_collapse_anchors_cursor_to_its_own_header() {
        let results = fixture();
        // Cursor on (b, match 0); collapse-all.
        let all = collapsed_set(&["/a.rs", "/b.rs", "/c.rs"]);
        let rows = build_rows(&results, &all, true);
        // Fixture C layout: 0:H0 1:Sep 2:H1 3:Sep 4:H2
        assert_eq!(
            anchor_selection(Some((Path::new("/b.rs"), 0)), &rows, &all, &results),
            2
        );
    }

    #[test]
    fn fold_toggle_all_expand_anchors_cursor_to_groups_first_match() {
        let results = fixture();
        // Cursor was on b's header while all-collapsed (keyed as match 0);
        // expand-all lands on b's first match row (5 in fixture A).
        let none = HashSet::new();
        let rows = build_rows(&results, &none, true);
        assert_eq!(
            anchor_selection(Some((Path::new("/b.rs"), 0)), &rows, &none, &results),
            5
        );
    }

    #[test]
    fn fold_other_group_keeps_cursor_on_its_shifted_row() {
        let results = fixture();
        // Cursor on (c, match 0) while a collapses via its chevron.
        let collapsed = collapsed_set(&["/a.rs"]);
        let rows = build_rows(&results, &collapsed, true);
        // Layout: 0:H0 1:Sep 2:H1 3:M10 4:Sep 5:H2 6:M20 7:M21
        assert_eq!(
            anchor_selection(Some((Path::new("/c.rs"), 0)), &rows, &collapsed, &results),
            6
        );
    }

    // selection_key_for_row

    #[test]
    fn selection_key_grep_match_rows_key_line_and_col() {
        let results = vec![snap_with_cols("/a.rs", &[(1, 4), (5, 0)])];
        let rows = build_rows(&results, &HashSet::new(), true);
        // Layout: 0:H0 1:M00 2:M01
        assert_eq!(
            selection_key_for_row(&rows, 1, &results, true),
            Some((PathBuf::from("/a.rs"), Some((1, 4))))
        );
        assert_eq!(
            selection_key_for_row(&rows, 2, &results, true),
            Some((PathBuf::from("/a.rs"), Some((5, 0))))
        );
    }

    #[test]
    fn selection_key_files_rows_key_path_only() {
        let results = fixture();
        let rows = build_rows(&results, &HashSet::new(), false);
        assert_eq!(
            selection_key_for_row(&rows, 0, &results, false),
            Some((PathBuf::from("/a.rs"), None))
        );
        assert_eq!(
            selection_key_for_row(&rows, 2, &results, false),
            Some((PathBuf::from("/c.rs"), None))
        );
    }

    // Toggling on a header row is a no-op: headers (expanded AND collapsed —
    // the collapsed ones are cursor-selectable), separators, and out-of-range
    // indices carry no selection key.
    #[test]
    fn selection_key_header_separator_and_out_of_range_have_none() {
        let results = fixture();
        let none = HashSet::new();
        let rows = build_rows(&results, &none, true);
        // Fixture A layout: 0:H0 (expanded header) 3:Sep
        assert_eq!(selection_key_for_row(&rows, 0, &results, true), None);
        assert_eq!(selection_key_for_row(&rows, 3, &results, true), None);
        assert_eq!(
            selection_key_for_row(&rows, usize::MAX, &results, true),
            None
        );
        let collapsed = collapsed_set(&["/a.rs"]);
        let rows = build_rows(&results, &collapsed, true);
        assert_eq!(selection_key_for_row(&rows, 0, &results, true), None);
    }

    // SelectionKey ordering in a BTreeSet

    #[test]
    fn selection_keys_order_by_path_then_line_then_col() {
        let mut set: BTreeSet<SelectionKey> = BTreeSet::new();
        set.insert((PathBuf::from("/b.rs"), Some((1, 0))));
        set.insert((PathBuf::from("/a.rs"), Some((9, 2))));
        set.insert((PathBuf::from("/a.rs"), Some((2, 7))));
        set.insert((PathBuf::from("/a.rs"), Some((2, 3))));
        set.insert((PathBuf::from("/a.rs"), None));
        assert_eq!(
            set.into_iter().collect::<Vec<_>>(),
            vec![
                (PathBuf::from("/a.rs"), None), // whole-file key sorts first
                (PathBuf::from("/a.rs"), Some((2, 3))),
                (PathBuf::from("/a.rs"), Some((2, 7))),
                (PathBuf::from("/a.rs"), Some((9, 2))),
                (PathBuf::from("/b.rs"), Some((1, 0))),
            ]
        );
    }

    #[test]
    fn selection_keys_same_line_different_cols_are_distinct() {
        let mut set: BTreeSet<SelectionKey> = BTreeSet::new();
        assert!(set.insert((PathBuf::from("/a.rs"), Some((3, 0)))));
        assert!(set.insert((PathBuf::from("/a.rs"), Some((3, 8)))));
        assert_eq!(set.len(), 2);
    }

    // key_survives / prune

    #[test]
    fn key_survives_grep_key_requires_exact_line_and_col() {
        let results = vec![snap_with_cols("/a.rs", &[(3, 4)])];
        assert!(key_survives(
            &(PathBuf::from("/a.rs"), Some((3, 4))),
            &results
        ));
        // Col drifted, line gone, file gone: all pruned.
        assert!(!key_survives(
            &(PathBuf::from("/a.rs"), Some((3, 5))),
            &results
        ));
        assert!(!key_survives(
            &(PathBuf::from("/a.rs"), Some((4, 4))),
            &results
        ));
        assert!(!key_survives(
            &(PathBuf::from("/b.rs"), Some((3, 4))),
            &results
        ));
    }

    #[test]
    fn key_survives_files_key_requires_path_only() {
        let results = fixture();
        assert!(key_survives(&(PathBuf::from("/a.rs"), None), &results));
        assert!(!key_survives(&(PathBuf::from("/zzz.rs"), None), &results));
        assert!(!key_survives(&(PathBuf::from("/a.rs"), None), &[]));
    }

    #[test]
    fn prune_retains_only_surviving_keys() {
        let mut selection: BTreeSet<SelectionKey> = BTreeSet::new();
        selection.insert((PathBuf::from("/a.rs"), Some((1, 0))));
        selection.insert((PathBuf::from("/a.rs"), Some((99, 0))));
        selection.insert((PathBuf::from("/gone.rs"), Some((2, 0))));
        let results = vec![snap_with_cols("/a.rs", &[(1, 0), (5, 2)])];
        selection.retain(|key| key_survives(key, &results));
        assert_eq!(
            selection.into_iter().collect::<Vec<_>>(),
            vec![(PathBuf::from("/a.rs"), Some((1, 0)))]
        );
    }

    // toggle_all_selection

    #[test]
    fn toggle_all_selection_grep_selects_every_match_triple() {
        let results = vec![
            snap_with_cols("/a.rs", &[(1, 0), (1, 6)]), // same line, two cols
            snap_with_cols("/b.rs", &[(2, 3)]),
        ];
        assert_eq!(
            toggle_all_selection(&BTreeSet::new(), &results, true)
                .into_iter()
                .collect::<Vec<_>>(),
            vec![
                (PathBuf::from("/a.rs"), Some((1, 0))),
                (PathBuf::from("/a.rs"), Some((1, 6))),
                (PathBuf::from("/b.rs"), Some((2, 3))),
            ]
        );
    }

    #[test]
    fn toggle_all_selection_files_selects_per_file_keys() {
        let results = fixture();
        assert_eq!(
            toggle_all_selection(&BTreeSet::new(), &results, false)
                .into_iter()
                .collect::<Vec<_>>(),
            vec![
                (PathBuf::from("/a.rs"), None),
                (PathBuf::from("/b.rs"), None),
                (PathBuf::from("/c.rs"), None),
            ]
        );
    }

    #[test]
    fn toggle_all_selection_any_selected_clears_everything() {
        let results = fixture();
        let mut partial: BTreeSet<SelectionKey> = BTreeSet::new();
        partial.insert((PathBuf::from("/a.rs"), Some((1, 0))));
        assert!(toggle_all_selection(&partial, &results, true).is_empty());
        // Nothing visible and nothing selected stays empty.
        assert!(toggle_all_selection(&BTreeSet::new(), &[], true).is_empty());
    }

    // toggle_multi_select_mode

    #[test]
    fn toggle_mode_off_clears_selection_on_keeps_it() {
        let mut selection: BTreeSet<SelectionKey> = BTreeSet::new();
        selection.insert((PathBuf::from("/a.rs"), Some((1, 0))));
        // ON -> OFF clears the selection.
        let (mode, cleared) = toggle_multi_select_mode(true, &selection);
        assert!(!mode);
        assert!(cleared.is_empty());
        // OFF -> ON keeps whatever is there (normally already empty).
        let (mode, kept) = toggle_multi_select_mode(false, &selection);
        assert!(mode);
        assert_eq!(kept, selection);
    }

    // dedupe_opens

    #[test]
    fn dedupe_opens_first_selected_match_per_file() {
        let mut selection: BTreeSet<SelectionKey> = BTreeSet::new();
        selection.insert((PathBuf::from("/a.rs"), Some((10, 4))));
        selection.insert((PathBuf::from("/a.rs"), Some((3, 7))));
        selection.insert((PathBuf::from("/a.rs"), Some((3, 0))));
        selection.insert((PathBuf::from("/b.rs"), Some((5, 1))));
        assert_eq!(
            dedupe_opens(&selection),
            vec![
                (PathBuf::from("/a.rs"), Some((3, 0))),
                (PathBuf::from("/b.rs"), Some((5, 1))),
            ]
        );
    }

    #[test]
    fn dedupe_opens_files_mode_keys_pass_through_sorted() {
        let mut selection: BTreeSet<SelectionKey> = BTreeSet::new();
        selection.insert((PathBuf::from("/b.txt"), None));
        selection.insert((PathBuf::from("/a.txt"), None));
        assert_eq!(
            dedupe_opens(&selection),
            vec![
                (PathBuf::from("/a.txt"), None),
                (PathBuf::from("/b.txt"), None),
            ]
        );
    }

    #[test]
    fn dedupe_opens_empty_selection_yields_nothing() {
        assert!(dedupe_opens(&BTreeSet::new()).is_empty());
    }

    // max_line_number

    #[test]
    fn max_line_number_spans_all_files() {
        let results = vec![snap("/a.rs", &[1, 5]), snap("/b.rs", &[200, 42])];
        assert_eq!(max_line_number(&results), 200);
    }

    #[test]
    fn max_line_number_zero_without_matches() {
        assert_eq!(max_line_number(&[]), 0);
        assert_eq!(max_line_number(&[snap("/a.rs", &[])]), 0);
    }
}
