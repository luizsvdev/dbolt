use std::cmp::Ordering;
use std::rc::Rc;

use gpui_kit::base::input::{DiagnosticColors, DisplayPoint, InputEditorStyle, RopeExt};
use gpui_kit::component::button::{Button, ButtonRounded, ButtonVariants};
use gpui_kit::component::input::EditorState;
use gpui_kit::component::{ActiveTheme, IconName, Selectable, Sizable};
use gpui_kit::*;

use crate::theme::editor_background;

/// Share of the line height used to round selection corners.
const SELECTION_CORNER_RATIO: f32 = 0.15;

/// Draws a code editor with the theme's line number colour and rounded
/// selections, and without the active line highlight while text is selected.
///
/// gpui-kit's editor sets its whole style again on every render and paints
/// square selections, so the style is replaced after the editor lays out, its
/// own selection is made transparent, and the selection is painted here
/// underneath the text.
pub struct StyledEditor {
    editor: Entity<EditorState>,
    child: AnyElement,
}

impl StyledEditor {
    pub fn new(editor: &Entity<EditorState>, child: impl IntoElement) -> Self {
        Self {
            editor: editor.clone(),
            child: child.into_any_element(),
        }
    }
}

fn editor_style(has_selection: bool, cx: &App) -> InputEditorStyle {
    let theme = cx.theme();
    let syntax = &theme.highlight_theme.style;
    InputEditorStyle {
        foreground: theme.foreground,
        muted_foreground: syntax
            .editor_line_number
            .unwrap_or(theme.muted_foreground),
        background: editor_background(cx),
        border: theme.border,
        selection: theme.transparent,
        caret: theme.caret,
        diagnostics: DiagnosticColors {
            error: syntax.status.error(cx),
            warning: syntax.status.warning(cx),
            info: syntax.status.info(cx),
            hint: syntax.status.hint(cx),
        },
        highlight_styles: theme.highlight_theme.clone(),
        editor_invisible: syntax.editor_invisible,
        editor_active_line: if has_selection {
            None
        } else {
            syntax.editor_active_line
        },
        editor_gutter_background: syntax.editor_gutter_background,
        fold_icon_renderer: Some(Rc::new(|ix, is_folded| {
            Button::new(("fold-icon", ix))
                .ghost()
                .icon(if is_folded {
                    IconName::ChevronRight
                } else {
                    IconName::ChevronDown
                })
                .xsmall()
                .rounded(ButtonRounded::Small)
                .size(px(14.))
                .selected(is_folded)
                .into_any_element()
        })),
    }
}

/// The horizontal extent of the selection on one line.
struct SelectedLine {
    start_x: Pixels,
    end_x: Pixels,
}

struct SelectionShape {
    top: Pixels,
    line_height: Pixels,
    lines: Vec<SelectedLine>,
}

/// The left edge of the character at `offset`, on the row where the editor
/// places that offset (a wrap boundary belongs to the row below).
fn x_at(
    editor: &mut EditorState,
    offset: usize,
    text_bounds: Bounds<Pixels>,
    window: &mut Window,
    cx: &mut Context<EditorState>,
) -> Option<Point<Pixels>> {
    let offset = editor
        .text()
        .offset_to_offset_utf16(offset);
    editor
        .bounds_for_range(offset..offset, text_bounds, window, cx)
        .map(|bounds| bounds.origin)
}

fn previous_char_offset(editor: &EditorState, offset: usize) -> usize {
    let text = editor.text();
    text.char_index_to_offset(
        text.offset_to_char_index(offset)
            .saturating_sub(1),
    )
}

/// Where the selection sits on each visible screen row, measured from the
/// editor's last layout. A soft-wrapped line takes one row per wrap.
fn selection_shape(
    editor: &mut EditorState,
    window: &mut Window,
    cx: &mut Context<EditorState>,
) -> Option<SelectionShape> {
    let selection = editor.selected_range();
    if selection.is_empty()
        || !editor
            .focus_handle(cx)
            .is_focused(window)
    {
        return None;
    }
    let line_height = editor.line_height()?;
    let text_bounds = editor.text_bounds()?;
    let visible_lines = editor.visible_row_range()?;
    let overshoot = line_height * SELECTION_CORNER_RATIO * 2.;
    let first_line = editor
        .text()
        .offset_to_point(selection.start)
        .row
        .max(visible_lines.start);
    let last_line = editor
        .text()
        .offset_to_point(selection.end)
        .row
        .min(
            visible_lines
                .end
                .saturating_sub(1),
        );

    let mut top = None;
    let mut lines = Vec::new();
    for line in first_line..=last_line {
        let line_start = editor
            .text()
            .line_start_offset(line);
        let line_end = editor
            .text()
            .line_end_offset(line);
        let Some(display_rows) = editor
            .display_map()
            .buffer_line_to_display_row_range(line)
        else {
            continue;
        };
        let row_starts: Vec<usize> = display_rows
            .clone()
            .map(|row| {
                line_start
                    + editor
                        .display_map()
                        .display_pos_to_buffer_pos(DisplayPoint::new(row, 0))
                        .col
            })
            .collect();
        for (row_ix, row_start) in row_starts
            .iter()
            .copied()
            .enumerate()
        {
            let wraps = row_ix + 1 < row_starts.len();
            let row_end = row_starts
                .get(row_ix + 1)
                .copied()
                .unwrap_or(line_end);
            let segment_start = selection.start.max(row_start);
            let segment_end = selection.end.min(row_end);
            let selects_newline = !wraps && selection.end > line_end;
            if segment_start > segment_end || (segment_start == segment_end && !selects_newline) {
                continue;
            }
            let start = x_at(editor, segment_start, text_bounds, window, cx)?;
            let end_x = if wraps && segment_end == row_end {
                // The wrap boundary is placed on the next row, so the row's
                // end is its last character plus that character's width.
                let last_char = previous_char_offset(editor, row_end);
                let last_x = x_at(editor, last_char, text_bounds, window, cx)?.x;
                let width = if last_char > row_start {
                    let before = previous_char_offset(editor, last_char);
                    last_x - x_at(editor, before, text_bounds, window, cx)?.x
                } else {
                    line_height / 2.
                };
                last_x + width
            } else {
                x_at(editor, segment_end, text_bounds, window, cx)?.x
            };
            top.get_or_insert(start.y);
            lines.push(SelectedLine {
                start_x: start.x,
                end_x: if selects_newline {
                    end_x + overshoot
                } else {
                    end_x
                },
            });
        }
    }
    Some(SelectionShape {
        top: top?,
        line_height,
        lines,
    })
}

impl SelectionShape {
    fn paint(&self, color: Hsla, window: &mut Window) {
        let corner_radius = self.line_height * SELECTION_CORNER_RATIO;
        // A selection whose first line starts right of where the second one
        // ends is two separate shapes.
        match (self.lines.first(), self.lines.get(1)) {
            (Some(first), Some(second)) if first.start_x > second.end_x => {
                paint_lines(
                    self.top,
                    &self.lines[..1],
                    self.line_height,
                    corner_radius,
                    color,
                    window,
                );
                paint_lines(
                    self.top + self.line_height,
                    &self.lines[1..],
                    self.line_height,
                    corner_radius,
                    color,
                    window,
                );
            }
            _ => paint_lines(
                self.top,
                &self.lines,
                self.line_height,
                corner_radius,
                color,
                window,
            ),
        }
    }
}

/// Traces the outline of consecutive selected lines, rounding every corner.
fn paint_lines(
    start_y: Pixels,
    lines: &[SelectedLine],
    line_height: Pixels,
    corner_radius: Pixels,
    color: Hsla,
    window: &mut Window,
) {
    let (Some(first_line), Some(last_line)) = (lines.first(), lines.last()) else {
        return;
    };
    let first_top_left = point(first_line.start_x, start_y);
    let first_top_right = point(first_line.end_x, start_y);
    let curve_height = point(Pixels::ZERO, corner_radius);
    let curve_width = |start_x: Pixels, end_x: Pixels| {
        let half = (end_x - start_x) / 2.;
        point(half.min(corner_radius), Pixels::ZERO)
    };

    let top_curve_width = curve_width(first_line.start_x, first_line.end_x);
    let mut builder = PathBuilder::fill();
    builder.move_to(first_top_right - top_curve_width);
    builder.curve_to(first_top_right + curve_height, first_top_right);

    let mut iter = lines
        .iter()
        .enumerate()
        .peekable();
    while let Some((line_ix, line)) = iter.next() {
        let bottom_right = point(line.end_x, start_y + line_height * (line_ix + 1) as f32);
        match iter.peek() {
            Some((_, next_line)) => {
                let next_top_right = point(next_line.end_x, bottom_right.y);
                match next_top_right
                    .x
                    .partial_cmp(&bottom_right.x)
                {
                    Some(Ordering::Less) => {
                        let curve_width = curve_width(next_top_right.x, bottom_right.x);
                        builder.line_to(bottom_right - curve_height);
                        builder.curve_to(bottom_right - curve_width, bottom_right);
                        builder.line_to(next_top_right + curve_width);
                        builder.curve_to(next_top_right + curve_height, next_top_right);
                    }
                    Some(Ordering::Greater) => {
                        let curve_width = curve_width(bottom_right.x, next_top_right.x);
                        builder.line_to(bottom_right - curve_height);
                        builder.curve_to(bottom_right + curve_width, bottom_right);
                        builder.line_to(next_top_right - curve_width);
                        builder.curve_to(next_top_right + curve_height, next_top_right);
                    }
                    Some(Ordering::Equal) | None => builder.line_to(bottom_right),
                }
            }
            None => {
                let curve_width = curve_width(line.start_x, line.end_x);
                builder.line_to(bottom_right - curve_height);
                builder.curve_to(bottom_right - curve_width, bottom_right);
                let bottom_left = point(line.start_x, bottom_right.y);
                builder.line_to(bottom_left + curve_width);
                builder.curve_to(bottom_left - curve_height, bottom_left);
            }
        }
    }

    if first_line.start_x > last_line.start_x {
        let curve_width = curve_width(last_line.start_x, first_line.start_x);
        let second_top_left = point(last_line.start_x, start_y + line_height);
        builder.line_to(second_top_left + curve_height);
        builder.curve_to(second_top_left + curve_width, second_top_left);
        let first_bottom_left = point(first_line.start_x, second_top_left.y);
        builder.line_to(first_bottom_left - curve_width);
        builder.curve_to(first_bottom_left - curve_height, first_bottom_left);
    }

    builder.line_to(first_top_left + curve_height);
    builder.curve_to(first_top_left + top_curve_width, first_top_left);
    builder.line_to(first_top_right - top_curve_width);

    if let Ok(path) = builder.build() {
        window.paint_path(path, color);
    }
}

impl IntoElement for StyledEditor {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for StyledEditor {
    type RequestLayoutState = ();
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, ()) {
        let layout_id = self
            .child
            .request_layout(window, cx);
        let has_selection = !self
            .editor
            .read(cx)
            .selected_range()
            .is_empty();
        let style = editor_style(has_selection, cx);
        self.editor
            .update(cx, |editor, _| editor.set_editor_style(style));
        (layout_id, ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) {
        self.child
            .prepaint(window, cx);
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        _prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let shape = self
            .editor
            .update(cx, |editor, cx| selection_shape(editor, window, cx));
        if let Some(shape) = shape {
            let color = cx.theme().selection;
            window.with_content_mask(Some(ContentMask { bounds }), |window| {
                shape.paint(color, window)
            });
        }
        self.child.paint(window, cx);
    }
}
