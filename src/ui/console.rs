use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::ActiveTheme;
use gpui_kit::component::dock::{BasePanel, Panel, PanelEvent};
use gpui_kit::component::scroll::Scrollbar;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;

use crate::query_execution::{ExecutionState, QueryExecution, QueryExecutionEvent};
use crate::theme::LABEL_SIZE_SMALL;
use crate::ui::results_panel::{format_duration, plural};

const ROW_HEIGHT: Pixels = px(24.);
const MAX_STATEMENT_CHARS: usize = 200;

/// What the console shows about one execution. A copy rather than the
/// execution itself, so the history does not keep every result's rows alive
/// after its result tab is closed.
struct ConsoleEntry {
    time: SharedString,
    connection_name: SharedString,
    statement: SharedString,
    preview: SharedString,
    duration: Option<SharedString>,
    outcome: SharedString,
    failed: bool,
    _subscription: Subscription,
}

impl ConsoleEntry {
    fn update_from(&mut self, execution: &QueryExecution) {
        self.duration = execution
            .duration()
            .map(format_duration);
        self.failed = matches!(execution.state(), ExecutionState::Failed(_));
        self.outcome = match execution.state() {
            ExecutionState::Running => "running…".into(),
            ExecutionState::Cancelled => "cancelled".into(),
            ExecutionState::Failed(error) => one_line(error, MAX_STATEMENT_CHARS).into(),
            ExecutionState::Ready if execution.columns().is_empty() => plural(
                execution
                    .rows_affected()
                    .unwrap_or(0),
                "row affected",
                "rows affected",
            )
            .into(),
            ExecutionState::Ready => {
                let rows = plural(execution.rows().len() as u64, "row", "rows");
                if execution.has_more() {
                    format!("{rows}+").into()
                } else {
                    rows.into()
                }
            }
        };
    }
}

pub struct ConsolePanel {
    focus_handle: FocusHandle,
    entries: Vec<ConsoleEntry>,
    scroll_handle: UniformListScrollHandle,
}

impl ConsolePanel {
    pub fn new(cx: &mut Context<Self>) -> Self {
        Self {
            focus_handle: cx.focus_handle(),
            entries: Vec::new(),
            scroll_handle: UniformListScrollHandle::new(),
        }
    }

    pub fn add_execution(&mut self, execution: Entity<QueryExecution>, cx: &mut Context<Self>) {
        let entry_ix = self.entries.len();
        let subscription = cx.subscribe(&execution, move |this, execution, event, cx| {
            if let QueryExecutionEvent::Finished
            | QueryExecutionEvent::RowsLoaded
            | QueryExecutionEvent::Failed
            | QueryExecutionEvent::Cancelled = event
                && let Some(entry) = this.entries.get_mut(entry_ix)
            {
                entry.update_from(execution.read(cx));
                cx.notify();
            }
        });
        let execution = execution.read(cx);
        let mut entry = ConsoleEntry {
            time: chrono::Local::now()
                .format("%H:%M:%S")
                .to_string()
                .into(),
            connection_name: execution
                .connection_name()
                .clone(),
            statement: execution.statement().clone(),
            preview: one_line(execution.statement(), MAX_STATEMENT_CHARS).into(),
            duration: None,
            outcome: SharedString::default(),
            failed: false,
            _subscription: subscription,
        };
        entry.update_from(execution);
        self.entries.push(entry);
        self.scroll_handle
            .scroll_to_item(entry_ix, ScrollStrategy::Top);
        cx.notify();
    }

    fn render_entries(
        &mut self,
        range: std::ops::Range<usize>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Vec<Stateful<Div>> {
        let theme = cx.theme();
        range
            .filter_map(|entry_ix| {
                let entry = self.entries.get(entry_ix)?;
                let outcome_color = if entry.failed {
                    theme.danger
                } else {
                    theme.muted_foreground
                };
                let statement = entry.statement.clone();
                Some(
                    h_flex()
                        .id(("console-entry", entry_ix))
                        .h(ROW_HEIGHT)
                        .px_2()
                        .gap_3()
                        .text_size(LABEL_SIZE_SMALL)
                        .cursor_pointer()
                        .hover(|style| style.bg(theme.list_hover))
                        .tooltip(|window, cx| {
                            Tooltip::new("Click to copy the statement").build(window, cx)
                        })
                        .on_click(move |_, _, cx| {
                            cx.write_to_clipboard(ClipboardItem::new_string(statement.to_string()));
                        })
                        .child(
                            div()
                                .flex_none()
                                .text_color(theme.muted_foreground)
                                .child(entry.time.clone()),
                        )
                        .child(
                            div()
                                .flex_none()
                                .max_w(px(160.))
                                .truncate()
                                .text_color(theme.muted_foreground)
                                .child(entry.connection_name.clone()),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .font_family(theme.mono_font_family.clone())
                                .text_color(theme.foreground)
                                .child(entry.preview.clone()),
                        )
                        .when_some(entry.duration.clone(), |this, duration| {
                            this.child(
                                div()
                                    .flex_none()
                                    .text_color(theme.muted_foreground)
                                    .child(duration),
                            )
                        })
                        .child(
                            div()
                                .flex_none()
                                .max_w(px(320.))
                                .truncate()
                                .text_color(outcome_color)
                                .child(entry.outcome.clone()),
                        ),
                )
            })
            .collect()
    }
}

fn one_line(text: &str, max_chars: usize) -> String {
    let mut line = text
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    if let Some((cut, _)) = line
        .char_indices()
        .nth(max_chars)
    {
        line.truncate(cut);
        line.push('…');
    }
    line
}

impl EventEmitter<PanelEvent> for ConsolePanel {}

impl Focusable for ConsolePanel {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl BasePanel for ConsolePanel {
    fn panel_name(&self) -> &'static str {
        "ConsolePanel"
    }
}

impl Panel for ConsolePanel {
    fn title(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        "Console"
    }

    fn title_bar(&self, _: &App) -> bool {
        false
    }
}

impl Render for ConsolePanel {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.entries.is_empty() {
            return v_flex()
                .size_full()
                .justify_center()
                .items_center()
                .text_size(LABEL_SIZE_SMALL)
                .text_color(cx.theme().muted_foreground)
                .child("Executed statements appear here")
                .into_any_element();
        }

        div()
            .relative()
            .size_full()
            .track_focus(&self.focus_handle)
            .child(
                uniform_list(
                    "console-entries",
                    self.entries.len(),
                    cx.processor(Self::render_entries),
                )
                .size_full()
                .track_scroll(&self.scroll_handle),
            )
            .child(Scrollbar::vertical(&self.scroll_handle))
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::one_line;

    #[test]
    fn statements_are_shown_on_one_bounded_line() {
        assert_eq!(
            one_line("SELECT *\n  FROM users", 50),
            "SELECT * FROM users"
        );
        assert_eq!(one_line("SELECT 1", 3), "SEL…");
    }
}
