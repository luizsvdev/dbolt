use gpui_kit::base::v_flex;
use gpui_kit::component::ActiveTheme;
use gpui_kit::component::dock::{BasePanel, Panel, PanelEvent};
use gpui_kit::*;

use crate::query_execution::QueryExecution;

pub struct ResultsPanel {
    focus_handle: FocusHandle,
    executions: Vec<Entity<QueryExecution>>,
}

impl ResultsPanel {
    pub fn new(_window: &mut Window, cx: &mut Context<Self>) -> Self {
        Self {
            focus_handle: cx.focus_handle(),
            executions: Vec::new(),
        }
    }

    pub fn show_execution(
        &mut self,
        execution: Entity<QueryExecution>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.executions
            .push(execution);
        cx.notify();
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
    fn title_bar(&self, _: &App) -> bool {
        false
    }
}

impl Render for ResultsPanel {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .size_full()
            .justify_center()
            .items_center()
            .text_color(cx.theme().muted_foreground)
            .child(format!("Results ({})", self.executions.len()))
    }
}
