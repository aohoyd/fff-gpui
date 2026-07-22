// Zed-style component builders (Zed's `crates/ui` at miniature scale). Each
// builder returns a styled `Stateful<Div>` with the id attached and nothing
// else — call sites in `picker.rs` chain their own `.on_click(...)`, so all
// orchestration/event wiring stays there. Render-only (untestable per project
// GPUI policy); `cargo build` warning-free is the gate.

use gpui::prelude::*;
use gpui::*;

use crate::theme::AppTheme;

// Fold toggle (Zed `ui::Disclosure`): 16×16 hoverable button holding a 14px
// chevron SVG — down when expanded, right when collapsed — tinted `icon_muted`.
pub fn disclosure(id: impl Into<ElementId>, expanded: bool, theme: &AppTheme) -> Stateful<Div> {
    let path = if expanded {
        "icons/chevron_down.svg"
    } else {
        "icons/chevron_right.svg"
    };
    div()
        .id(id)
        .w(px(16.0))
        .h(px(16.0))
        .flex_shrink_0()
        .rounded(px(2.0))
        .hover(|s| s.bg(rgba(theme.hover_row)))
        .flex()
        .items_center()
        .justify_center()
        .child(
            svg()
                .path(path)
                .size(px(14.0))
                .text_color(rgba(theme.icon_muted)),
        )
}

// Multiselect checkbox (Zed `ui::Checkbox`): 16×16 box in a 16px slot, 2px
// radius, 1px `border` outline. Unchecked stays transparent; checked fills
// with `hover_row` (deterministic stand-in for Zed's `element_background` —
// subtle, not accent) and draws a 14px check SVG tinted `text_accent`.
pub fn checkbox(id: impl Into<ElementId>, checked: bool, theme: &AppTheme) -> Stateful<Div> {
    div()
        .id(id)
        .w(px(16.0))
        .h(px(16.0))
        .flex_shrink_0()
        .rounded(px(2.0))
        .border_1()
        .border_color(rgba(theme.border))
        .flex()
        .items_center()
        .justify_center()
        .when(checked, |this| {
            this.bg(rgba(theme.hover_row)).child(
                svg()
                    .path("icons/check.svg")
                    .size(px(14.0))
                    .text_color(rgba(theme.text_accent)),
            )
        })
}

// Git-status edge bar: a full-row-height 3px strip flush against the row's
// left edge, tinted `color` when the row carries a git status and left
// transparent otherwise so text alignment stays consistent across rows with
// and without a status. Shared by every result-row renderer.
pub fn git_edge_bar(color: Option<u32>) -> Div {
    let mut bar = div().w(px(3.0)).h_full().flex_shrink_0();
    if let Some(color) = color {
        bar = bar.bg(rgba(color));
    }
    bar
}

// Icon button for the query-input row (Zed `ui::IconButton`,
// `ButtonSize::Default` = 22px square, fits the 36px input row): hoverable
// square holding a 14px SVG tinted `icon_muted` idle / `text_accent` when
// `active`.
pub fn icon_button(
    id: impl Into<ElementId>,
    icon_path: impl Into<SharedString>,
    active: bool,
    theme: &AppTheme,
) -> Stateful<Div> {
    let tint = if active {
        theme.text_accent
    } else {
        theme.icon_muted
    };
    div()
        .id(id)
        .w(px(22.0))
        .h(px(22.0))
        .flex_shrink_0()
        .rounded(px(2.0))
        .hover(|s| s.bg(rgba(theme.hover_row)))
        .flex()
        .items_center()
        .justify_center()
        .child(svg().path(icon_path).size(px(14.0)).text_color(rgba(tint)))
}
