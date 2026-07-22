use std::ops::Range;

use gpui::{
    App, Bounds, ClipboardItem, Context, CursorStyle, Element, ElementId, ElementInputHandler,
    Entity, EntityInputHandler, FocusHandle, Focusable, GlobalElementId, LayoutId, MouseButton,
    MouseDownEvent, MouseMoveEvent, MouseUpEvent, PaintQuad, Pixels, Point, ShapedLine,
    SharedString, Style, TextRun, UTF16Selection, UnderlineStyle, Window, actions, div, fill,
    point, prelude::*, px, relative, rgba, size,
};
use tracing::warn;
use unicode_segmentation::UnicodeSegmentation;

use crate::theme;
use crate::theme::AppTheme;

actions!(
    fff_text_field,
    [
        FieldBackspace,
        FieldDelete,
        FieldLeft,
        FieldRight,
        FieldSelectLeft,
        FieldSelectRight,
        FieldSelectAll,
        FieldHome,
        FieldEnd,
        FieldPaste,
        FieldCut,
        FieldCopy,
    ]
);

pub struct TextField {
    focus_handle: FocusHandle,
    placeholder: SharedString,
    content: SharedString,
    selected_range: Range<usize>,
    selection_reversed: bool,
    marked_range: Option<Range<usize>>,
    last_layout: Option<ShapedLine>,
    last_bounds: Option<Bounds<Pixels>>,
    is_selecting: bool,
}

impl TextField {
    // Create a new text field with the given placeholder text.
    pub fn new(placeholder: impl Into<SharedString>, cx: &mut Context<Self>) -> Self {
        Self {
            focus_handle: cx.focus_handle(),
            placeholder: placeholder.into(),
            content: SharedString::new(""),
            selected_range: 0..0,
            selection_reversed: false,
            marked_range: None,
            last_layout: None,
            last_bounds: None,
            is_selecting: false,
        }
    }

    // Return the current text content.
    pub fn text(&self) -> String {
        self.content.to_string()
    }

    // Replace the current text content and reset the selection.
    pub fn set_text(&mut self, text: impl Into<SharedString>, cx: &mut Context<Self>) {
        self.content = text.into();
        self.selected_range = self.content.len()..self.content.len();
        self.selection_reversed = false;
        self.marked_range = None;
        cx.notify();
    }

    // Move the cursor or selection to the left.
    fn left(&mut self, _: &FieldLeft, _: &mut Window, cx: &mut Context<Self>) {
        if self.selected_range.is_empty() {
            self.move_to(self.previous_boundary(self.cursor_offset()), cx);
        } else {
            self.move_to(self.selected_range.start, cx);
        }
    }

    // Move the cursor or selection to the right.
    fn right(&mut self, _: &FieldRight, _: &mut Window, cx: &mut Context<Self>) {
        if self.selected_range.is_empty() {
            self.move_to(self.next_boundary(self.selected_range.end), cx);
        } else {
            self.move_to(self.selected_range.end, cx);
        }
    }

    // Extend the selection one grapheme to the left.
    fn select_left(&mut self, _: &FieldSelectLeft, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(self.previous_boundary(self.cursor_offset()), cx);
    }

    // Extend the selection one grapheme to the right.
    fn select_right(&mut self, _: &FieldSelectRight, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(self.next_boundary(self.cursor_offset()), cx);
    }

    // Select the entire field contents.
    fn select_all(&mut self, _: &FieldSelectAll, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(0, cx);
        self.select_to(self.content.len(), cx);
    }

    // Move the cursor to the start of the field.
    fn home(&mut self, _: &FieldHome, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(0, cx);
    }

    // Move the cursor to the end of the field.
    fn end(&mut self, _: &FieldEnd, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(self.content.len(), cx);
    }

    // Delete the selected text or previous grapheme.
    fn backspace(&mut self, _: &FieldBackspace, window: &mut Window, cx: &mut Context<Self>) {
        if self.selected_range.is_empty() {
            self.select_to(self.previous_boundary(self.cursor_offset()), cx);
        }
        self.replace_text_in_range(None, "", window, cx);
    }

    // Delete the selected text or next grapheme.
    fn delete(&mut self, _: &FieldDelete, window: &mut Window, cx: &mut Context<Self>) {
        if self.selected_range.is_empty() {
            self.select_to(self.next_boundary(self.cursor_offset()), cx);
        }
        self.replace_text_in_range(None, "", window, cx);
    }

    // Start mouse selection at the clicked position.
    fn on_mouse_down(
        &mut self,
        event: &MouseDownEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.is_selecting = true;
        if event.modifiers.shift {
            self.select_to(self.index_for_mouse_position(event.position), cx);
        } else {
            self.move_to(self.index_for_mouse_position(event.position), cx);
        }
    }

    // End mouse selection.
    fn on_mouse_up(&mut self, _: &MouseUpEvent, _window: &mut Window, _: &mut Context<Self>) {
        self.is_selecting = false;
    }

    // Extend mouse selection while dragging.
    fn on_mouse_move(&mut self, event: &MouseMoveEvent, _: &mut Window, cx: &mut Context<Self>) {
        if self.is_selecting {
            self.select_to(self.index_for_mouse_position(event.position), cx);
        }
    }

    // Paste clipboard text into the field.
    fn paste(&mut self, _: &FieldPaste, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
            self.replace_text_in_range(None, &text.replace('\n', " "), window, cx);
        }
    }

    // Copy the selected text to the clipboard.
    fn copy(&mut self, _: &FieldCopy, _: &mut Window, cx: &mut Context<Self>) {
        if !self.selected_range.is_empty() {
            cx.write_to_clipboard(ClipboardItem::new_string(
                self.content[self.selected_range.clone()].to_string(),
            ));
        }
    }

    // Cut the selected text to the clipboard.
    fn cut(&mut self, _: &FieldCut, window: &mut Window, cx: &mut Context<Self>) {
        if !self.selected_range.is_empty() {
            cx.write_to_clipboard(ClipboardItem::new_string(
                self.content[self.selected_range.clone()].to_string(),
            ));
            self.replace_text_in_range(None, "", window, cx);
        }
    }

    // Collapse the selection at a byte offset.
    fn move_to(&mut self, offset: usize, cx: &mut Context<Self>) {
        self.selected_range = offset..offset;
        cx.notify();
    }

    // Return the cursor offset within the current selection.
    fn cursor_offset(&self) -> usize {
        if self.selection_reversed {
            self.selected_range.start
        } else {
            self.selected_range.end
        }
    }

    // Convert a mouse position to a content byte offset.
    fn index_for_mouse_position(&self, position: Point<Pixels>) -> usize {
        if self.content.is_empty() {
            return 0;
        }
        let (Some(bounds), Some(line)) = (self.last_bounds.as_ref(), self.last_layout.as_ref())
        else {
            return 0;
        };
        if position.y < bounds.top() {
            return 0;
        }
        if position.y > bounds.bottom() {
            return self.content.len();
        }
        line.closest_index_for_x(position.x - bounds.left())
    }

    // Extend the selection to a byte offset.
    fn select_to(&mut self, offset: usize, cx: &mut Context<Self>) {
        let (range, reversed) =
            selection_after_select_to(self.selected_range.clone(), self.selection_reversed, offset);
        self.selected_range = range;
        self.selection_reversed = reversed;
        cx.notify();
    }

    // Convert a UTF-16 offset to a UTF-8 byte offset.
    fn offset_from_utf16(&self, offset: usize) -> usize {
        offset_from_utf16(&self.content, offset)
    }

    // Convert a UTF-8 byte offset to a UTF-16 offset.
    fn offset_to_utf16(&self, offset: usize) -> usize {
        offset_to_utf16(&self.content, offset)
    }

    // Convert a UTF-8 byte range to a UTF-16 range.
    fn range_to_utf16(&self, range: &Range<usize>) -> Range<usize> {
        self.offset_to_utf16(range.start)..self.offset_to_utf16(range.end)
    }

    // Convert a UTF-16 range to a UTF-8 byte range.
    fn range_from_utf16(&self, range_utf16: &Range<usize>) -> Range<usize> {
        self.offset_from_utf16(range_utf16.start)..self.offset_from_utf16(range_utf16.end)
    }

    // Find the previous grapheme boundary before an offset.
    fn previous_boundary(&self, offset: usize) -> usize {
        previous_boundary(&self.content, offset)
    }

    // Find the next grapheme boundary after an offset.
    fn next_boundary(&self, offset: usize) -> usize {
        next_boundary(&self.content, offset)
    }
}

// Pure, gpui-free helpers backing the field's editing methods. Extracted so the
// grapheme/UTF-16/selection logic is unit-testable without a gpui `Context` or
// `FocusHandle` (see `#[cfg(test)]` below). Every returned byte offset lands on
// a UTF-8 char boundary, so `replace_text_in_range`'s slicing never panics on
// emoji / IME input.

// Convert a UTF-16 offset to a UTF-8 byte offset within `content`.
fn offset_from_utf16(content: &str, offset: usize) -> usize {
    let mut utf8_offset = 0;
    let mut utf16_count = 0;
    for ch in content.chars() {
        if utf16_count >= offset {
            break;
        }
        utf16_count += ch.len_utf16();
        utf8_offset += ch.len_utf8();
    }
    utf8_offset
}

// Convert a UTF-8 byte offset to a UTF-16 offset within `content`.
fn offset_to_utf16(content: &str, offset: usize) -> usize {
    let mut utf16_offset = 0;
    let mut utf8_count = 0;
    for ch in content.chars() {
        if utf8_count >= offset {
            break;
        }
        utf8_count += ch.len_utf8();
        utf16_offset += ch.len_utf16();
    }
    utf16_offset
}

// Find the previous grapheme boundary before a byte offset within `content`.
fn previous_boundary(content: &str, offset: usize) -> usize {
    content
        .grapheme_indices(true)
        .rev()
        .find_map(|(index, _)| (index < offset).then_some(index))
        .unwrap_or(0)
}

// Find the next grapheme boundary after a byte offset within `content`.
fn next_boundary(content: &str, offset: usize) -> usize {
    content
        .grapheme_indices(true)
        .find_map(|(index, _)| (index > offset).then_some(index))
        .unwrap_or(content.len())
}

// Compute the new `(selected_range, selection_reversed)` after extending a
// selection to `offset`. Extending past the anchor flips the reversal flag and
// re-normalizes the range so `start <= end`.
fn selection_after_select_to(
    selected_range: Range<usize>,
    selection_reversed: bool,
    offset: usize,
) -> (Range<usize>, bool) {
    let mut range = selected_range;
    let mut reversed = selection_reversed;
    if reversed {
        range.start = offset;
    } else {
        range.end = offset;
    }
    if range.end < range.start {
        reversed = !reversed;
        range = range.end..range.start;
    }
    (range, reversed)
}

impl EntityInputHandler for TextField {
    // Return text for the requested IME range.
    fn text_for_range(
        &mut self,
        range_utf16: Range<usize>,
        actual_range: &mut Option<Range<usize>>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<String> {
        let range = self.range_from_utf16(&range_utf16);
        actual_range.replace(self.range_to_utf16(&range));
        Some(self.content[range].to_string())
    }

    // Return the selected text range for IME integration.
    fn selected_text_range(
        &mut self,
        _ignore_disabled_input: bool,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        Some(UTF16Selection {
            range: self.range_to_utf16(&self.selected_range),
            reversed: self.selection_reversed,
        })
    }

    // Return the active marked text range for IME integration.
    fn marked_text_range(
        &self,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Range<usize>> {
        self.marked_range
            .as_ref()
            .map(|range| self.range_to_utf16(range))
    }

    // Clear the active marked text range.
    fn unmark_text(&mut self, _window: &mut Window, _cx: &mut Context<Self>) {
        self.marked_range = None;
    }

    // Replace text in the requested range.
    fn replace_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let range = range_utf16
            .as_ref()
            .map(|range_utf16| self.range_from_utf16(range_utf16))
            .or(self.marked_range.clone())
            .unwrap_or(self.selected_range.clone());

        self.content =
            (self.content[0..range.start].to_owned() + new_text + &self.content[range.end..])
                .into();
        self.selected_range = range.start + new_text.len()..range.start + new_text.len();
        self.marked_range.take();
        cx.notify();
    }

    // Replace text and mark an IME composition range.
    fn replace_and_mark_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        new_selected_range_utf16: Option<Range<usize>>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let range = range_utf16
            .as_ref()
            .map(|range_utf16| self.range_from_utf16(range_utf16))
            .or(self.marked_range.clone())
            .unwrap_or(self.selected_range.clone());

        self.content =
            (self.content[0..range.start].to_owned() + new_text + &self.content[range.end..])
                .into();
        self.marked_range =
            (!new_text.is_empty()).then_some(range.start..range.start + new_text.len());
        self.selected_range = new_selected_range_utf16
            .as_ref()
            .map(|range_utf16| self.range_from_utf16(range_utf16))
            .map(|new_range| new_range.start + range.start..new_range.end + range.end)
            .unwrap_or_else(|| range.start + new_text.len()..range.start + new_text.len());
        cx.notify();
    }

    // Return bounds for the requested text range.
    fn bounds_for_range(
        &mut self,
        range_utf16: Range<usize>,
        bounds: Bounds<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        let line = self.last_layout.as_ref()?;
        let range = self.range_from_utf16(&range_utf16);
        Some(Bounds::from_corners(
            point(bounds.left() + line.x_for_index(range.start), bounds.top()),
            point(bounds.left() + line.x_for_index(range.end), bounds.bottom()),
        ))
    }

    // Return the UTF-16 character index at a window point.
    fn character_index_for_point(
        &mut self,
        point: Point<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<usize> {
        let line_point = self.last_bounds?.localize(&point)?;
        let line = self.last_layout.as_ref()?;
        let utf8_index = line.index_for_x(point.x - line_point.x)?;
        Some(self.offset_to_utf16(utf8_index))
    }
}

struct TextFieldElement {
    input: Entity<TextField>,
}

struct PrepaintState {
    line: Option<ShapedLine>,
    cursor: Option<PaintQuad>,
    selection: Option<PaintQuad>,
}

impl IntoElement for TextFieldElement {
    type Element = Self;

    // Convert the wrapper into a GPUI element.
    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for TextFieldElement {
    type RequestLayoutState = ();
    type PrepaintState = PrepaintState;

    // Return a stable element id when one is needed.
    fn id(&self) -> Option<ElementId> {
        None
    }

    // Return source location metadata for diagnostics.
    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    // Request layout for the single-line text input.
    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&gpui::InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let mut style = Style::default();
        style.size.width = relative(1.).into();
        style.size.height = px(18.0).into();
        (window.request_layout(style, [], cx), ())
    }

    // Shape text and prepare cursor and selection quads.
    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&gpui::InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        let input = self.input.read(cx);
        let content = input.content.clone();
        let selected_range = input.selected_range.clone();
        let cursor = input.cursor_offset();

        let display_text = if content.is_empty() {
            input.placeholder.clone()
        } else {
            content
        };
        let palette = theme::palette();
        let text_color = if input.content.is_empty() {
            rgba(palette.text_dim)
        } else {
            rgba(palette.input_text)
        };

        let run = TextRun {
            len: display_text.len(),
            font: window.text_style().font(),
            color: text_color.into(),
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        let runs = if let Some(marked_range) = input.marked_range.as_ref() {
            vec![
                TextRun {
                    len: marked_range.start,
                    ..run.clone()
                },
                TextRun {
                    len: marked_range.end - marked_range.start,
                    underline: Some(UnderlineStyle {
                        color: Some(run.color),
                        thickness: px(1.0),
                        wavy: false,
                    }),
                    ..run.clone()
                },
                TextRun {
                    len: display_text.len() - marked_range.end,
                    ..run
                },
            ]
            .into_iter()
            .filter(|run| run.len > 0)
            .collect()
        } else {
            vec![run]
        };

        let font_size = window.text_style().font_size.to_pixels(window.rem_size());
        let line = window
            .text_system()
            .shape_line(display_text, font_size, &runs, None);
        let cursor_x = line.x_for_index(cursor);
        let (selection, cursor) = if selected_range.is_empty() {
            (
                None,
                Some(fill(
                    Bounds::new(
                        point(bounds.left() + cursor_x, bounds.top() + px(1.0)),
                        size(px(2.0), bounds.bottom() - bounds.top() - px(2.0)),
                    ),
                    rgba(palette.cursor),
                )),
            )
        } else {
            (
                Some(fill(
                    Bounds::from_corners(
                        point(
                            bounds.left() + line.x_for_index(selected_range.start),
                            bounds.top(),
                        ),
                        point(
                            bounds.left() + line.x_for_index(selected_range.end),
                            bounds.bottom(),
                        ),
                    ),
                    rgba(theme::with_alpha(palette.selected_row, 0x44)),
                )),
                None,
            )
        };

        PrepaintState {
            line: Some(line),
            cursor,
            selection,
        }
    }

    // Paint the shaped text and update input geometry.
    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&gpui::InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let focus_handle = self.input.read(cx).focus_handle.clone();
        window.handle_input(
            &focus_handle,
            ElementInputHandler::new(bounds, self.input.clone()),
            cx,
        );

        if let Some(selection) = prepaint.selection.take() {
            window.paint_quad(selection);
        }

        // gpui's element contract guarantees paint follows prepaint, so `line` is
        // always populated here.
        let line = prepaint.line.take().unwrap();
        // `ShapedLine::paint` is fallible (e.g. glyph rasterization / atlas errors on
        // unusual glyphs). The field repaints every frame while focused, so log and
        // continue instead of panicking the whole picker on a transient failure.
        if let Err(err) = line.paint(
            bounds.origin,
            window.line_height(),
            gpui::TextAlign::Left,
            None,
            window,
            cx,
        ) {
            warn!(error = %err, "failed to paint text field line");
        }

        if focus_handle.is_focused(window)
            && let Some(cursor) = prepaint.cursor.take()
        {
            window.paint_quad(cursor);
        }

        self.input.update(cx, |input, _cx| {
            input.last_layout = Some(line);
            input.last_bounds = Some(bounds);
        });
    }
}

impl Render for TextField {
    // Render the text field shell.
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.global::<AppTheme>().clone();
        div()
            .key_context("FffTextField")
            .track_focus(&self.focus_handle(cx))
            .cursor(CursorStyle::IBeam)
            .on_action(cx.listener(Self::backspace))
            .on_action(cx.listener(Self::delete))
            .on_action(cx.listener(Self::left))
            .on_action(cx.listener(Self::right))
            .on_action(cx.listener(Self::select_left))
            .on_action(cx.listener(Self::select_right))
            .on_action(cx.listener(Self::select_all))
            .on_action(cx.listener(Self::home))
            .on_action(cx.listener(Self::end))
            .on_action(cx.listener(Self::paste))
            .on_action(cx.listener(Self::cut))
            .on_action(cx.listener(Self::copy))
            .on_mouse_down(MouseButton::Left, cx.listener(Self::on_mouse_down))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::on_mouse_up))
            .on_mouse_up_out(MouseButton::Left, cx.listener(Self::on_mouse_up))
            .on_mouse_move(cx.listener(Self::on_mouse_move))
            // Plain Zed-style input: no box/border/rounding. The picker's 36px
            // search row already supplies height and horizontal padding. Fill
            // the row height (`h_full`) so the whole row is clickable, and
            // center the ~18px text element within it (`flex().items_center()`)
            // so the text still sits vertically centered exactly as before.
            .h_full()
            .flex()
            .items_center()
            .line_height(px(theme.buffer_font_size * 1.2))
            .text_size(px(theme.buffer_font_size))
            .child(TextFieldElement { input: cx.entity() })
    }
}

impl Focusable for TextField {
    // Return the focus handle for this text field.
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::{
        next_boundary, offset_from_utf16, offset_to_utf16, previous_boundary,
        selection_after_select_to,
    };

    // Grapheme stepping over ASCII: every step advances/retreats one byte.
    #[test]
    fn boundaries_step_ascii_one_byte_at_a_time() {
        let s = "abc";
        assert_eq!(next_boundary(s, 0), 1);
        assert_eq!(next_boundary(s, 1), 2);
        // Past the last boundary clamps to the content length.
        assert_eq!(next_boundary(s, 2), 3);
        assert_eq!(next_boundary(s, 3), 3);

        assert_eq!(previous_boundary(s, 3), 2);
        assert_eq!(previous_boundary(s, 1), 0);
        // Before the first boundary clamps to 0.
        assert_eq!(previous_boundary(s, 0), 0);
    }

    // Grapheme stepping over a multibyte char steps the whole char, landing on
    // char boundaries so downstream slicing can never panic.
    #[test]
    fn boundaries_step_over_multibyte_chars() {
        let s = "café"; // 'é' occupies bytes 3..5
        assert_eq!(next_boundary(s, 2), 3);
        assert_eq!(next_boundary(s, 3), 5);
        assert_eq!(previous_boundary(s, 5), 3);
        assert!(s.is_char_boundary(next_boundary(s, 3)));
        assert!(s.is_char_boundary(previous_boundary(s, 5)));
    }

    // A ZWJ emoji sequence is a single grapheme cluster: one step crosses the
    // whole cluster, never splitting it into an invalid byte offset.
    #[test]
    fn boundaries_treat_zwj_emoji_as_one_cluster() {
        // Man + ZWJ + Woman + ZWJ + Girl: 18 bytes, one grapheme cluster.
        let s = "👨‍👩‍👧";
        assert_eq!(s.len(), 18);
        assert_eq!(next_boundary(s, 0), 18);
        assert_eq!(previous_boundary(s, 18), 0);
        // A simple emoji then ASCII: stepping crosses the 4-byte emoji as a unit.
        let s = "👍a";
        assert_eq!(next_boundary(s, 0), 4);
        assert_eq!(next_boundary(s, 4), 5);
        assert_eq!(previous_boundary(s, 5), 4);
        assert_eq!(previous_boundary(s, 4), 0);
    }

    // UTF-16 <-> UTF-8 offsets round-trip on multibyte content (BMP + astral).
    #[test]
    fn utf16_utf8_offsets_round_trip() {
        // "a" (1 byte / 1 u16), "é" (2 bytes / 1 u16), "👍" (4 bytes / 2 u16).
        let s = "aé👍";
        // Byte offsets at each char boundary.
        for &byte_off in &[0usize, 1, 3, 7] {
            let utf16 = offset_to_utf16(s, byte_off);
            assert_eq!(
                offset_from_utf16(s, utf16),
                byte_off,
                "round trip failed at byte {byte_off}"
            );
        }
        // Known-value spot checks.
        assert_eq!(offset_to_utf16(s, 0), 0);
        assert_eq!(offset_to_utf16(s, 1), 1); // after 'a'
        assert_eq!(offset_to_utf16(s, 3), 2); // after 'é'
        assert_eq!(offset_to_utf16(s, 7), 4); // after '👍' (surrogate pair)
        assert_eq!(offset_from_utf16(s, 4), 7);
    }

    // select_to without crossing the anchor extends in place, no reversal.
    #[test]
    fn select_to_extends_forward_without_reversing() {
        let (range, reversed) = selection_after_select_to(2..2, false, 5);
        assert_eq!(range, 2..5);
        assert!(!reversed);
    }

    // Extending a forward selection back past its anchor flips it to reversed
    // and re-normalizes so start <= end.
    #[test]
    fn select_to_past_anchor_flips_to_reversed() {
        let (range, reversed) = selection_after_select_to(3..6, false, 1);
        assert_eq!(range, 1..3);
        assert!(reversed);
    }

    // A reversed selection moves its start; pulling it back past the anchor
    // flips it forward again.
    #[test]
    fn select_to_reversed_selection_moves_start_and_can_flip_back() {
        // Reversed 2..6 (anchor at end=6), extend start left to 1.
        let (range, reversed) = selection_after_select_to(2..6, true, 1);
        assert_eq!(range, 1..6);
        assert!(reversed);
        // Reversed 2..6, drag start right past the anchor (to 9) → flips forward.
        let (range, reversed) = selection_after_select_to(2..6, true, 9);
        assert_eq!(range, 6..9);
        assert!(!reversed);
    }
}
