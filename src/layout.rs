// Pure viewport sizing / split clamp / scroll math for the picker modal.
// Plain f32/usize types (no gpui) so everything here stays unit-testable.

/// Fraction of the display used for each modal dimension by default.
pub const DEFAULT_DISPLAY_FRACTION: f32 = 0.60;
/// Hard ceiling for each modal dimension relative to the display.
pub const MAX_DISPLAY_FRACTION: f32 = 0.95;
/// Minimum modal footprint (min results 280 + 1px divider + min preview 128
/// wide, 320 tall).
pub const MIN_MODAL_WIDTH: f32 = 409.0;
pub const MIN_MODAL_HEIGHT: f32 = 320.0;
/// Modal size used when the display size cannot be determined.
pub const FALLBACK_WIDTH: f32 = 960.0;
pub const FALLBACK_HEIGHT: f32 = 520.0;
/// Absolute ceiling for each modal dimension when the display size is unknown.
/// With no display to derive the 95% cap from, an oversized px override (e.g. a
/// `window_width = 96000.0` config typo) would otherwise be honored verbatim,
/// producing a non-resizable window far larger than any screen. Sized to a 4K
/// display (3840x2160) so real overrides pass through while pathological ones
/// are clamped to something that still fits on-screen.
pub const MAX_FALLBACK_WIDTH: f32 = 3840.0;
pub const MAX_FALLBACK_HEIGHT: f32 = 2160.0;
/// Window origin used when the display bounds cannot be determined.
pub const FALLBACK_ORIGIN_X: f32 = 400.0;
pub const FALLBACK_ORIGIN_Y: f32 = 200.0;
/// Default results-pane share of the modal width (preview gets the rest).
pub const DEFAULT_RESULTS_FRACTION: f32 = 0.50;
/// Minimum pane widths enforced by the divider clamps.
pub const MIN_RESULTS_WIDTH: f32 = 280.0;
pub const MIN_PREVIEW_WIDTH: f32 = 128.0;
/// Width of the 1px results/preview divider, reserved out of the modal width
/// before the panes are allocated so both pane minimums still hold with the
/// divider rendered between them.
pub const DIVIDER_WIDTH: f32 = 1.0;
/// Gap between the line-number gutter and the code text.
pub const GUTTER_GAP: f32 = 8.0;
/// Fixed non-directory chrome in a results row: edge bar + horizontal paddings
/// + file icon + the gaps around the filename text. Excludes the leading
/// chevron/checkbox slot, which the caller adds via `extra_chrome_px` since it
/// is present only on header rows and multiselect file rows.
pub const ROW_BASE_OVERHEAD: f32 = 101.0;
/// A leading chevron (grep header) or checkbox (multiselect file row) slot:
/// 16px glyph + 8px gap. Added to the base overhead when the slot renders.
pub const ROW_LEADING_SLOT: f32 = 24.0;

/// Results/preview widths for the modal body row.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Split {
    pub results_w: f32,
    pub preview_w: f32,
}

// Compute the modal size for a display, honoring px overrides from config.
// Defaults to 60%x60% of the display, clamped to <=95% of the display and
// floored at the minimum modal footprint; falls back to 960x520 when the
// display size is unavailable, clamped to the MAX_FALLBACK ceiling so a
// pathological override cannot exceed a plausible screen with no display info.
pub fn modal_size(
    display: Option<(f32, f32)>,
    cfg_w: Option<f32>,
    cfg_h: Option<f32>,
) -> (f32, f32) {
    match display {
        Some((display_w, display_h)) => (
            axis_size(display_w, cfg_w, MIN_MODAL_WIDTH),
            axis_size(display_h, cfg_h, MIN_MODAL_HEIGHT),
        ),
        None => (
            cfg_w
                .unwrap_or(FALLBACK_WIDTH)
                .clamp(MIN_MODAL_WIDTH, MAX_FALLBACK_WIDTH),
            cfg_h
                .unwrap_or(FALLBACK_HEIGHT)
                .clamp(MIN_MODAL_HEIGHT, MAX_FALLBACK_HEIGHT),
        ),
    }
}

// Compute the picker window's origin and size in pixels. `display` is the
// active display's (origin, size); the modal is centered horizontally and
// placed one-third from the top of the display. With no display info the modal
// size falls back (see `modal_size`) and the origin is a fixed offset. Returns
// plain floats ((origin_x, origin_y), (width, height)) so the caller converts
// to gpui `Bounds<Pixels>` and this module stays gpui-free and testable.
pub fn window_bounds(
    display: Option<((f32, f32), (f32, f32))>,
    cfg_w: Option<f32>,
    cfg_h: Option<f32>,
) -> ((f32, f32), (f32, f32)) {
    let display_size = display.map(|(_origin, size)| size);
    let (modal_w, modal_h) = modal_size(display_size, cfg_w, cfg_h);
    match display {
        Some(((origin_x, origin_y), (display_w, display_h))) => {
            let x = origin_x + (display_w - modal_w) / 2.0;
            let y = origin_y + (display_h - modal_h) / 3.0;
            ((x, y), (modal_w, modal_h))
        }
        None => ((FALLBACK_ORIGIN_X, FALLBACK_ORIGIN_Y), (modal_w, modal_h)),
    }
}

// Size one modal axis: px override wins over the percentage default, but both
// are clamped to <=95% of the display and floored at the minimum.
fn axis_size(display: f32, override_px: Option<f32>, min: f32) -> f32 {
    let desired = override_px.unwrap_or(display * DEFAULT_DISPLAY_FRACTION);
    desired.min(display * MAX_DISPLAY_FRACTION).max(min)
}

// Split the modal width into results/preview panes. The override (config or
// drag position) wins over the 50/50 default; results are kept >=280px and the
// preview >=128px, with the results minimum taking priority when the modal is
// too narrow for both. The 1px divider is reserved up front so that
// `results_w + DIVIDER_WIDTH + preview_w == modal_w` and both minimums hold at
// the clamp boundary.
pub fn split(modal_w: f32, override_results_w: Option<f32>) -> Split {
    let avail = (modal_w - DIVIDER_WIDTH).max(0.0);
    let desired = override_results_w.unwrap_or(avail * DEFAULT_RESULTS_FRACTION);
    let max_results = (avail - MIN_PREVIEW_WIDTH).max(MIN_RESULTS_WIDTH);
    let results_w = desired.clamp(MIN_RESULTS_WIDTH, max_results);
    Split {
        results_w,
        preview_w: (avail - results_w).max(0.0),
    }
}

// Clamp an in-progress divider drag (desired results width) to the pane
// minimums.
pub fn clamp_drag(results_w: f32, modal_w: f32) -> Split {
    split(modal_w, Some(results_w))
}

// Reset the divider to the 50/50 default split.
pub fn reset_split(modal_w: f32) -> Split {
    split(modal_w, None)
}

// First visible row that vertically centers `match_row`, clamped to the top:
// `match_row - (visible_rows - 1) / 2`, never below 0. Consumed by the
// preview highlight-window centering (`preview::window_start_line`); the
// in-pane scroll centering uses gpui's strict `ScrollStrategy::Center`
// instead, which needs no visible-row estimate.
pub fn scroll_center_row(match_row: usize, visible_rows: usize) -> usize {
    match_row.saturating_sub(visible_rows.saturating_sub(1) / 2)
}

// Width of the line-number gutter: digit count of the largest visible line
// number times the character width, plus the 8px gap before the code.
pub fn gutter_width(max_line_no: usize, char_w: f32) -> f32 {
    digit_count(max_line_no) as f32 * char_w + GUTTER_GAP
}

// Directory characters that fit in a results row after the filename and the
// fixed chrome, using the monospace 0.6-em advance-width heuristic (`char_px`).
// `extra_chrome_px` is the leading chevron/checkbox slot when it renders (0
// otherwise) — the directory text has no CSS ellipsis fallback, so the budget
// must account for the actual per-row chrome or an over-estimate overflows the
// row. Floors at 12 so the directory never fully vanishes.
pub fn dir_max_chars(
    results_pane_width: f32,
    char_px: f32,
    filename_chars: usize,
    extra_chrome_px: f32,
) -> usize {
    if char_px <= 0.0 {
        return 12;
    }
    let overhead = ROW_BASE_OVERHEAD + extra_chrome_px;
    let avail_px = (results_pane_width - filename_chars as f32 * char_px - overhead).max(0.0);
    ((avail_px / char_px) as usize).max(12)
}

// Number of decimal digits in `n` (0 counts as one digit).
fn digit_count(mut n: usize) -> usize {
    let mut digits = 1;
    while n >= 10 {
        n /= 10;
        digits += 1;
    }
    digits
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_close(actual: f32, expected: f32) {
        assert!(
            (actual - expected).abs() < 0.01,
            "expected {expected}, got {actual}"
        );
    }

    // modal_size

    #[test]
    fn modal_size_defaults_to_sixty_percent_of_display() {
        let (w, h) = modal_size(Some((2000.0, 1000.0)), None, None);
        assert_close(w, 1200.0);
        assert_close(h, 600.0);
    }

    #[test]
    fn modal_size_px_overrides_win_over_percentage() {
        let (w, h) = modal_size(Some((2000.0, 1000.0)), Some(500.0), Some(400.0));
        assert_close(w, 500.0);
        assert_close(h, 400.0);
    }

    #[test]
    fn modal_size_clamps_overrides_to_max_ninety_five_percent() {
        let (w, h) = modal_size(Some((1000.0, 800.0)), Some(2000.0), Some(1000.0));
        assert_close(w, 950.0);
        assert_close(h, 760.0);
    }

    #[test]
    fn modal_size_floors_on_display_smaller_than_minimum_footprint() {
        // 60% (180x120) and even the full 95% clamp (285x190) sit below the
        // minimum footprint, so the floor wins.
        let (w, h) = modal_size(Some((300.0, 200.0)), None, None);
        assert_close(w, MIN_MODAL_WIDTH);
        assert_close(h, MIN_MODAL_HEIGHT);
    }

    #[test]
    fn modal_size_floors_tiny_overrides() {
        let (w, h) = modal_size(Some((2000.0, 1000.0)), Some(100.0), Some(50.0));
        assert_close(w, MIN_MODAL_WIDTH);
        assert_close(h, MIN_MODAL_HEIGHT);
    }

    #[test]
    fn modal_size_overrides_each_axis_independently() {
        // Only width overridden: height falls back to the 60% default.
        let (w, h) = modal_size(Some((2000.0, 1000.0)), Some(500.0), None);
        assert_close(w, 500.0);
        assert_close(h, 600.0);
        // Only height overridden: width falls back to the 60% default.
        let (w, h) = modal_size(Some((2000.0, 1000.0)), None, Some(400.0));
        assert_close(w, 1200.0);
        assert_close(h, 400.0);
    }

    #[test]
    fn modal_size_falls_back_when_display_unavailable() {
        let (w, h) = modal_size(None, None, None);
        assert_close(w, FALLBACK_WIDTH);
        assert_close(h, FALLBACK_HEIGHT);
    }

    #[test]
    fn modal_size_fallback_still_honors_overrides() {
        let (w, h) = modal_size(None, Some(800.0), Some(450.0));
        assert_close(w, 800.0);
        assert_close(h, 450.0);
    }

    #[test]
    fn modal_size_fallback_floors_below_minimum_overrides() {
        // Display unavailable and both overrides below the floor: the minimum
        // footprint wins on each axis.
        let (w, h) = modal_size(None, Some(100.0), Some(50.0));
        assert_close(w, MIN_MODAL_WIDTH);
        assert_close(h, MIN_MODAL_HEIGHT);
    }

    #[test]
    fn modal_size_caps_pathological_override_when_display_present() {
        // A config typo (96000x50000) is clamped to 95% of the display.
        let (w, h) = modal_size(Some((1000.0, 800.0)), Some(96000.0), Some(50000.0));
        assert_close(w, 950.0);
        assert_close(h, 760.0);
    }

    #[test]
    fn modal_size_caps_pathological_override_when_display_absent() {
        // With no display info the 95% cap cannot apply, so the absolute
        // MAX_FALLBACK ceiling clamps the pathological override instead of
        // honoring it verbatim.
        let (w, h) = modal_size(None, Some(96000.0), Some(50000.0));
        assert_close(w, MAX_FALLBACK_WIDTH);
        assert_close(h, MAX_FALLBACK_HEIGHT);
    }

    // window_bounds

    #[test]
    fn window_bounds_centers_on_display_with_origin_offset() {
        // Display at origin (100, 50), size 2000x1000: default modal is 60%
        // (1200x600), centered horizontally and one-third from the top.
        let ((x, y), (w, h)) = window_bounds(Some(((100.0, 50.0), (2000.0, 1000.0))), None, None);
        assert_close(w, 1200.0);
        assert_close(h, 600.0);
        assert_close(x, 100.0 + (2000.0 - 1200.0) / 2.0); // 500
        assert_close(y, 50.0 + (1000.0 - 600.0) / 3.0); // ~183.33
    }

    #[test]
    fn window_bounds_uses_fallback_origin_when_display_absent() {
        let ((x, y), (w, h)) = window_bounds(None, None, None);
        assert_close(x, FALLBACK_ORIGIN_X);
        assert_close(y, FALLBACK_ORIGIN_Y);
        assert_close(w, FALLBACK_WIDTH);
        assert_close(h, FALLBACK_HEIGHT);
    }

    #[test]
    fn window_bounds_honors_overrides_and_recenters() {
        // Override the modal to 500x400; it should recenter within the display.
        let ((x, y), (w, h)) = window_bounds(
            Some(((0.0, 0.0), (2000.0, 1000.0))),
            Some(500.0),
            Some(400.0),
        );
        assert_close(w, 500.0);
        assert_close(h, 400.0);
        assert_close(x, (2000.0 - 500.0) / 2.0); // 750
        assert_close(y, (1000.0 - 400.0) / 3.0); // 200
    }

    // split / clamp_drag / reset_split

    #[test]
    fn split_defaults_to_fifty_fifty() {
        // The 1px divider is reserved first, so the 50/50 split is taken over
        // the remaining 999px.
        let s = split(1000.0, None);
        assert_close(s.results_w, 499.5);
        assert_close(s.preview_w, 499.5);
        assert_close(s.results_w + s.preview_w + DIVIDER_WIDTH, 1000.0);
    }

    #[test]
    fn split_honors_override_within_bounds() {
        let s = split(1000.0, Some(500.0));
        assert_close(s.results_w, 500.0);
        assert_close(s.preview_w, 499.0);
    }

    #[test]
    fn split_clamps_results_to_minimum() {
        let s = split(1000.0, Some(100.0));
        assert_close(s.results_w, MIN_RESULTS_WIDTH);
        assert_close(s.preview_w, 719.0);
    }

    #[test]
    fn split_clamps_preview_to_minimum() {
        let s = split(1000.0, Some(950.0));
        assert_close(s.results_w, 871.0);
        assert_close(s.preview_w, MIN_PREVIEW_WIDTH);
    }

    #[test]
    fn split_at_minimum_modal_width_fits_both_minimums() {
        // 409 == 280 results + 1px divider + 128 preview, so both minimums fit
        // exactly with the divider reserved.
        let s = split(MIN_MODAL_WIDTH, Some(0.0));
        assert_close(s.results_w, MIN_RESULTS_WIDTH);
        assert_close(s.preview_w, MIN_PREVIEW_WIDTH);
        assert_close(s.results_w + s.preview_w + DIVIDER_WIDTH, MIN_MODAL_WIDTH);
    }

    #[test]
    fn split_below_minimum_modal_width_preserves_results_minimum() {
        // Below MIN_MODAL_WIDTH there is no room for both minimums plus the
        // divider: the results minimum wins and the preview minimum silently
        // gives way.
        let s = split(300.0, None);
        assert_close(s.results_w, MIN_RESULTS_WIDTH);
        assert!(
            s.preview_w < MIN_PREVIEW_WIDTH,
            "preview minimum should give way below the floor"
        );
        assert_close(s.preview_w, 19.0); // 300 - 1px divider - 280
    }

    #[test]
    fn clamp_drag_clamps_both_sides() {
        let low = clamp_drag(0.0, 1000.0);
        assert_close(low.results_w, MIN_RESULTS_WIDTH);
        let high = clamp_drag(10_000.0, 1000.0);
        assert_close(high.preview_w, MIN_PREVIEW_WIDTH);
        let mid = clamp_drag(600.0, 1000.0);
        assert_close(mid.results_w, 600.0);
        assert_close(mid.preview_w, 399.0);
    }

    #[test]
    fn reset_split_restores_default() {
        assert_eq!(reset_split(1000.0), split(1000.0, None));
        let s = reset_split(1000.0);
        assert_close(s.results_w, 499.5);
        assert_close(s.preview_w, 499.5);
    }

    // scroll_center_row / gutter_width

    #[test]
    fn scroll_center_row_centers_match() {
        assert_eq!(scroll_center_row(50, 21), 40);
        assert_eq!(scroll_center_row(50, 20), 41);
        assert_eq!(scroll_center_row(50, 1), 50);
    }

    #[test]
    fn scroll_center_row_clamps_near_top() {
        assert_eq!(scroll_center_row(3, 21), 0);
        assert_eq!(scroll_center_row(0, 21), 0);
        assert_eq!(scroll_center_row(10, 21), 0);
        assert_eq!(scroll_center_row(11, 21), 1);
    }

    #[test]
    fn gutter_width_scales_with_digit_count() {
        assert_close(gutter_width(5, 8.0), 16.0); // 1 digit
        assert_close(gutter_width(42, 8.0), 24.0); // 2 digits
        assert_close(gutter_width(999, 8.0), 32.0); // 3 digits
        assert_close(gutter_width(1000, 8.0), 40.0); // 4 digits
        assert_close(gutter_width(99_999, 8.0), 48.0); // 5 digits
    }

    #[test]
    fn gutter_width_treats_zero_as_one_digit() {
        assert_close(gutter_width(0, 8.0), 16.0);
    }

    // dir_max_chars

    #[test]
    fn dir_max_chars_baseline_no_chrome() {
        // 600px pane, 10px chars, 8-char filename: budget is
        // 600 - 80 - 101 = 419px -> 41 chars.
        assert_eq!(dir_max_chars(600.0, 10.0, 8, 0.0), 41);
    }

    #[test]
    fn dir_max_chars_chevron_chrome_shrinks_budget() {
        // Same row plus a 24px chevron slot: 419 - 24 = 395px -> 39 chars.
        let base = dir_max_chars(600.0, 10.0, 8, 0.0);
        let with_chevron = dir_max_chars(600.0, 10.0, 8, ROW_LEADING_SLOT);
        assert_eq!(with_chevron, 39);
        assert!(with_chevron < base, "chrome must shrink the budget");
    }

    #[test]
    fn dir_max_chars_checkbox_chrome_shrinks_budget() {
        // A checkbox slot is the same 24px as the chevron, so it costs the same.
        assert_eq!(
            dir_max_chars(600.0, 10.0, 8, ROW_LEADING_SLOT),
            dir_max_chars(600.0, 10.0, 8, 24.0)
        );
        assert!(dir_max_chars(600.0, 10.0, 8, 24.0) < dir_max_chars(600.0, 10.0, 8, 0.0));
    }

    #[test]
    fn dir_max_chars_floors_at_twelve() {
        // Narrow pane and a long filename drive the budget negative; the floor
        // of 12 holds so the directory never fully disappears.
        assert_eq!(dir_max_chars(300.0, 10.0, 40, ROW_LEADING_SLOT), 12);
        // Degenerate char width also floors instead of dividing by zero.
        assert_eq!(dir_max_chars(600.0, 0.0, 8, 0.0), 12);
    }
}
