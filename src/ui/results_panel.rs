use std::collections::VecDeque;
use std::ops::Range;
use std::rc::Rc;
use std::time::Duration;

use dbolt_database::{ColumnKey, Value};
use gpui_kit::assets::IconName as AssetIconName;
use gpui_kit::base::dock::DockPlacement;
use gpui_kit::base::input::Rope;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::dock::{BasePanel, DockArea, Panel, PanelEvent};
use gpui_kit::component::highlighter::SyntaxHighlighter;
use gpui_kit::component::input::{
    Editor, EditorState, FoldRange, HighlightStyleResolver, InputEdit, InputHighlighter,
    InputHighlighterFactory,
};
use gpui_kit::component::menu::{ContextMenuExt, PopupMenu, PopupMenuItem};
use gpui_kit::component::tab::{Tab, TabBar};
use gpui_kit::component::table::{Column as TableColumn, DataTable, TableDelegate, TableState};
use gpui_kit::component::{ActiveTheme, Icon, IconName, Sizable, Size};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;

use crate::components::button::Button;
use crate::components::icon_button::IconButton;
use crate::components::menu::entity_menu_item;
use crate::query_execution::{ExecutionState, QueryExecution, QueryRun};
use crate::sql_statements::from_target;
use crate::theme::{
    EDITOR_FONT_FAMILY, EDITOR_FONT_SIZE, EDITOR_FONT_WEIGHT, EDITOR_LINE_HEIGHT, ICON_SIZE_SMALL,
    LABEL_SIZE_SMALL, PANEL_HEADER_HEIGHT, editor_background, editor_font_features,
};
use crate::ui::editor_panel::{ConsoleRun, TabId};
use crate::util::ResultExt;

/// Longest text drawn in a cell. Laying out a whole document for a narrow
/// cell would cost time on every frame for nothing the user can see.
const MAX_CELL_CHARS: usize = 256;
/// Rows sampled to size the columns when the first page arrives.
const COLUMN_WIDTH_SAMPLE_ROWS: usize = 100;
const CHAR_WIDTH: f32 = 7.;
const CELL_PADDING: f32 = 24.;
const MIN_COLUMN_WIDTH: f32 = 60.;
const MAX_COLUMN_WIDTH: f32 = 400.;

#[derive(Clone, Copy, PartialEq, Eq)]
enum ValueKind {
    Number,
    Boolean,
    Date,
    Json,
    Text,
}

impl ValueKind {
    fn of_type(type_name: &str) -> Self {
        let type_name = type_name.to_ascii_lowercase();
        match type_name.as_str() {
            "int2" | "int4" | "int8" | "smallint" | "integer" | "bigint" | "float4" | "float8"
            | "real" | "double precision" | "numeric" | "decimal" | "oid" | "money" => Self::Number,
            "bool" | "boolean" => Self::Boolean,
            "json" | "jsonb" => Self::Json,
            "date" | "time" | "timetz" | "timestamp" | "timestamptz" | "interval" => Self::Date,
            _ if type_name.starts_with("timestamp") || type_name.starts_with("time ") => Self::Date,
            _ => Self::Text,
        }
    }
}

fn value_text(value: &Value) -> SharedString {
    match value {
        Value::Null => "NULL".into(),
        Value::Bool(true) => "true".into(),
        Value::Bool(false) => "false".into(),
        Value::Int(number) => number.to_string().into(),
        Value::Float(number) => number.to_string().into(),
        Value::Text(text) => single_line(text, MAX_CELL_CHARS).into(),
        Value::Bytes(bytes) => {
            let mut text = String::from("\\x");
            for byte in bytes
                .iter()
                .take(MAX_CELL_CHARS / 2)
            {
                text.push_str(&format!("{byte:02x}"));
            }
            if bytes.len() > MAX_CELL_CHARS / 2 {
                text.push('…');
            }
            text.into()
        }
        Value::Unsupported(type_name) => format!("<{type_name}>").into(),
    }
}

/// Keeps a cell to one line, since table rows have a fixed height.
fn single_line(text: &str, max_chars: usize) -> String {
    let mut line: String = text
        .chars()
        .take(max_chars)
        .map(|character| {
            if character.is_control() {
                ' '
            } else {
                character
            }
        })
        .collect();
    if text
        .chars()
        .nth(max_chars)
        .is_some()
    {
        line.push('…');
    }
    line
}

pub(crate) fn format_duration(duration: Duration) -> SharedString {
    let milliseconds = duration.as_millis();
    if milliseconds < 1000 {
        format!("{milliseconds} ms").into()
    } else if milliseconds < 60_000 {
        format!("{:.2} s", duration.as_secs_f64()).into()
    } else {
        let seconds = duration.as_secs();
        format!("{} m {:02} s", seconds / 60, seconds % 60).into()
    }
}

pub(crate) fn plural(count: u64, singular: &str, plural: &str) -> String {
    if count == 1 {
        format!("{count} {singular}")
    } else {
        format!("{count} {plural}")
    }
}

const ROW_NUMBER_WIDTH: Pixels = px(48.);
const GRID_ROW_HEIGHT: Pixels = px(24.);
const GRID_HEADER_HEIGHT: Pixels = px(26.);
const CELL_TEXT_SIZE: Pixels = px(12.);
const HEADER_TEXT_SIZE: Pixels = px(11.);
const TYPE_LABEL_SIZE: Pixels = px(10.);
const KEY_ICON_SIZE: Pixels = px(11.);
/// Characters a key icon and the gaps around the type label take in a header.
const HEADER_EXTRA_CHARS: usize = 4;
const SELECTED_ROW_TINT: f32 = 0.08;
const SELECTED_CELL_TINT: f32 = 0.18;

/// A rectangle of cells in data coordinates (row, column), from where the
/// drag started to where it is now.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct CellRange {
    anchor: (usize, usize),
    head: (usize, usize),
}

impl CellRange {
    fn cell(row_ix: usize, col_ix: usize) -> Self {
        Self {
            anchor: (row_ix, col_ix),
            head: (row_ix, col_ix),
        }
    }

    fn rows(&self) -> Range<usize> {
        self.anchor.0.min(self.head.0)..self.anchor.0.max(self.head.0) + 1
    }

    fn cols(&self) -> Range<usize> {
        self.anchor.1.min(self.head.1)..self.anchor.1.max(self.head.1) + 1
    }

    fn contains(&self, row_ix: usize, col_ix: usize) -> bool {
        self.rows().contains(&row_ix) && self.cols().contains(&col_ix)
    }
}

/// Table column 0 is the row number; data column `n` is table column `n + 1`.
struct ResultTableDelegate {
    execution: Entity<QueryExecution>,
    columns: Vec<TableColumn>,
    kinds: Vec<ValueKind>,
    selection: Option<CellRange>,
    dragging: bool,
}

impl ResultTableDelegate {
    fn new(execution: Entity<QueryExecution>) -> Self {
        Self {
            execution,
            columns: Vec::new(),
            kinds: Vec::new(),
            selection: None,
            dragging: false,
        }
    }

    fn sync_columns(&mut self, cx: &App) {
        let execution = self.execution.read(cx);
        self.kinds = execution
            .columns()
            .iter()
            .map(|column| ValueKind::of_type(&column.type_name))
            .collect();
        let row_number = TableColumn::new("row-number", "")
            .width(ROW_NUMBER_WIDTH)
            .fixed_left()
            .resizable(false)
            .movable(false)
            .selectable(false)
            .p_0();
        let data_columns = execution
            .columns()
            .iter()
            .enumerate()
            .map(|(column_ix, column)| {
                let widest_value = execution
                    .rows()
                    .iter()
                    .take(COLUMN_WIDTH_SAMPLE_ROWS)
                    .filter_map(|row| row.get(column_ix))
                    .map(|value| {
                        value_text(value)
                            .chars()
                            .count()
                    })
                    .max()
                    .unwrap_or(0);
                let header = column.name.chars().count()
                    + column
                        .type_name
                        .chars()
                        .count()
                    + HEADER_EXTRA_CHARS;
                let widest = widest_value.max(header);
                let width = (widest as f32 * CHAR_WIDTH + CELL_PADDING)
                    .clamp(MIN_COLUMN_WIDTH, MAX_COLUMN_WIDTH);
                TableColumn::new(
                    SharedString::from(column_ix.to_string()),
                    column.name.clone(),
                )
                .width(px(width))
                .movable(false)
                .p_0()
            });
        self.columns = std::iter::once(row_number)
            .chain(data_columns)
            .collect();
    }

    fn data_column_count(&self) -> usize {
        self.columns
            .len()
            .saturating_sub(1)
    }

    fn value(&self, row_ix: usize, col_ix: usize, cx: &App) -> Option<Value> {
        self.execution
            .read(cx)
            .rows()
            .get(row_ix)
            .and_then(|row| row.get(col_ix))
            .cloned()
    }

    fn copy_text(&self, row_ix: usize, col_ix: usize, cx: &App) -> String {
        match self.value(row_ix, col_ix, cx) {
            Some(Value::Text(text)) => text,
            Some(value) => value_text(&value).to_string(),
            None => String::new(),
        }
    }

    /// The selected cells as CSV, with the selected columns' names first.
    fn selection_csv(&self, cx: &App) -> Option<String> {
        let selection = self.selection?;
        let execution = self.execution.read(cx);
        let columns = execution.columns();
        let rows = execution.rows();
        let col_range = selection.cols();
        let row_range = selection.rows().start
            ..selection
                .rows()
                .end
                .min(rows.len());
        let mut csv = columns
            .get(col_range.clone())?
            .iter()
            .map(|column| csv_field(&column.name))
            .collect::<Vec<_>>()
            .join(",");
        for row in rows
            .get(row_range)
            .unwrap_or_default()
        {
            csv.push('\n');
            let line = row
                .get(col_range.clone())
                .unwrap_or_default()
                .iter()
                .map(|value| match value {
                    Value::Null => String::new(),
                    Value::Text(text) => csv_field(text.as_str()),
                    value => csv_field(&value_text(value)),
                })
                .collect::<Vec<_>>()
                .join(",");
            csv.push_str(&line);
        }
        Some(csv)
    }

    /// Starts a selection at `cell`, or stretches the current one to it with
    /// Shift held.
    fn press(&mut self, range: CellRange, extend: bool) {
        self.selection = match (self.selection, extend) {
            (Some(selection), true) => Some(CellRange {
                anchor: selection.anchor,
                head: range.head,
            }),
            _ => Some(range),
        };
        self.dragging = true;
    }

    fn drag_to(&mut self, head: (usize, usize), button_held: bool) -> bool {
        if !button_held {
            self.dragging = false;
            return false;
        }
        match &mut self.selection {
            Some(selection) if self.dragging && selection.head != head => {
                selection.head = head;
                true
            }
            _ => false,
        }
    }
}

fn csv_field(text: &str) -> String {
    let needs_quotes =
        text.contains([',', '"', '\n', '\r']) || text.starts_with(' ') || text.ends_with(' ');
    if needs_quotes {
        format!("\"{}\"", text.replace('"', "\"\""))
    } else {
        text.to_string()
    }
}

fn focus_table(table: &TableState<ResultTableDelegate>, window: &mut Window, cx: &mut App) {
    let focus_handle = table.focus_handle(cx);
    window.focus(&focus_handle, cx);
}

impl TableDelegate for ResultTableDelegate {
    fn columns_count(&self, _: &App) -> usize {
        self.columns.len()
    }

    fn rows_count(&self, cx: &App) -> usize {
        self.execution
            .read(cx)
            .rows()
            .len()
    }

    fn column(&self, col_ix: usize, _: &App) -> TableColumn {
        self.columns
            .get(col_ix)
            .cloned()
            .unwrap_or_default()
    }

    fn render_th(
        &mut self,
        col_ix: usize,
        _window: &mut Window,
        cx: &mut Context<TableState<Self>>,
    ) -> impl IntoElement {
        let border_color = cx.theme().table_row_border;
        let Some(data_col) = col_ix.checked_sub(1) else {
            // The table keeps room for a sort icon on the right of each header
            // cell, so filling the cell would stop short of the column edge.
            return cell_frame()
                .flex_none()
                .w(ROW_NUMBER_WIDTH)
                .border_r_1()
                .border_color(border_color)
                .into_any_element();
        };
        let theme = cx.theme();
        let name = self
            .columns
            .get(col_ix)
            .map(|column| column.name.clone())
            .unwrap_or_default();
        let source = self
            .execution
            .read(cx)
            .columns()
            .get(data_col);
        let key_icon = source
            .and_then(|column| column.key)
            .map(|key| match key {
                ColumnKey::Primary => (AssetIconName::KeyRound, theme.yellow),
                ColumnKey::Foreign => (AssetIconName::Link2, theme.blue),
            });
        let type_name: SharedString = source
            .map(|column| column.type_name.clone())
            .unwrap_or_default()
            .into();
        let name_color = theme.foreground;
        let type_color = theme.muted_foreground;
        let mono_font = theme.mono_font_family.clone();
        let last_row = self
            .rows_count(cx)
            .saturating_sub(1);
        // The resize handle draws this column's right edge in the header.
        cell_frame()
            .id(("result-header", col_ix))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |table, event: &MouseDownEvent, window, cx| {
                    table.delegate_mut().press(
                        CellRange {
                            anchor: (0, data_col),
                            head: (last_row, data_col),
                        },
                        event.modifiers.shift,
                    );
                    focus_table(table, window, cx);
                    cx.notify();
                }),
            )
            .gap(px(5.))
            .text_size(HEADER_TEXT_SIZE)
            .font_weight(FontWeight::MEDIUM)
            .text_color(name_color)
            .when_some(key_icon, |this, (icon, color)| {
                this.child(
                    Icon::new(icon)
                        .size(KEY_ICON_SIZE)
                        .text_color(color)
                        .flex_none(),
                )
            })
            .child(div().flex_none().child(name))
            .child(
                div()
                    .min_w_0()
                    .truncate()
                    .font_family(mono_font)
                    .font_weight(FontWeight::NORMAL)
                    .text_size(TYPE_LABEL_SIZE)
                    .text_color(type_color)
                    .child(type_name),
            )
            .into_any_element()
    }

    fn render_td(
        &mut self,
        row_ix: usize,
        col_ix: usize,
        _window: &mut Window,
        cx: &mut Context<TableState<Self>>,
    ) -> impl IntoElement {
        let theme = cx.theme();
        let border_color = theme.table_row_border;
        let row_selected = self
            .selection
            .is_some_and(|selection| {
                selection
                    .rows()
                    .contains(&row_ix)
            });
        let Some(data_col) = col_ix.checked_sub(1) else {
            let last_col = self
                .data_column_count()
                .saturating_sub(1);
            return cell_frame()
                .id(("row-number", row_ix))
                .justify_end()
                .bg(if row_selected {
                    theme.border
                } else {
                    theme.tokens.table_head.into()
                })
                .border_r_1()
                .border_color(border_color)
                .font_family(theme.mono_font_family.clone())
                .text_size(HEADER_TEXT_SIZE)
                .text_color(if row_selected {
                    theme.foreground
                } else {
                    theme.muted_foreground
                })
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |table, event: &MouseDownEvent, window, cx| {
                        table.delegate_mut().press(
                            CellRange {
                                anchor: (row_ix, 0),
                                head: (row_ix, last_col),
                            },
                            event.modifiers.shift,
                        );
                        focus_table(table, window, cx);
                        cx.notify();
                    }),
                )
                .child((row_ix + 1).to_string())
                .into_any_element();
        };
        let selected = self
            .selection
            .is_some_and(|selection| selection.contains(row_ix, data_col));
        let is_head = self
            .selection
            .is_some_and(|selection| selection.head == (row_ix, data_col));
        let tint = if selected {
            Some(
                theme
                    .ring
                    .opacity(SELECTED_CELL_TINT),
            )
        } else if row_selected {
            Some(
                theme
                    .ring
                    .opacity(SELECTED_ROW_TINT),
            )
        } else {
            None
        };
        let focus_color = theme.ring;
        let mono_font = theme.mono_font_family.clone();
        let table = cx.weak_entity();
        let execution = self.execution.read(cx);
        let Some(value) = execution
            .rows()
            .get(row_ix)
            .and_then(|row| row.get(data_col))
        else {
            return cell_frame()
                .border_r_1()
                .border_color(border_color)
                .into_any_element();
        };
        let kind = self
            .kinds
            .get(data_col)
            .copied()
            .unwrap_or(ValueKind::Text);
        let number_color = theme
            .highlight_theme
            .style("number")
            .and_then(|style| style.color)
            .unwrap_or(theme.yellow);
        let color = match (value, kind) {
            (Value::Null | Value::Unsupported(_), _) => theme.muted_foreground,
            (Value::Int(_) | Value::Float(_), _) | (_, ValueKind::Number) => number_color,
            (Value::Bool(_), _) | (_, ValueKind::Boolean) => theme.magenta,
            (_, ValueKind::Date) => theme.cyan,
            (_, ValueKind::Json) => theme.green,
            _ => theme.foreground,
        };
        let is_number = kind == ValueKind::Number;
        let text = value_text(value);
        let is_placeholder = matches!(value, Value::Null | Value::Unsupported(_));
        let cell = (row_ix, data_col);
        cell_frame()
            .id((
                "result-cell",
                row_ix
                    * self
                        .data_column_count()
                        .max(1)
                    + data_col,
            ))
            .relative()
            .border_r_1()
            .border_color(border_color)
            .font_family(mono_font)
            .text_size(CELL_TEXT_SIZE)
            .when_some(tint, |this, tint| this.bg(tint))
            .when(is_number, |this| this.justify_end())
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |table, event: &MouseDownEvent, window, cx| {
                    table
                        .delegate_mut()
                        .press(CellRange::cell(cell.0, cell.1), event.modifiers.shift);
                    focus_table(table, window, cx);
                    cx.notify();
                }),
            )
            .on_mouse_move(cx.listener(move |table, event: &MouseMoveEvent, _, cx| {
                let button_held = event.pressed_button == Some(MouseButton::Left);
                if table
                    .delegate_mut()
                    .drag_to(cell, button_held)
                {
                    cx.notify();
                }
            }))
            .child(
                div()
                    .truncate()
                    .text_color(color)
                    .when(is_placeholder, |this| this.italic())
                    .child(text),
            )
            .when(is_head, |this| {
                this.child(
                    div()
                        .absolute()
                        .inset_0()
                        .border_1()
                        .border_color(focus_color),
                )
            })
            .context_menu(move |menu, _window, cx| cell_context_menu(&table, cell, menu, cx))
            .into_any_element()
    }

    fn cell_text(&self, row_ix: usize, col_ix: usize, cx: &App) -> String {
        match col_ix.checked_sub(1) {
            Some(data_col) => self.copy_text(row_ix, data_col, cx),
            None => (row_ix + 1).to_string(),
        }
    }

    fn has_more(&self, cx: &App) -> bool {
        self.execution
            .read(cx)
            .has_more()
    }

    fn load_more(&mut self, _window: &mut Window, cx: &mut Context<TableState<Self>>) {
        self.execution
            .update(cx, |execution, cx| execution.load_more(cx));
    }
}

/// Oldest entries are dropped past this, so a long session cannot grow the
/// output without bound.
const MAX_OUTPUT_ENTRIES: usize = 2000;
const CONSOLE_LIST_WIDTH: Pixels = px(240.);
const CONSOLE_ROW_HEIGHT: Pixels = px(24.);
const CONSOLE_INDENT: Pixels = px(16.);

const OUTPUT_LANGUAGE: &str = "dbolt-output";
const ERROR_PREFIX: &str = "ERROR: ";

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum OutputSegment {
    Time,
    Prompt,
    Sql,
    Plain,
    Error,
}

/// Splits the output into styled segments covering all of it. A line starts
/// with `[time]`; a statement line follows it with a `prompt>` token, and the
/// statement's other lines are indented under it.
fn output_segments(text: &str) -> Vec<(Range<usize>, OutputSegment)> {
    let mut segments = Vec::new();
    let mut in_statement = false;
    let mut line_start = 0;
    for line in text.split_inclusive('\n') {
        let line_end = line_start + line.len();
        let content = line.trim_end_matches(['\n', '\r']);
        let time_end = content
            .starts_with('[')
            .then(|| content.find("] "))
            .flatten();
        match time_end {
            Some(time_end) => {
                let time_end = line_start + time_end + 1;
                segments.push((line_start..time_end, OutputSegment::Time));
                let rest = &text[time_end..line_end];
                let body_start = time_end + rest.len() - rest.trim_start().len();
                let prompt_len = rest
                    .split_whitespace()
                    .next()
                    .filter(|token| token.ends_with('>'))
                    .map(str::len);
                segments.push((time_end..body_start, OutputSegment::Plain));
                in_statement = prompt_len.is_some();
                match prompt_len {
                    Some(prompt_len) => {
                        let prompt_end = body_start + prompt_len;
                        segments.push((body_start..prompt_end, OutputSegment::Prompt));
                        segments.push((prompt_end..line_end, OutputSegment::Sql));
                    }
                    None if rest
                        .trim_start()
                        .starts_with(ERROR_PREFIX) =>
                    {
                        segments.push((body_start..line_end, OutputSegment::Error));
                    }
                    None => segments.push((body_start..line_end, OutputSegment::Plain)),
                }
            }
            None if in_statement => segments.push((line_start..line_end, OutputSegment::Sql)),
            None => segments.push((line_start..line_end, OutputSegment::Plain)),
        }
        line_start = line_end;
    }
    segments.retain(|(range, _)| !range.is_empty());
    segments
}

/// Highlights the output log: statements as SQL, times and prompts with the
/// syntax theme's comment and number colours, and errors in red.
struct OutputHighlighter {
    sql: SyntaxHighlighter,
    segments: Vec<(Range<usize>, OutputSegment)>,
    error_color: Hsla,
}

impl OutputHighlighter {
    fn segment_style(
        &self,
        segment: OutputSegment,
        resolver: &dyn HighlightStyleResolver,
    ) -> HighlightStyle {
        let color = match segment {
            OutputSegment::Time => resolver
                .style("comment")
                .and_then(|style| style.color),
            OutputSegment::Prompt => resolver
                .style("number")
                .and_then(|style| style.color),
            OutputSegment::Error => Some(self.error_color),
            OutputSegment::Sql | OutputSegment::Plain => None,
        };
        HighlightStyle {
            color,
            ..HighlightStyle::default()
        }
    }
}

impl InputHighlighter for OutputHighlighter {
    fn language(&self) -> SharedString {
        OUTPUT_LANGUAGE.into()
    }

    fn update(
        &mut self,
        _edit: Option<InputEdit>,
        text: &Rope,
        _folding: bool,
        _window: &mut Window,
        _cx: &mut Context<EditorState>,
    ) {
        let text = text.to_string();
        self.segments = output_segments(&text);
        // The parser sees only the statements, at their own offsets: other
        // text becomes spaces, and each log line starts with `;` so every
        // statement parses on its own.
        let mut sql_only = text.into_bytes();
        for (range, segment) in &self.segments {
            if *segment == OutputSegment::Sql {
                continue;
            }
            for (offset, byte) in sql_only
                .get_mut(range.clone())
                .into_iter()
                .flatten()
                .enumerate()
            {
                if *byte != b'\n' {
                    *byte = if offset == 0 && *segment == OutputSegment::Time {
                        b';'
                    } else {
                        b' '
                    };
                }
            }
        }
        // Every replaced byte was a whole segment, so only whole characters
        // were overwritten and the bytes stay valid UTF-8.
        let sql_only = String::from_utf8(sql_only).unwrap_or_default();
        self.sql
            .update(None, &Rope::from_str(&sql_only), None);
    }

    fn styles(
        &self,
        range: &Range<usize>,
        resolver: &dyn HighlightStyleResolver,
    ) -> Vec<(Range<usize>, HighlightStyle)> {
        let mut styles = Vec::new();
        let mut covered = range.start;
        for (segment_range, segment) in &self.segments {
            let start = segment_range
                .start
                .max(range.start);
            let end = segment_range
                .end
                .min(range.end);
            if start >= end {
                continue;
            }
            if *segment == OutputSegment::Sql {
                for (sql_range, style) in self
                    .sql
                    .styles(&(start..end), resolver)
                {
                    let sql_start = sql_range.start.max(covered);
                    if sql_start < sql_range.end {
                        if covered < sql_start {
                            styles.push((covered..sql_start, HighlightStyle::default()));
                        }
                        styles.push((sql_start..sql_range.end, style));
                        covered = sql_range.end;
                    }
                }
                if covered < end {
                    styles.push((covered..end, HighlightStyle::default()));
                }
            } else {
                if covered < start {
                    styles.push((covered..start, HighlightStyle::default()));
                }
                styles.push((start..end, self.segment_style(*segment, resolver)));
            }
            covered = end;
        }
        if covered < range.end {
            styles.push((covered..range.end, HighlightStyle::default()));
        }
        styles
    }

    fn fold_ranges(&self, _text: &Rope) -> Vec<FoldRange> {
        Vec::new()
    }
}

fn output_highlighter_factory(cx: &App) -> InputHighlighterFactory {
    let error_color = cx.theme().danger;
    Rc::new(move |language| {
        (language == OUTPUT_LANGUAGE).then(|| {
            Box::new(OutputHighlighter {
                sql: SyntaxHighlighter::new("sql"),
                segments: Vec::new(),
                error_color,
            }) as Box<dyn InputHighlighter>
        })
    })
}

fn timestamp() -> String {
    chrono::Local::now()
        .format("[%Y-%m-%d %H:%M:%S:%3f]")
        .to_string()
}

/// `[time] prompt> first line`, with the statement's other lines indented
/// under its first one.
fn statement_entry(prompt: &str, statement: &str) -> String {
    let prefix = format!("{} {prompt}> ", timestamp());
    let indent = " ".repeat(prefix.chars().count());
    let mut entry = prefix;
    for (line_ix, line) in statement.lines().enumerate() {
        if line_ix > 0 {
            entry.push('\n');
            entry.push_str(&indent);
        }
        entry.push_str(line);
    }
    entry
}

fn outcome_entry(execution: &QueryExecution) -> String {
    let duration = execution
        .duration()
        .map(format_duration)
        .unwrap_or_default();
    let outcome = match execution.state() {
        ExecutionState::Failed(error) => format!("{ERROR_PREFIX}{error}"),
        ExecutionState::Cancelled => "Cancelled".to_string(),
        ExecutionState::Running => "Running…".to_string(),
        ExecutionState::Ready if execution.columns().is_empty() => format!(
            "{} in {duration}",
            plural(
                execution
                    .rows_affected()
                    .unwrap_or(0),
                "row affected",
                "rows affected",
            )
        ),
        ExecutionState::Ready => {
            let rows = plural(execution.rows().len() as u64, "row", "rows");
            let more = if execution.has_more() {
                " (more available)"
            } else {
                ""
            };
            format!("{rows} retrieved in {duration}{more}")
        }
    };
    format!("{} {outcome}", timestamp())
}

actions!(
    results_panel,
    [
        /// Copies the selected cells as CSV, with the column names first.
        CopySelection,
    ]
);

const GRID_KEY_CONTEXT: &str = "ResultGrid";

#[cfg(target_os = "macos")]
const COPY_KEY: &str = "cmd-c";
#[cfg(not(target_os = "macos"))]
const COPY_KEY: &str = "ctrl-c";

pub fn init(cx: &mut App) {
    cx.bind_keys([KeyBinding::new(
        COPY_KEY,
        CopySelection,
        Some(GRID_KEY_CONTEXT),
    )]);
}

fn copy_selection(table: &Entity<TableState<ResultTableDelegate>>, cx: &mut App) {
    if let Some(csv) = table
        .read(cx)
        .delegate()
        .selection_csv(cx)
    {
        cx.write_to_clipboard(ClipboardItem::new_string(csv));
    }
}

/// Cells fill their column with no padding from the table, so their right
/// borders meet the row borders and join into column lines.
fn cell_frame() -> Div {
    div()
        .size_full()
        .flex()
        .items_center()
        .px_2()
        .overflow_hidden()
}

fn cell_context_menu(
    table: &WeakEntity<TableState<ResultTableDelegate>>,
    cell: (usize, usize),
    menu: PopupMenu,
    cx: &mut Context<PopupMenu>,
) -> PopupMenu {
    let Some(table) = table.upgrade() else {
        return menu;
    };
    table.update(cx, |table, cx| {
        let delegate = table.delegate_mut();
        delegate.dragging = false;
        if !delegate
            .selection
            .is_some_and(|selection| selection.contains(cell.0, cell.1))
        {
            delegate.selection = Some(CellRange::cell(cell.0, cell.1));
            cx.notify();
        }
    });
    let delegate = table.read(cx).delegate();
    let value = delegate.copy_text(cell.0, cell.1, cx);
    let csv = delegate.selection_csv(cx);
    menu.item(
        PopupMenuItem::new("Copy Value")
            .icon(AssetIconName::Copy)
            .on_click(move |_, _, cx| {
                cx.write_to_clipboard(ClipboardItem::new_string(value.clone()))
            }),
    )
    .when_some(csv, |menu, csv| {
        menu.item(PopupMenuItem::new("Copy as CSV").on_click(move |_, _, cx| {
            cx.write_to_clipboard(ClipboardItem::new_string(csv.clone()))
        }))
    })
}

struct ResultTab {
    execution: Entity<QueryExecution>,
    title: SharedString,
    table: Entity<TableState<ResultTableDelegate>>,
    _subscriptions: [Subscription; 2],
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ActiveTab {
    Output,
    Result(usize),
}

/// One editor tab's runs: their output log and the results of the latest run.
struct Console {
    id: TabId,
    name: SharedString,
    connection_name: SharedString,
    database: Option<SharedString>,
    run: Entity<QueryRun>,
    /// How much of `run` is already in the output: statements started and
    /// statements ended.
    logged_started: usize,
    logged_ended: usize,
    output: VecDeque<String>,
    output_editor: Entity<EditorState>,
    results: Vec<ResultTab>,
    active_tab: ActiveTab,
    _run_subscription: Subscription,
}

impl Console {
    fn result_title(&self, statement: &str) -> SharedString {
        match (from_target(statement), &self.database) {
            (Some(target), Some(database)) => format!("{database}.{target}").into(),
            (Some(target), None) => target.to_string().into(),
            (None, _) => format!("Result {}", self.results.len() + 1).into(),
        }
    }

    fn prompt(&self) -> &SharedString {
        self.database
            .as_ref()
            .unwrap_or(&self.connection_name)
    }

    /// Logs what the run did since the last sync, in order: each statement,
    /// then its outcome. Returns whether anything was added.
    fn sync(&mut self, window: &mut Window, cx: &mut Context<ResultsPanel>) -> bool {
        let run = self.run.read(cx);
        let executions = run.executions().to_vec();
        let ended_count = run.ended_count();
        let mut changed = false;
        loop {
            if self.logged_started == self.logged_ended
                && let Some(execution) = executions.get(self.logged_started)
            {
                let entry = statement_entry(self.prompt(), execution.read(cx).statement());
                self.push_output(entry);
                self.logged_started += 1;
            } else if self.logged_ended < ended_count
                && let Some(execution) = executions.get(self.logged_ended)
            {
                let ended = execution.read(cx);
                let entry = outcome_entry(ended);
                let returns_rows =
                    *ended.state() == ExecutionState::Ready && !ended.columns().is_empty();
                let title = returns_rows.then(|| self.result_title(ended.statement()));
                self.push_output(entry);
                self.logged_ended += 1;
                if let Some(title) = title {
                    let result = new_result_tab(execution.clone(), title, window, cx);
                    self.results.push(result);
                    self.active_tab = ActiveTab::Result(self.results.len() - 1);
                }
            } else {
                break;
            }
            changed = true;
        }
        if changed {
            let text = self
                .output
                .iter()
                .map(String::as_str)
                .collect::<Vec<_>>()
                .join("\n");
            self.output_editor
                .update(cx, |editor, cx| {
                    editor.set_value(text, window, cx);
                    // Clamped to the end of the text on the next layout.
                    editor.set_scroll_offset(point(px(0.), px(-1_000_000.)), cx);
                });
        }
        changed
    }

    fn push_output(&mut self, entry: String) {
        if self.output.len() == MAX_OUTPUT_ENTRIES {
            self.output.pop_front();
        }
        self.output.push_back(entry);
    }
}

pub struct ResultsPanel {
    focus_handle: FocusHandle,
    dock_area: WeakEntity<DockArea>,
    consoles: Vec<Console>,
    selected: Option<TabId>,
}

impl ResultsPanel {
    pub fn new(dock_area: WeakEntity<DockArea>, cx: &mut Context<Self>) -> Self {
        Self {
            focus_handle: cx.focus_handle(),
            dock_area,
            consoles: Vec::new(),
            selected: None,
        }
    }

    pub fn show_run(
        &mut self,
        console_run: &ConsoleRun,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let console_id = console_run.console;
        let run_subscription =
            cx.observe_in(&console_run.run, window, move |this, _, window, cx| {
                this.sync_console(console_id, window, cx)
            });
        match self.console_mut(console_id) {
            Some(console) => {
                console.name = console_run
                    .console_name
                    .clone();
                console.connection_name = console_run
                    .connection_name
                    .clone();
                console.database = console_run.database.clone();
                console.run = console_run.run.clone();
                console.logged_started = 0;
                console.logged_ended = 0;
                console.results.clear();
                console.active_tab = ActiveTab::Output;
                console._run_subscription = run_subscription;
            }
            None => {
                let highlighter_factory = output_highlighter_factory(cx);
                let output_editor = cx.new(|cx| {
                    let mut editor = EditorState::new(window, cx)
                        .language(OUTPUT_LANGUAGE)
                        .line_number(false)
                        .folding(false)
                        .indent_guides(false)
                        .scroll_beyond_last_line(Some(0));
                    editor.set_highlighter_factory(highlighter_factory, cx);
                    editor
                });
                self.consoles.push(Console {
                    id: console_id,
                    name: console_run
                        .console_name
                        .clone(),
                    connection_name: console_run
                        .connection_name
                        .clone(),
                    database: console_run.database.clone(),
                    run: console_run.run.clone(),
                    logged_started: 0,
                    logged_ended: 0,
                    output: VecDeque::new(),
                    output_editor,
                    results: Vec::new(),
                    active_tab: ActiveTab::Output,
                    _run_subscription: run_subscription,
                });
            }
        }
        self.selected = Some(console_id);
        self.sync_console(console_id, window, cx);
        cx.notify();
    }

    fn sync_console(&mut self, console_id: TabId, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(console) = self.console_mut(console_id) {
            console.sync(window, cx);
        }
        cx.notify();
    }

    fn close_console(&mut self, console_id: TabId, cx: &mut Context<Self>) {
        let Some(index) = self
            .consoles
            .iter()
            .position(|console| console.id == console_id)
        else {
            return;
        };
        self.consoles.remove(index);
        if self.selected == Some(console_id) {
            self.selected = self
                .consoles
                .get(index)
                .or_else(|| self.consoles.last())
                .map(|console| console.id);
        }
        cx.notify();
    }

    fn console(&self, id: TabId) -> Option<&Console> {
        self.consoles
            .iter()
            .find(|console| console.id == id)
    }

    fn console_mut(&mut self, id: TabId) -> Option<&mut Console> {
        self.consoles
            .iter_mut()
            .find(|console| console.id == id)
    }

    fn select_tab(&mut self, tab: ActiveTab, cx: &mut Context<Self>) {
        if let Some(console) = self
            .selected
            .and_then(|id| self.console_mut(id))
        {
            console.active_tab = tab;
            cx.notify();
        }
    }

    fn close_result(&mut self, result_ix: usize, cx: &mut Context<Self>) {
        let Some(console) = self
            .selected
            .and_then(|id| self.console_mut(id))
        else {
            return;
        };
        if result_ix >= console.results.len() {
            return;
        }
        console
            .results
            .remove(result_ix);
        console.active_tab = match console.active_tab {
            ActiveTab::Result(active) if active == result_ix => match result_ix.checked_sub(1) {
                Some(previous) => ActiveTab::Result(previous),
                None if console.results.is_empty() => ActiveTab::Output,
                None => ActiveTab::Result(0),
            },
            ActiveTab::Result(active) if active > result_ix => ActiveTab::Result(active - 1),
            active => active,
        };
        cx.notify();
    }

    fn close_panel(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.dock_area
            .update(cx, |area, cx| {
                area.toggle_dock(DockPlacement::Bottom, window, cx)
            })
            .log_err();
    }

    fn render_close_button(&self, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .flex_none()
            .px_2()
            .child(
                IconButton::new("close-results-panel", IconName::Close)
                    .small()
                    .tooltip("Close Panel")
                    .on_click(cx.listener(|this, _, window, cx| this.close_panel(window, cx))),
            )
    }

    fn render_console_list(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let panel = cx.weak_entity();
        let theme = cx.theme();
        let mut connection_names: Vec<&SharedString> = Vec::new();
        for console in &self.consoles {
            if !connection_names.contains(&&console.connection_name) {
                connection_names.push(&console.connection_name);
            }
        }
        let mut rows: Vec<AnyElement> = Vec::new();
        for (connection_ix, connection_name) in connection_names
            .into_iter()
            .enumerate()
        {
            rows.push(
                h_flex()
                    .id(("console-connection", connection_ix))
                    .h(CONSOLE_ROW_HEIGHT)
                    .flex_none()
                    .px_2()
                    .gap_1p5()
                    .child(
                        Icon::new(AssetIconName::Database)
                            .size(ICON_SIZE_SMALL)
                            .text_color(theme.muted_foreground),
                    )
                    .child(
                        div()
                            .truncate()
                            .child(connection_name.clone()),
                    )
                    .into_any_element(),
            );
            for console in self
                .consoles
                .iter()
                .filter(|console| &console.connection_name == connection_name)
            {
                let id = console.id;
                let selected = self.selected == Some(id);
                let run = console.run.read(cx);
                let status: SharedString = match run.duration() {
                    Some(duration) => format_duration(duration),
                    None => "running…".into(),
                };
                rows.push(
                    h_flex()
                        .id(("console", id.index()))
                        .h(CONSOLE_ROW_HEIGHT)
                        .flex_none()
                        .pl(CONSOLE_INDENT + px(8.))
                        .pr_2()
                        .gap_1p5()
                        .cursor_pointer()
                        .when(selected, |this| this.bg(theme.list_active))
                        .when(!selected, |this| {
                            this.hover(|style| style.bg(theme.list_hover))
                        })
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.selected = Some(id);
                            cx.notify();
                        }))
                        .child(
                            Icon::new(AssetIconName::FileCode)
                                .size(ICON_SIZE_SMALL)
                                .text_color(theme.muted_foreground),
                        )
                        .child(
                            div()
                                .min_w_0()
                                .truncate()
                                .child(console.name.clone()),
                        )
                        .child(
                            div()
                                .flex_none()
                                .text_color(theme.muted_foreground)
                                .child(status),
                        )
                        .context_menu({
                            let panel = panel.clone();
                            move |menu, _, _| {
                                menu.item(entity_menu_item(
                                    &panel,
                                    "Close",
                                    Some(AssetIconName::Close),
                                    move |this, _, cx| this.close_console(id, cx),
                                ))
                            }
                        })
                        .into_any_element(),
                );
            }
        }

        v_flex()
            .id("console-list")
            .flex_none()
            .w(CONSOLE_LIST_WIDTH)
            .h_full()
            .py_1()
            .overflow_y_scroll()
            .border_r_1()
            .border_color(theme.border)
            .text_size(LABEL_SIZE_SMALL)
            .children(rows)
    }

    fn render_tab_bar(&self, console: &Console, cx: &mut Context<Self>) -> impl IntoElement {
        let selected_index = match console.active_tab {
            ActiveTab::Output => 0,
            ActiveTab::Result(result_ix) => result_ix + 1,
        };
        let output_tab = Tab::new()
            .prefix(
                div()
                    .pl_2()
                    .child(Icon::new(IconName::SquareTerminal).size(ICON_SIZE_SMALL)),
            )
            .child("Output");
        let result_tabs = console
            .results
            .iter()
            .enumerate()
            .map(|(result_ix, result)| {
                Tab::new()
                    .prefix(
                        div()
                            .pl_2()
                            .child(Icon::new(AssetIconName::Table).size(ICON_SIZE_SMALL)),
                    )
                    .child(result.title.clone())
                    .suffix(
                        div().mr_1().child(
                            IconButton::new(("close-result", result_ix), IconName::Close)
                                .small()
                                .tooltip("Close Result")
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    cx.stop_propagation();
                                    this.close_result(result_ix, cx);
                                })),
                        ),
                    )
            });
        TabBar::new("result-tabs")
            .selected_index(selected_index)
            .on_click(cx.listener(|this, tab_ix: &usize, _, cx| {
                let tab = match tab_ix.checked_sub(1) {
                    Some(result_ix) => ActiveTab::Result(result_ix),
                    None => ActiveTab::Output,
                };
                this.select_tab(tab, cx);
            }))
            .child(output_tab)
            .children(result_tabs)
            .suffix(
                h_flex()
                    .h_full()
                    .items_center()
                    .border_b_1()
                    .border_color(cx.theme().border)
                    .child(self.render_close_button(cx)),
            )
    }

    fn render_output(&self, console: &Console, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .bg(editor_background(cx))
            .child(
                Editor::new(&console.output_editor)
                    .readonly(true)
                    .appearance(false)
                    .bordered(false)
                    .font_family(EDITOR_FONT_FAMILY)
                    .text_size(EDITOR_FONT_SIZE)
                    .font_weight(EDITOR_FONT_WEIGHT)
                    .line_height(relative(EDITOR_LINE_HEIGHT))
                    .font_features(editor_font_features())
                    .size_full(),
            )
    }

    fn render_header(&self, tab: &ResultTab, cx: &mut Context<Self>) -> impl IntoElement {
        let execution = tab.execution.read(cx);
        let theme = cx.theme();
        let duration = execution
            .duration()
            .map(format_duration);
        let rows = plural(execution.rows().len() as u64, "row", "rows");
        let summary = if execution.is_loading_more() {
            format!("{rows}, loading more…")
        } else if execution.has_more() {
            format!("{rows}, more available")
        } else {
            rows
        };
        let loading = execution.is_loading_more();

        h_flex()
            .flex_none()
            .h(PANEL_HEADER_HEIGHT)
            .px_2()
            .gap_2()
            .bg(editor_background(cx))
            .border_b_1()
            .border_color(theme.border)
            .text_size(LABEL_SIZE_SMALL)
            .text_color(theme.muted_foreground)
            .child(summary)
            .when_some(duration, |this, duration| this.child(duration))
            .child(div().flex_1())
            .when(loading, |this| {
                let execution = tab.execution.clone();
                this.child(
                    Button::new("cancel-execution")
                        .label("Cancel")
                        .label_size(LABEL_SIZE_SMALL)
                        .tooltip("Stop loading more rows")
                        .on_click(move |_, _, cx| {
                            execution.update(cx, |execution, cx| execution.cancel(cx));
                        }),
                )
            })
    }

    fn render_result(&self, tab: &ResultTab, cx: &mut Context<Self>) -> impl IntoElement {
        let error = match tab.execution.read(cx).state() {
            ExecutionState::Failed(error) => Some(error.clone()),
            _ => None,
        };
        let border_color = cx.theme().border;
        let danger_color = cx.theme().danger;
        v_flex()
            .size_full()
            .child(self.render_header(tab, cx))
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .key_context(GRID_KEY_CONTEXT)
                    .on_action({
                        let table = tab.table.clone();
                        move |_: &CopySelection, _, cx| copy_selection(&table, cx)
                    })
                    .child(
                        DataTable::new(&tab.table)
                            .bordered(false)
                            .fixed_cols_border(false)
                            .with_size(Size::Size(GRID_ROW_HEIGHT))
                            .header_row_height(GRID_HEADER_HEIGHT),
                    ),
            )
            .when_some(error, |this, error| {
                this.child(
                    div()
                        .flex_none()
                        .px_2()
                        .py_1()
                        .border_t_1()
                        .border_color(border_color)
                        .text_size(LABEL_SIZE_SMALL)
                        .text_color(danger_color)
                        .child(error),
                )
            })
    }
}

fn new_result_tab(
    execution: Entity<QueryExecution>,
    title: SharedString,
    window: &mut Window,
    cx: &mut Context<ResultsPanel>,
) -> ResultTab {
    let table = cx.new(|cx| {
        let mut delegate = ResultTableDelegate::new(execution.clone());
        delegate.sync_columns(cx);
        TableState::new(delegate, window, cx)
            .sortable(false)
            .col_movable(false)
            .row_selectable(false)
            .col_selectable(false)
    });
    let events = cx.subscribe(&execution, {
        let table = table.clone();
        move |_, _, _, cx| table.update(cx, |_, cx| cx.notify())
    });
    let changes = cx.observe(&execution, |_, _, cx| cx.notify());
    ResultTab {
        execution,
        title,
        table,
        _subscriptions: [events, changes],
    }
}

impl EventEmitter<PanelEvent> for ResultsPanel {}

impl Focusable for ResultsPanel {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl BasePanel for ResultsPanel {
    fn panel_name(&self) -> &'static str {
        "ResultsPanel"
    }
}

impl Panel for ResultsPanel {
    fn title(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        "Results"
    }

    fn title_bar(&self, _: &App) -> bool {
        false
    }
}

impl Render for ResultsPanel {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(console) = self
            .selected
            .and_then(|id| self.console(id))
        else {
            return v_flex()
                .size_full()
                .track_focus(&self.focus_handle)
                .child(
                    h_flex()
                        .flex_none()
                        .h(PANEL_HEADER_HEIGHT)
                        .justify_end()
                        .child(self.render_close_button(cx)),
                )
                .into_any_element();
        };
        let body = match console.active_tab {
            ActiveTab::Result(result_ix) => match console.results.get(result_ix) {
                Some(result) => self
                    .render_result(result, cx)
                    .into_any_element(),
                None => self
                    .render_output(console, cx)
                    .into_any_element(),
            },
            ActiveTab::Output => self
                .render_output(console, cx)
                .into_any_element(),
        };

        h_flex()
            .size_full()
            .track_focus(&self.focus_handle)
            .child(self.render_console_list(cx))
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .child(self.render_tab_bar(console, cx))
                    .child(
                        div()
                            .flex_1()
                            .min_h_0()
                            .child(body),
                    ),
            )
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use dbolt_database::Value;

    use super::{
        CellRange, OutputSegment, ValueKind, csv_field, format_duration, output_segments,
        single_line, statement_entry, value_text,
    };

    #[test]
    fn value_kinds_follow_the_column_type() {
        assert!(ValueKind::of_type("int8") == ValueKind::Number);
        assert!(ValueKind::of_type("NUMERIC") == ValueKind::Number);
        assert!(ValueKind::of_type("bool") == ValueKind::Boolean);
        assert!(ValueKind::of_type("timestamptz") == ValueKind::Date);
        assert!(ValueKind::of_type("jsonb") == ValueKind::Json);
        assert!(ValueKind::of_type("_int4") == ValueKind::Text);
    }

    #[test]
    fn cell_text_is_one_bounded_line() {
        assert_eq!(single_line("a\nb", 10), "a b");
        assert_eq!(single_line("abcdef", 3), "abc…");
        assert_eq!(value_text(&Value::Bytes(vec![0, 255])).as_ref(), "\\x00ff");
        assert_eq!(value_text(&Value::Null).as_ref(), "NULL");
    }

    #[test]
    fn durations_use_a_readable_unit() {
        assert_eq!(format_duration(Duration::from_millis(42)).as_ref(), "42 ms");
        assert_eq!(
            format_duration(Duration::from_millis(1500)).as_ref(),
            "1.50 s"
        );
        assert_eq!(
            format_duration(Duration::from_secs(65)).as_ref(),
            "1 m 05 s"
        );
    }

    #[test]
    fn csv_fields_are_quoted_only_when_needed() {
        assert_eq!(csv_field("plain"), "plain");
        assert_eq!(csv_field("a,b"), "\"a,b\"");
        assert_eq!(csv_field("say \"hi\""), "\"say \"\"hi\"\"\"");
        assert_eq!(csv_field("two\nlines"), "\"two\nlines\"");
    }

    #[test]
    fn statement_lines_are_indented_under_the_first() {
        let entry = statement_entry("db", "select 1\nfrom t");
        let mut lines = entry.lines();
        let first = lines
            .next()
            .unwrap_or_default();
        let second = lines
            .next()
            .unwrap_or_default();
        assert!(first.ends_with("db> select 1"));
        assert_eq!(second.trim_start(), "from t");
        assert_eq!(
            second.len() - "from t".len(),
            first.len() - "select 1".len()
        );
    }

    #[test]
    fn a_dragged_range_covers_the_rectangle_in_any_direction() {
        let range = CellRange {
            anchor: (5, 3),
            head: (2, 1),
        };
        assert_eq!(range.rows(), 2..6);
        assert_eq!(range.cols(), 1..4);
        assert!(range.contains(2, 1));
        assert!(range.contains(5, 3));
        assert!(!range.contains(6, 3));
        assert!(!range.contains(4, 0));
    }

    #[test]
    fn output_lines_split_into_time_prompt_sql_and_errors() {
        let text = "[t1] db> select 1\n         from t\n[t2] 1 row\n[t3] ERROR: boom";
        let kinds: Vec<(&str, OutputSegment)> = output_segments(text)
            .into_iter()
            .filter(|(_, segment)| *segment != OutputSegment::Plain)
            .map(|(range, segment)| {
                (
                    text.get(range)
                        .unwrap_or_default()
                        .trim_end(),
                    segment,
                )
            })
            .collect();
        assert_eq!(
            kinds,
            vec![
                ("[t1]", OutputSegment::Time),
                ("db>", OutputSegment::Prompt),
                (" select 1", OutputSegment::Sql),
                ("         from t", OutputSegment::Sql),
                ("[t2]", OutputSegment::Time),
                ("[t3]", OutputSegment::Time),
                ("ERROR: boom", OutputSegment::Error),
            ]
        );
    }
}
