use std::time::Duration;

use dbolt_database::Value;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::dock::{BasePanel, Panel, PanelEvent};
use gpui_kit::component::tab::{Tab, TabBar};
use gpui_kit::component::table::{Column as TableColumn, DataTable, TableDelegate, TableState};
use gpui_kit::component::{ActiveTheme, IconName, Sizable};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;

use crate::components::button::Button;
use crate::components::icon_button::IconButton;
use crate::query_execution::{ExecutionState, QueryExecution, QueryExecutionEvent};
use crate::theme::{LABEL_SIZE_SMALL, PANEL_HEADER_HEIGHT};

/// Longest text drawn in a cell. Laying out a whole document for a narrow
/// cell would cost time on every frame for nothing the user can see.
const MAX_CELL_CHARS: usize = 256;
const MAX_TAB_LABEL_CHARS: usize = 32;
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

struct ResultTableDelegate {
    execution: Entity<QueryExecution>,
    columns: Vec<TableColumn>,
    kinds: Vec<ValueKind>,
}

impl ResultTableDelegate {
    fn new(execution: Entity<QueryExecution>) -> Self {
        Self {
            execution,
            columns: Vec::new(),
            kinds: Vec::new(),
        }
    }

    fn sync_columns(&mut self, cx: &App) {
        let execution = self.execution.read(cx);
        self.kinds = execution
            .columns()
            .iter()
            .map(|column| ValueKind::of_type(&column.type_name))
            .collect();
        self.columns = execution
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
                let widest = widest_value.max(column.name.chars().count());
                let width = (widest as f32 * CHAR_WIDTH + CELL_PADDING)
                    .clamp(MIN_COLUMN_WIDTH, MAX_COLUMN_WIDTH);
                let table_column = TableColumn::new(
                    SharedString::from(column_ix.to_string()),
                    column.name.clone(),
                )
                .width(px(width))
                .movable(false);
                if self.kinds.get(column_ix) == Some(&ValueKind::Number) {
                    table_column.text_right()
                } else {
                    table_column
                }
            })
            .collect();
    }

    fn value(&self, row_ix: usize, col_ix: usize, cx: &App) -> Option<Value> {
        self.execution
            .read(cx)
            .rows()
            .get(row_ix)
            .and_then(|row| row.get(col_ix))
            .cloned()
    }
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

    fn render_td(
        &mut self,
        row_ix: usize,
        col_ix: usize,
        _window: &mut Window,
        cx: &mut Context<TableState<Self>>,
    ) -> impl IntoElement {
        let execution = self.execution.read(cx);
        let Some(value) = execution
            .rows()
            .get(row_ix)
            .and_then(|row| row.get(col_ix))
        else {
            return div();
        };
        let theme = cx.theme();
        let kind = self
            .kinds
            .get(col_ix)
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
        div()
            .w_full()
            .truncate()
            .text_color(color)
            .when(
                matches!(value, Value::Null | Value::Unsupported(_)),
                |this| this.italic(),
            )
            .child(value_text(value))
    }

    fn cell_text(&self, row_ix: usize, col_ix: usize, cx: &App) -> String {
        match self.value(row_ix, col_ix, cx) {
            Some(Value::Text(text)) => text,
            Some(value) => value_text(&value).to_string(),
            None => String::new(),
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

struct ResultTab {
    execution: Entity<QueryExecution>,
    label: SharedString,
    table: Entity<TableState<ResultTableDelegate>>,
    _subscriptions: [Subscription; 2],
}

pub struct ResultsPanel {
    focus_handle: FocusHandle,
    tabs: Vec<ResultTab>,
    active_tab: usize,
}

impl ResultsPanel {
    pub fn new(_window: &mut Window, cx: &mut Context<Self>) -> Self {
        Self {
            focus_handle: cx.focus_handle(),
            tabs: Vec::new(),
            active_tab: 0,
        }
    }

    pub fn show_execution(
        &mut self,
        execution: Entity<QueryExecution>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let table = cx.new(|cx| {
            TableState::new(ResultTableDelegate::new(execution.clone()), window, cx)
                .sortable(false)
                .col_movable(false)
        });
        let events = cx.subscribe(&execution, {
            let table = table.clone();
            move |_, _, event, cx| {
                table.update(cx, |table, cx| {
                    if let QueryExecutionEvent::Finished = event {
                        table
                            .delegate_mut()
                            .sync_columns(cx);
                        table.refresh(cx);
                    }
                    cx.notify();
                });
            }
        });
        let changes = cx.observe(&execution, |_, _, cx| cx.notify());
        let label = single_line(
            execution
                .read(cx)
                .statement()
                .trim(),
            MAX_TAB_LABEL_CHARS,
        )
        .into();
        self.tabs.push(ResultTab {
            execution,
            label,
            table,
            _subscriptions: [events, changes],
        });
        self.active_tab = self.tabs.len() - 1;
        cx.notify();
    }

    fn close_tab(&mut self, tab_ix: usize, cx: &mut Context<Self>) {
        if tab_ix >= self.tabs.len() {
            return;
        }
        self.tabs.remove(tab_ix);
        if self.active_tab > tab_ix || self.active_tab >= self.tabs.len() {
            self.active_tab = self
                .active_tab
                .saturating_sub(1);
        }
        cx.notify();
    }

    fn render_tab_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        TabBar::new("result-tabs")
            .selected_index(self.active_tab)
            .on_click(cx.listener(|this, tab_ix: &usize, _, cx| {
                this.active_tab = *tab_ix;
                cx.notify();
            }))
            .children(
                self.tabs
                    .iter()
                    .enumerate()
                    .map(|(tab_ix, tab)| {
                        Tab::new()
                            .child(tab.label.clone())
                            .suffix(
                                IconButton::new(("close-result", tab_ix), IconName::Close)
                                    .small()
                                    .tooltip("Close result")
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        cx.stop_propagation();
                                        this.close_tab(tab_ix, cx);
                                    })),
                            )
                    }),
            )
    }

    fn render_header(&self, tab: &ResultTab, cx: &mut Context<Self>) -> impl IntoElement {
        let execution = tab.execution.read(cx);
        let theme = cx.theme();
        let duration = execution
            .duration()
            .map(format_duration);
        let (summary, color) = match execution.state() {
            ExecutionState::Running => ("Running…".to_string(), theme.muted_foreground),
            ExecutionState::Cancelled => ("Cancelled".to_string(), theme.muted_foreground),
            ExecutionState::Failed(_) => ("Failed".to_string(), theme.danger),
            ExecutionState::Ready if execution.columns().is_empty() => (
                plural(
                    execution
                        .rows_affected()
                        .unwrap_or(0),
                    "row affected",
                    "rows affected",
                ),
                theme.muted_foreground,
            ),
            ExecutionState::Ready => {
                let rows = plural(execution.rows().len() as u64, "row", "rows");
                let summary = if execution.is_loading_more() {
                    format!("{rows}, loading more…")
                } else if execution.has_more() {
                    format!("{rows}, more available")
                } else {
                    rows
                };
                (summary, theme.muted_foreground)
            }
        };
        let running = execution.state() == &ExecutionState::Running || execution.is_loading_more();

        h_flex()
            .flex_none()
            .h(PANEL_HEADER_HEIGHT)
            .px_2()
            .gap_2()
            .border_b_1()
            .border_color(theme.border)
            .text_size(LABEL_SIZE_SMALL)
            .child(
                div()
                    .text_color(color)
                    .child(summary),
            )
            .when_some(duration, |this, duration| {
                this.child(
                    div()
                        .text_color(theme.muted_foreground)
                        .child(duration),
                )
            })
            .child(div().flex_1())
            .when(running, |this| {
                let execution = tab.execution.clone();
                this.child(
                    Button::new("cancel-execution")
                        .label("Cancel")
                        .label_size(LABEL_SIZE_SMALL)
                        .tooltip("Stop waiting for this statement")
                        .on_click(move |_, _, cx| {
                            execution.update(cx, |execution, cx| execution.cancel(cx));
                        }),
                )
            })
    }

    fn render_body(&self, tab: &ResultTab, cx: &mut Context<Self>) -> AnyElement {
        let execution = tab.execution.read(cx);
        let theme = cx.theme();
        let has_columns = !execution.columns().is_empty();
        let message = |text: SharedString| {
            div()
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .text_size(LABEL_SIZE_SMALL)
                .text_color(theme.muted_foreground)
                .child(text)
                .into_any_element()
        };
        let error = match execution.state() {
            ExecutionState::Failed(error) => Some(error.clone()),
            _ => None,
        };

        if has_columns {
            return v_flex()
                .size_full()
                .child(
                    div()
                        .flex_1()
                        .min_h_0()
                        .child(
                            DataTable::new(&tab.table)
                                .bordered(false)
                                .small(),
                        ),
                )
                .when_some(error, |this, error| {
                    this.child(
                        div()
                            .flex_none()
                            .px_2()
                            .py_1()
                            .border_t_1()
                            .border_color(theme.border)
                            .text_size(LABEL_SIZE_SMALL)
                            .text_color(theme.danger)
                            .child(error),
                    )
                })
                .into_any_element();
        }

        match execution.state() {
            ExecutionState::Running => message("Running…".into()),
            ExecutionState::Cancelled => message("Cancelled".into()),
            ExecutionState::Failed(error) => div()
                .id("result-error")
                .size_full()
                .overflow_y_scroll()
                .p_2()
                .text_size(LABEL_SIZE_SMALL)
                .text_color(theme.danger)
                .child(error.clone())
                .into_any_element(),
            ExecutionState::Ready => message(
                plural(
                    execution
                        .rows_affected()
                        .unwrap_or(0),
                    "row affected",
                    "rows affected",
                )
                .into(),
            ),
        }
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
        let Some(tab) = self.tabs.get(self.active_tab) else {
            return v_flex()
                .size_full()
                .justify_center()
                .items_center()
                .text_size(LABEL_SIZE_SMALL)
                .text_color(cx.theme().muted_foreground)
                .child("Run a statement to see its results here")
                .into_any_element();
        };

        v_flex()
            .size_full()
            .track_focus(&self.focus_handle)
            .child(self.render_tab_bar(cx))
            .child(self.render_header(tab, cx))
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .child(self.render_body(tab, cx)),
            )
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use dbolt_database::Value;

    use super::{ValueKind, format_duration, single_line, value_text};

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
}
