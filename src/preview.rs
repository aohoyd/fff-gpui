use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex, OnceLock};

use crate::theme;

use tree_sitter::Language;
use tree_sitter_highlight::{HighlightConfiguration, HighlightEvent, Highlighter};

pub const MAX_PREVIEW_LINES: usize = 500;

#[derive(Clone)]
pub struct HighlightedSpan {
    pub color: u32,
    pub bg: Option<u32>,
    pub italic: bool,
    pub bold: bool,
    pub underline: bool,
    pub strikethrough: bool,
    // True only for chunks produced by the grep-match overlay
    // (`overlay_match_ranges`). Syntax-derived spans are always `false`, even
    // when a theme gives a capture a `background_color` — so `line_has_match`
    // can tell a real grep hit apart from a theme-painted token.
    pub matched: bool,
    pub text: String,
}

#[derive(Clone)]
pub struct HighlightedLine {
    pub spans: Vec<HighlightedSpan>,
}

struct TreeSitterLanguageSpec {
    language: Language,
    name: &'static str,
    highlights_query: &'static str,
    injections_query: &'static str,
    locals_query: &'static str,
}

fn syntax_set_for_path(path: &Path) -> Option<TreeSitterLanguageSpec> {
    let extension = path.extension()?.to_string_lossy().to_ascii_lowercase();

    match extension.as_str() {
        "rs" => Some(TreeSitterLanguageSpec {
            language: tree_sitter_rust::LANGUAGE.into(),
            name: "rust",
            highlights_query: tree_sitter_rust::HIGHLIGHTS_QUERY,
            injections_query: tree_sitter_rust::INJECTIONS_QUERY,
            locals_query: "",
        }),
        "swift" => Some(TreeSitterLanguageSpec {
            language: tree_sitter_swift::LANGUAGE.into(),
            name: "swift",
            highlights_query: tree_sitter_swift::HIGHLIGHTS_QUERY,
            injections_query: tree_sitter_swift::INJECTIONS_QUERY,
            locals_query: tree_sitter_swift::LOCALS_QUERY,
        }),
        "js" | "mjs" | "cjs" | "jsx" => Some(TreeSitterLanguageSpec {
            language: tree_sitter_javascript::LANGUAGE.into(),
            name: "javascript",
            highlights_query: if extension == "jsx" {
                tree_sitter_javascript::JSX_HIGHLIGHT_QUERY
            } else {
                tree_sitter_javascript::HIGHLIGHT_QUERY
            },
            injections_query: tree_sitter_javascript::INJECTIONS_QUERY,
            locals_query: tree_sitter_javascript::LOCALS_QUERY,
        }),
        "ts" => Some(TreeSitterLanguageSpec {
            language: tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
            name: "typescript",
            highlights_query: tree_sitter_typescript::HIGHLIGHTS_QUERY,
            injections_query: "",
            locals_query: tree_sitter_typescript::LOCALS_QUERY,
        }),
        "tsx" => Some(TreeSitterLanguageSpec {
            language: tree_sitter_typescript::LANGUAGE_TSX.into(),
            name: "tsx",
            highlights_query: tree_sitter_typescript::HIGHLIGHTS_QUERY,
            injections_query: "",
            locals_query: tree_sitter_typescript::LOCALS_QUERY,
        }),
        "go" => Some(TreeSitterLanguageSpec {
            language: tree_sitter_go::LANGUAGE.into(),
            name: "go",
            highlights_query: tree_sitter_go::HIGHLIGHTS_QUERY,
            injections_query: "",
            locals_query: "",
        }),
        "py" => Some(TreeSitterLanguageSpec {
            language: tree_sitter_python::LANGUAGE.into(),
            name: "python",
            highlights_query: tree_sitter_python::HIGHLIGHTS_QUERY,
            injections_query: "",
            locals_query: "",
        }),
        "json" | "jsonc" => Some(TreeSitterLanguageSpec {
            language: tree_sitter_json::LANGUAGE.into(),
            name: "json",
            highlights_query: tree_sitter_json::HIGHLIGHTS_QUERY,
            injections_query: "",
            locals_query: "",
        }),
        "yaml" | "yml" => Some(TreeSitterLanguageSpec {
            language: tree_sitter_yaml::LANGUAGE.into(),
            name: "yaml",
            highlights_query: tree_sitter_yaml::HIGHLIGHTS_QUERY,
            injections_query: "",
            locals_query: "",
        }),
        "sh" | "bash" | "zsh" => Some(TreeSitterLanguageSpec {
            language: tree_sitter_bash::LANGUAGE.into(),
            name: "bash",
            highlights_query: tree_sitter_bash::HIGHLIGHT_QUERY,
            injections_query: "",
            locals_query: "",
        }),
        "c" | "h" => Some(TreeSitterLanguageSpec {
            language: tree_sitter_c::LANGUAGE.into(),
            name: "c",
            highlights_query: tree_sitter_c::HIGHLIGHT_QUERY,
            injections_query: "",
            locals_query: "",
        }),
        "cc" | "cpp" | "cxx" | "hpp" | "hh" | "hxx" => Some(TreeSitterLanguageSpec {
            language: tree_sitter_cpp::LANGUAGE.into(),
            name: "cpp",
            highlights_query: tree_sitter_cpp::HIGHLIGHT_QUERY,
            injections_query: "",
            locals_query: "",
        }),
        "css" => Some(TreeSitterLanguageSpec {
            language: tree_sitter_css::LANGUAGE.into(),
            name: "css",
            highlights_query: tree_sitter_css::HIGHLIGHTS_QUERY,
            injections_query: "",
            locals_query: "",
        }),
        "html" | "htm" => Some(TreeSitterLanguageSpec {
            language: tree_sitter_html::LANGUAGE.into(),
            name: "html",
            highlights_query: tree_sitter_html::HIGHLIGHTS_QUERY,
            injections_query: tree_sitter_html::INJECTIONS_QUERY,
            locals_query: "",
        }),
        "md" | "markdown" => Some(TreeSitterLanguageSpec {
            language: tree_sitter_md::LANGUAGE.into(),
            name: "markdown",
            highlights_query: tree_sitter_md::HIGHLIGHT_QUERY_BLOCK,
            injections_query: tree_sitter_md::INJECTION_QUERY_BLOCK,
            locals_query: "",
        }),
        "regex" | "re" => Some(TreeSitterLanguageSpec {
            language: tree_sitter_regex::LANGUAGE.into(),
            name: "regex",
            highlights_query: tree_sitter_regex::HIGHLIGHTS_QUERY,
            injections_query: "",
            locals_query: "",
        }),
        _ => None,
    }
}

fn build_highlight_config(spec: &TreeSitterLanguageSpec) -> Option<HighlightConfiguration> {
    let mut config = HighlightConfiguration::new(
        spec.language.clone(),
        spec.name,
        spec.highlights_query,
        spec.injections_query,
        spec.locals_query,
    )
    .ok()?;
    let capture_names: Vec<String> = config
        .query
        .capture_names()
        .into_iter()
        .map(|name| name.to_string())
        .collect();
    config.configure(&capture_names);
    Some(config)
}

// Compiling a `HighlightConfiguration` (full highlight/injection/locals query
// compilation) costs ~10ms per language. The grep path highlights one line per
// match, so without a cache a single results page would recompile the same
// grammar hundreds of times. Cache one `Arc<HighlightConfiguration>` per
// language spec name (a `&'static str`); both the preview and per-line paths go
// through `highlighted_lines_with`, so both share the cache. A `None` result
// (query compilation failed) is cached too, so a broken grammar is not retried
// on every call.
fn cached_highlight_config(spec: &TreeSitterLanguageSpec) -> Option<Arc<HighlightConfiguration>> {
    static CACHE: OnceLock<Mutex<HashMap<&'static str, Option<Arc<HighlightConfiguration>>>>> =
        OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    let mut guard = cache.lock().expect("highlight config cache poisoned");
    guard
        .entry(spec.name)
        .or_insert_with(|| build_highlight_config(spec).map(Arc::new))
        .clone()
}

fn append_span(line: &mut HighlightedLine, style: theme::SyntaxRenderStyle, text: &str) {
    if text.is_empty() {
        return;
    }

    if let Some(last) = line.spans.last_mut()
        && last.color == style.color
        && last.bg == style.bg
        && last.italic == style.italic
        && last.bold == style.bold
        && last.underline == style.underline
        && last.strikethrough == style.strikethrough
    {
        last.text.push_str(text);
        return;
    }

    line.spans.push(HighlightedSpan {
        color: style.color,
        bg: style.bg,
        italic: style.italic,
        bold: style.bold,
        underline: style.underline,
        strikethrough: style.strikethrough,
        matched: false,
        text: text.to_string(),
    });
}

fn push_text(lines: &mut Vec<HighlightedLine>, mut text: &str, style: theme::SyntaxRenderStyle) {
    while !text.is_empty() {
        let newline = text.find('\n');
        match newline {
            Some(idx) => {
                let (head, tail) = text.split_at(idx);
                let head = head.strip_suffix('\r').unwrap_or(head);
                append_span(
                    lines.last_mut().expect("at least one line exists"),
                    style,
                    head,
                );
                lines.push(HighlightedLine { spans: Vec::new() });
                text = &tail[1..];
            }
            None => {
                let text = text.strip_suffix('\r').unwrap_or(text);
                append_span(
                    lines.last_mut().expect("at least one line exists"),
                    style,
                    text,
                );
                return;
            }
        }
    }
}

fn highlighted_lines(content: &str, spec: &TreeSitterLanguageSpec) -> Option<Vec<HighlightedLine>> {
    highlighted_lines_with(content, spec, theme::syntax_render_style)
}

// Core of `highlighted_lines` with capture-name style resolution injected, so
// tests can observe capture boundaries independently of the process-global
// theme (whose default syntax table maps every capture to one style, merging
// adjacent spans).
fn highlighted_lines_with(
    content: &str,
    spec: &TreeSitterLanguageSpec,
    style_for: impl Fn(&str) -> theme::SyntaxRenderStyle,
) -> Option<Vec<HighlightedLine>> {
    if content.is_empty() {
        return Some(Vec::new());
    }

    let mut highlighter = Highlighter::new();
    let config = cached_highlight_config(spec)?;
    let capture_names = config.query.capture_names();
    let mut lines = vec![HighlightedLine { spans: Vec::new() }];
    let default_style = style_for("text");
    let mut style_stack = vec![default_style];

    let events = highlighter
        .highlight(config.as_ref(), content.as_bytes(), None, |_| None)
        .ok()?;

    for event in events {
        match event.ok()? {
            HighlightEvent::Source { start, end } => {
                let style = style_stack.last().copied().unwrap_or(default_style);
                push_text(&mut lines, &content[start..end], style);
            }
            HighlightEvent::HighlightStart(highlight) => {
                let capture_name = capture_names.get(highlight.0).copied().unwrap_or("text");
                style_stack.push(style_for(capture_name));
            }
            HighlightEvent::HighlightEnd => {
                if style_stack.len() > 1 {
                    style_stack.pop();
                }
            }
        }
    }

    Some(lines)
}

fn plain_text_lines(content: &str) -> Vec<HighlightedLine> {
    let mut lines = Vec::new();
    let mut current = HighlightedLine { spans: Vec::new() };

    for chunk in content.split_inclusive('\n') {
        let chunk = chunk.strip_suffix('\n').unwrap_or(chunk);
        let chunk = chunk.strip_suffix('\r').unwrap_or(chunk);
        if !chunk.is_empty() {
            let style = theme::syntax_render_style("text");
            current.spans.push(HighlightedSpan {
                color: style.color,
                bg: style.bg,
                italic: style.italic,
                bold: style.bold,
                underline: style.underline,
                strikethrough: style.strikethrough,
                matched: false,
                text: chunk.to_string(),
            });
        }
        lines.push(current);
        current = HighlightedLine { spans: Vec::new() };
    }

    if lines.is_empty() && !content.is_empty() {
        lines.push(current);
    }

    lines
}

fn slice_preview_lines(
    lines: Vec<HighlightedLine>,
    start_line: usize,
) -> (usize, Vec<HighlightedLine>) {
    let start = start_line.saturating_sub(1);
    let preview = lines
        .into_iter()
        .skip(start)
        .take(MAX_PREVIEW_LINES)
        .collect();
    (start_line, preview)
}

fn syntax_lines_for_path(path: &Path, content: &str) -> Vec<HighlightedLine> {
    if content.is_empty() {
        return Vec::new();
    }

    let Some(spec) = syntax_set_for_path(path) else {
        return plain_text_lines(content);
    };

    highlighted_lines(content, &spec).unwrap_or_else(|| plain_text_lines(content))
}

// Syntax-highlight a single line of text on its own. Language is picked from
// the path's extension; unknown extensions and highlighter failures fall back
// to one plain span covering the whole line (built into
// `syntax_lines_for_path`), and empty input yields no spans.
pub fn highlight_single_line(path: &Path, line: &str) -> Vec<HighlightedSpan> {
    syntax_lines_for_path(path, line)
        .into_iter()
        .next()
        .map(|line| line.spans)
        .unwrap_or_default()
}

// Warm the syntax and theme caches before the first preview render.
pub fn warm_highlighter() {
    let _ = theme::palette();
    let _ = theme::syntax_color("keyword");
}

// Overlay grep match ranges onto already-highlighted spans.
pub fn overlay_match_ranges(
    spans: &[HighlightedSpan],
    byte_ranges: &[(u32, u32)],
    match_bg: Option<u32>,
) -> Vec<HighlightedSpan> {
    if byte_ranges.is_empty() {
        return spans.to_vec();
    }

    let mut sorted_ranges = byte_ranges.to_vec();
    sorted_ranges.sort_unstable_by_key(|&(s, _)| s);

    let mut result = Vec::new();
    let mut byte_pos: u32 = 0;

    for span in spans {
        let span_start = byte_pos;
        let span_end = span_start + span.text.len() as u32;
        let mut chunk_start = span_start;

        for &(range_start, range_end) in &sorted_ranges {
            let overlap_start = range_start.max(chunk_start);
            let overlap_end = range_end.min(span_end);

            if overlap_start >= overlap_end {
                continue;
            }

            let pre_s = (chunk_start - span_start) as usize;
            let pre_e = (overlap_start - span_start) as usize;
            if let Some((pre_s, pre_e)) = clamp_range_to_char_boundaries(&span.text, pre_s, pre_e) {
                result.push(HighlightedSpan {
                    color: span.color,
                    bg: span.bg,
                    italic: span.italic,
                    bold: span.bold,
                    underline: span.underline,
                    strikethrough: span.strikethrough,
                    matched: span.matched,
                    text: span.text[pre_s..pre_e].to_string(),
                });
            }

            let hi_s = (overlap_start - span_start) as usize;
            let hi_e = (overlap_end - span_start) as usize;
            if let Some((hi_s, hi_e)) = clamp_range_to_char_boundaries(&span.text, hi_s, hi_e) {
                result.push(HighlightedSpan {
                    color: span.color,
                    bg: match_bg.or(span.bg),
                    italic: span.italic,
                    // Matched substrings render bold on top of the flat
                    // search-match background (Zed-style emphasis).
                    bold: true,
                    underline: span.underline,
                    strikethrough: span.strikethrough,
                    matched: true,
                    text: span.text[hi_s..hi_e].to_string(),
                });
            }

            chunk_start = overlap_end;
        }

        let tail_s = (chunk_start - span_start) as usize;
        if let Some((tail_s, tail_e)) =
            clamp_range_to_char_boundaries(&span.text, tail_s, span.text.len())
        {
            result.push(HighlightedSpan {
                color: span.color,
                bg: span.bg,
                italic: span.italic,
                bold: span.bold,
                underline: span.underline,
                strikethrough: span.strikethrough,
                matched: span.matched,
                text: span.text[tail_s..tail_e].to_string(),
            });
        }

        byte_pos = span_end;
    }

    result
}

fn clamp_range_to_char_boundaries(text: &str, start: usize, end: usize) -> Option<(usize, usize)> {
    let mut start = start.min(text.len());
    let mut end = end.min(text.len());

    while start > 0 && !text.is_char_boundary(start) {
        start -= 1;
    }
    while end < text.len() && !text.is_char_boundary(end) {
        end += 1;
    }

    (start < end).then_some((start, end))
}

// A line carries a grep match when any of its spans was produced by the
// match overlay (`overlay_match_ranges` sets `matched: true` on exactly those
// chunks). Span backgrounds are NOT a reliable signal here: a theme (or
// `theme_overrides`) can give a syntax capture a `background_color`, so
// `bg.is_some()` would misidentify unrelated tokens as matches.
pub fn line_has_match(line: &HighlightedLine) -> bool {
    line.spans.iter().any(|span| span.matched)
}

// 1-based start line for the `MAX_PREVIEW_LINES` window, centered on
// `center_line` (also 1-based). Files that fit entirely in the window, or that
// have no match to center on, start at line 1. When centering, the window is
// clamped so it never runs past the last line, giving a deep match ~250 lines
// of context on each side. Pure — unit-tested below.
pub fn window_start_line(center_line: Option<usize>, total_lines: usize) -> usize {
    if total_lines <= MAX_PREVIEW_LINES {
        return 1;
    }
    let Some(center) = center_line else {
        return 1;
    };
    let start0 = crate::layout::scroll_center_row(center.saturating_sub(1), MAX_PREVIEW_LINES);
    let max_start0 = total_lines - MAX_PREVIEW_LINES;
    start0.min(max_start0) + 1
}

// Map a match's 1-based file line number to its row index within a preview
// window that starts at `start_line` (also 1-based). Matches BEFORE the window
// return `None` so they never collapse onto row 0 (a saturating subtraction
// would clamp them there, stacking a bogus highlight onto the window's real
// first line); a match AT the window start maps to `Some(0)`, and one inside
// maps to `Some(n)`. Matches past the window's end still map to some `Some(n)`,
// which the caller's `get_mut` bounds-check discards. Pure — unit-tested below.
pub fn overlay_row_index(line_number: usize, start_line: usize) -> Option<usize> {
    line_number.checked_sub(start_line)
}

// Read a file and return a syntax-highlighted preview window centered on the
// match line (if any).
pub fn highlight_file_window(
    path: &Path,
    center_line: Option<usize>,
) -> (usize, Vec<HighlightedLine>) {
    let content = match std::fs::read_to_string(path) {
        Ok(content) => content,
        Err(_) => return (1, vec![]),
    };

    let lines = syntax_lines_for_path(path, &content);
    let start_line = window_start_line(center_line, lines.len());

    slice_preview_lines(lines, start_line)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tree_sitter_capture_name_aliases_resolve_to_zed_tokens() {
        assert_eq!(
            theme::syntax_color("comment.documentation"),
            theme::syntax_color("comment")
        );
    }

    #[test]
    fn tree_sitter_highlight_map_detects_typescript_files() {
        let spec =
            syntax_set_for_path(Path::new("component.ts")).expect("typescript should be supported");

        assert_eq!(spec.name, "typescript");
    }

    #[test]
    fn highlight_config_cache_returns_same_arc() {
        // Two independent specs for the same language must resolve to one shared
        // compiled config, so the ~10ms compile happens at most once per grammar
        // rather than once per grep match line.
        let first = syntax_set_for_path(Path::new("a.rs")).expect("rust spec");
        let second = syntax_set_for_path(Path::new("b.rs")).expect("rust spec");
        let a = cached_highlight_config(&first).expect("config builds");
        let b = cached_highlight_config(&second).expect("config builds");
        assert!(
            Arc::ptr_eq(&a, &b),
            "cache should hand back the same Arc for the same language"
        );
    }

    fn span(bg: Option<u32>) -> HighlightedSpan {
        span_matched(bg, false)
    }

    fn span_matched(bg: Option<u32>, matched: bool) -> HighlightedSpan {
        HighlightedSpan {
            color: 0,
            bg,
            italic: false,
            bold: false,
            underline: false,
            strikethrough: false,
            matched,
            text: "x".to_string(),
        }
    }

    #[test]
    fn line_has_match_tracks_match_flag_not_background() {
        // Empty line: no match.
        assert!(!line_has_match(&HighlightedLine { spans: vec![] }));
        // Plain syntax spans (no match overlay): no match.
        assert!(!line_has_match(&HighlightedLine {
            spans: vec![span(None), span(None)]
        }));
        // Theme-derived backgrounds (e.g. a capture's `background_color`) must
        // NOT register as a grep match — regression guard for overloading
        // `bg.is_some()`.
        assert!(!line_has_match(&HighlightedLine {
            spans: vec![span(None), span_matched(Some(0x123456), false)]
        }));
        // A chunk flagged by the overlay/match path is a real match, even when
        // it carries no distinct background.
        assert!(line_has_match(&HighlightedLine {
            spans: vec![span(None), span_matched(None, true)]
        }));
    }

    #[test]
    fn window_start_line_starts_at_one_when_file_fits() {
        // File shorter than the window: always line 1, even with a match.
        assert_eq!(window_start_line(Some(300), 400), 1);
        assert_eq!(window_start_line(None, 400), 1);
        // Exactly the window size still fits.
        assert_eq!(window_start_line(Some(400), MAX_PREVIEW_LINES), 1);
    }

    #[test]
    fn window_start_line_centers_deep_matches() {
        // 1200-line file, match at line 600 → centered window (249 above).
        // scroll_center_row(599, 500) = 599 - 249 = 350, +1 = 351.
        assert_eq!(window_start_line(Some(600), 1200), 351);
        // No match in a long file → start at line 1.
        assert_eq!(window_start_line(None, 1200), 1);
    }

    #[test]
    fn window_start_line_clamps_to_last_window() {
        // Match near the end must not scroll past the final window.
        // 1200 lines, window 500 → max start is line 701.
        assert_eq!(window_start_line(Some(1190), 1200), 701);
        assert_eq!(window_start_line(Some(1200), 1200), 701);
    }

    #[test]
    fn window_start_line_clamps_near_top() {
        // Match near the start centers no further left than line 1.
        assert_eq!(window_start_line(Some(10), 1200), 1);
    }

    #[test]
    fn overlay_row_index_maps_lines_relative_to_window() {
        // Match before the window: no row (must NOT collapse onto row 0).
        assert_eq!(overlay_row_index(5, 10), None);
        // Match on the window's first line: row 0.
        assert_eq!(overlay_row_index(10, 10), Some(0));
        // Match inside the window: offset from the start line.
        assert_eq!(overlay_row_index(15, 10), Some(5));
        // Window starting at line 1 maps 1-based lines to 0-based rows.
        assert_eq!(overlay_row_index(1, 1), Some(0));
        assert_eq!(overlay_row_index(7, 1), Some(6));
    }

    #[test]
    fn overlay_match_ranges_makes_matches_bold() {
        let spans = vec![span(None)];
        // "x" is one byte; match the whole span.
        let out = overlay_match_ranges(&spans, &[(0, 1)], Some(0x112233));
        assert!(out.iter().any(|s| s.bg == Some(0x112233) && s.bold));
        // Only the overlay path sets `matched`, and it does so on the matched
        // chunk (this drives `line_has_match`).
        assert!(out.iter().any(|s| s.matched));
    }

    #[test]
    fn overlay_match_ranges_flags_only_matched_chunks() {
        // Two-byte span "ab" with only the second byte matched: the pre-chunk
        // stays unmatched, the overlaid chunk is flagged.
        let base = HighlightedSpan {
            text: "ab".to_string(),
            ..span(None)
        };
        let out = overlay_match_ranges(&[base], &[(1, 2)], Some(0x112233));
        let unmatched: Vec<&str> = out
            .iter()
            .filter(|s| !s.matched)
            .map(|s| s.text.as_str())
            .collect();
        let matched: Vec<&str> = out
            .iter()
            .filter(|s| s.matched)
            .map(|s| s.text.as_str())
            .collect();
        assert_eq!(unmatched, vec!["a"]);
        assert_eq!(matched, vec!["b"]);
    }

    // --- helpers for the end-to-end windowing and overlay tests ---

    // A temp file that removes itself on drop, so tests never leak files even
    // when an assertion panics mid-way.
    struct TempFile {
        path: std::path::PathBuf,
    }

    impl TempFile {
        fn new(name: &str, content: &str) -> Self {
            use std::sync::atomic::{AtomicU64, Ordering};
            static COUNTER: AtomicU64 = AtomicU64::new(0);
            let n = COUNTER.fetch_add(1, Ordering::Relaxed);
            let mut path = std::env::temp_dir();
            path.push(format!(
                "fff_preview_test_{}_{}_{}",
                std::process::id(),
                n,
                name
            ));
            std::fs::write(&path, content).expect("write temp file");
            TempFile { path }
        }

        fn path(&self) -> &Path {
            &self.path
        }
    }

    impl Drop for TempFile {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.path);
        }
    }

    // `n` lines, each encoding its own 1-based line number so a row in the
    // preview window can be mapped back to the absolute file line.
    fn numbered_lines(n: usize) -> String {
        (1..=n)
            .map(|i| format!("line {i:04}"))
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn line_text(line: &HighlightedLine) -> String {
        line.spans.iter().map(|s| s.text.as_str()).collect()
    }

    fn colored_span(text: &str, color: u32) -> HighlightedSpan {
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

    // A1. A file previewed with no match to center on starts at line 1.
    #[test]
    fn highlight_file_window_no_match_starts_at_line_one() {
        let file = TempFile::new("nomatch.txt", &numbered_lines(20));
        let (start, preview) = highlight_file_window(file.path(), None);
        assert_eq!(start, 1);
        assert_eq!(preview.len(), 20);
        assert_eq!(line_text(&preview[0]), "line 0001");
        assert_eq!(line_text(&preview[19]), "line 0020");
    }

    // A2. An early match in a large file clamps the window to the file start,
    // and preview rows still map to their absolute file lines.
    #[test]
    fn highlight_file_window_early_match_clamps_to_start() {
        let file = TempFile::new("early.txt", &numbered_lines(1200));
        let (start, preview) = highlight_file_window(file.path(), Some(5));
        assert_eq!(start, 1);
        assert_eq!(preview.len(), MAX_PREVIEW_LINES);
        // Row = absolute line - start.
        assert_eq!(line_text(&preview[5 - start]), "line 0005");
        assert_eq!(line_text(&preview[0]), "line 0001");
    }

    // A3. A middle match centers the window: start = match - 249.
    #[test]
    fn highlight_file_window_middle_match_centers() {
        let file = TempFile::new("middle.txt", &numbered_lines(1200));
        let (start, preview) = highlight_file_window(file.path(), Some(600));
        // Centered window: start matches the pure windowing math.
        assert_eq!(start, 600 - 249);
        assert_eq!(start, window_start_line(Some(600), 1200));
        assert_eq!(preview.len(), MAX_PREVIEW_LINES);
        // The match row maps back to the correct absolute line.
        assert_eq!(line_text(&preview[600 - start]), "line 0600");
    }

    // A4. A match near the end clamps so the window ends on the last line,
    // leaving the match in the window's back half.
    #[test]
    fn highlight_file_window_end_match_clamps_to_last_window() {
        let file = TempFile::new("end.txt", &numbered_lines(1200));
        let (start, preview) = highlight_file_window(file.path(), Some(1190));
        assert_eq!(start, 1200 - MAX_PREVIEW_LINES + 1); // 701
        assert_eq!(preview.len(), MAX_PREVIEW_LINES);
        // Window ends on the last file line.
        assert_eq!(line_text(&preview[MAX_PREVIEW_LINES - 1]), "line 1200");
        // Match sits in the back half of the window.
        let match_row = 1190 - start;
        assert_eq!(line_text(&preview[match_row]), "line 1190");
        assert!(match_row >= MAX_PREVIEW_LINES / 2);
    }

    // A5. A file longer than the cap yields exactly MAX_PREVIEW_LINES lines.
    #[test]
    fn highlight_file_window_caps_at_max_preview_lines() {
        let file = TempFile::new("cap.txt", &numbered_lines(812));
        let (start, preview) = highlight_file_window(file.path(), None);
        assert_eq!(start, 1);
        assert_eq!(preview.len(), MAX_PREVIEW_LINES);
    }

    // A6. A nonexistent / unreadable file returns the empty contract (1, []).
    #[test]
    fn highlight_file_window_missing_file_returns_empty() {
        let mut path = std::env::temp_dir();
        path.push("fff_preview_test_definitely_missing_zzz.txt");
        let _ = std::fs::remove_file(&path);
        let (start, preview) = highlight_file_window(&path, Some(10));
        assert_eq!(start, 1);
        assert!(preview.is_empty());
    }

    // B7. A match range spanning a syntax-span boundary splices cleanly: the
    // text reassembles exactly, only the matched chunks get the background,
    // and the surrounding chunks keep their original color and stay clean.
    #[test]
    fn overlay_match_ranges_splices_across_span_boundary() {
        let spans = vec![
            colored_span("foo", 0x111111),
            colored_span("bar", 0x222222),
            colored_span("baz", 0x333333),
        ];
        // Line bytes: foo=0..3, bar=3..6, baz=6..9. Range 2..5 = "oba",
        // straddling the foo/bar boundary.
        let out = overlay_match_ranges(&spans, &[(2, 5)], Some(0xAABBCC));

        // Text reassembles byte-for-byte.
        let reassembled: String = out.iter().map(|s| s.text.as_str()).collect();
        assert_eq!(reassembled, "foobarbaz");

        // Exact chunk breakdown: (text, color, bg, bold).
        let got: Vec<(String, u32, Option<u32>, bool)> = out
            .iter()
            .map(|s| (s.text.clone(), s.color, s.bg, s.bold))
            .collect();
        assert_eq!(
            got,
            vec![
                ("fo".to_string(), 0x111111, None, false),
                ("o".to_string(), 0x111111, Some(0xAABBCC), true),
                ("ba".to_string(), 0x222222, Some(0xAABBCC), true),
                ("r".to_string(), 0x222222, None, false),
                ("baz".to_string(), 0x333333, None, false),
            ]
        );

        // Only the matched bytes carry a background.
        let marked: String = out
            .iter()
            .filter(|s| s.bg.is_some())
            .map(|s| s.text.as_str())
            .collect();
        assert_eq!(marked, "oba");
    }

    // B8. Multiple ranges on one line each get a background, and the clean
    // chunk between them is left untouched.
    #[test]
    fn overlay_match_ranges_handles_multiple_ranges() {
        let spans = vec![colored_span("hello world", 0x111111)];
        // "hello world": ranges 0..2 = "he", 6..9 = "wor".
        let out = overlay_match_ranges(&spans, &[(0, 2), (6, 9)], Some(0xAABBCC));

        let got: Vec<(String, Option<u32>, bool)> =
            out.iter().map(|s| (s.text.clone(), s.bg, s.bold)).collect();
        assert_eq!(
            got,
            vec![
                ("he".to_string(), Some(0xAABBCC), true),
                ("llo ".to_string(), None, false),
                ("wor".to_string(), Some(0xAABBCC), true),
                ("ld".to_string(), None, false),
            ]
        );
    }

    // B9. An empty range list leaves every span unmarked and unchanged.
    #[test]
    fn overlay_match_ranges_empty_leaves_spans_unchanged() {
        let spans = vec![colored_span("abc", 0x111111), colored_span("def", 0x222222)];
        let out = overlay_match_ranges(&spans, &[], Some(0xAABBCC));

        assert_eq!(out.len(), spans.len());
        for (o, i) in out.iter().zip(spans.iter()) {
            assert_eq!(o.text, i.text);
            assert_eq!(o.color, i.color);
            assert_eq!(o.bg, None);
            assert!(!o.bold);
        }
    }

    // C10. The top-clamp transition: for a 1200-line file the window stops
    // clamping to start=1 and begins centering exactly between lines 250/251.
    #[test]
    fn window_start_line_top_clamp_boundary() {
        // Line 250 still centers no further left than the file start.
        assert_eq!(window_start_line(Some(250), 1200), 1);
        // Line 251 is the first that begins to scroll (start = 2).
        assert_eq!(window_start_line(Some(251), 1200), 2);
    }

    // --- highlight_single_line ---

    fn concat_spans(spans: &[HighlightedSpan]) -> String {
        spans.iter().map(|s| s.text.as_str()).collect()
    }

    // Deterministic per-capture color so distinct captures are observable.
    // The process-global test theme has an empty syntax table (every capture
    // resolves to one style, merging adjacent spans), so capture-boundary
    // tests inject this resolver into `highlighted_lines_with` instead.
    fn style_per_capture(capture: &str) -> theme::SyntaxRenderStyle {
        theme::SyntaxRenderStyle {
            color: capture
                .bytes()
                .fold(0u32, |acc, b| acc.wrapping_mul(31).wrapping_add(b as u32)),
            ..Default::default()
        }
    }

    // D11. A Rust line splits into multiple spans with distinct syntax colors
    // (keyword vs string, at minimum), and the text survives byte-for-byte.
    #[test]
    fn highlight_single_line_rust_yields_multiple_colored_spans() {
        let line = "let x = \"s\";";
        let spec = syntax_set_for_path(Path::new("main.rs")).expect("rust is supported");
        let lines =
            highlighted_lines_with(line, &spec, style_per_capture).expect("highlighting succeeds");

        assert_eq!(lines.len(), 1);
        let spans = &lines[0].spans;
        assert!(
            spans.len() > 1,
            "expected multiple spans, got {}",
            spans.len()
        );
        let distinct_colors: std::collections::HashSet<u32> =
            spans.iter().map(|s| s.color).collect();
        assert!(
            distinct_colors.len() > 1,
            "expected more than one syntax color"
        );
        assert_eq!(concat_spans(spans), line);

        // The public helper takes the same tree-sitter path and reassembles
        // the exact input (colors merge under the uniform test theme).
        assert_eq!(
            concat_spans(&highlight_single_line(Path::new("main.rs"), line)),
            line
        );
    }

    // D12. A YAML line highlights (non-empty spans) and reassembles exactly;
    // the injected resolver proves real yaml captures fire (key vs value).
    #[test]
    fn highlight_single_line_yaml_yields_spans() {
        let line = "key: value";
        let spans = highlight_single_line(Path::new("config.yml"), line);
        assert!(!spans.is_empty());
        assert_eq!(concat_spans(&spans), line);

        let spec = syntax_set_for_path(Path::new("config.yml")).expect("yaml is supported");
        let lines =
            highlighted_lines_with(line, &spec, style_per_capture).expect("highlighting succeeds");
        assert!(lines[0].spans.len() > 1, "expected yaml captures to fire");
        assert_eq!(concat_spans(&lines[0].spans), line);
    }

    // D13. An unknown extension falls back to a single plain (default-styled)
    // span covering the whole line.
    #[test]
    fn highlight_single_line_unknown_extension_yields_one_plain_span() {
        let line = "some unknown content";
        let spans = highlight_single_line(Path::new("file.xyz"), line);

        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0].text, line);
        let plain = theme::syntax_render_style("text");
        assert_eq!(spans[0].color, plain.color);
        assert_eq!(spans[0].bg, None);
        assert!(!spans[0].bold);
        assert!(!spans[0].italic);
    }

    // D14. Empty input yields no spans (which still concatenates to "").
    #[test]
    fn highlight_single_line_empty_input_yields_no_spans() {
        let spans = highlight_single_line(Path::new("main.rs"), "");
        assert!(spans.is_empty());
        assert_eq!(concat_spans(&spans), "");
    }

    // D15. An extension-less path takes the plain fallback without panicking.
    #[test]
    fn highlight_single_line_extensionless_path_falls_back_to_plain() {
        let line = "all: build test";
        let spans = highlight_single_line(Path::new("Makefile"), line);

        assert_eq!(spans.len(), 1);
        assert_eq!(concat_spans(&spans), line);
    }

    // D16. Binary-ish garbage (NULs, control bytes, odd Unicode) through a
    // real highlighter does not panic and still reassembles exactly.
    #[test]
    fn highlight_single_line_binary_garbage_does_not_panic() {
        let line = "\u{0}\u{1}\u{7f}\u{fffd}ÿ\u{0}garbage\u{2}";
        let spans = highlight_single_line(Path::new("data.rs"), line);
        assert_eq!(concat_spans(&spans), line);

        // Same garbage through the plain fallback path.
        let spans = highlight_single_line(Path::new("data.bin"), line);
        assert_eq!(concat_spans(&spans), line);
    }
}
