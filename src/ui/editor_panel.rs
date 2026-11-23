use gpui_kit::base::v_flex;
use gpui_kit::component::ActiveTheme;
use gpui_kit::component::dock::{BasePanel, Panel, PanelEvent};
use gpui_kit::*;

use crate::project::Project;
use crate::query_execution::QueryExecution;

pub enum EditorPanelEvent {
    ExecutionStarted(Entity<QueryExecution>),
}

pub struct EditorPanel {
    focus_handle: FocusHandle,
    _project: Entity<Project>,
}

impl EditorPanel {
    pub fn new(project: Entity<Project>, _window: &mut Window, cx: &mut Context<Self>) -> Self {
        Self {
            focus_handle: cx.focus_handle(),
            _project: project,
        }
    }
}

impl EventEmitter<PanelEvent> for EditorPanel {}
impl EventEmitter<EditorPanelEvent> for EditorPanel {}

impl Focusable for EditorPanel {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl BasePanel for EditorPanel {
    fn panel_name(&self) -> &'static str {
        "EditorPanel"
    }
}

impl Panel for EditorPanel {
    fn title_bar(&self, _: &App) -> bool {
        false
    }
}

impl Render for EditorPanel {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .size_full()
            .justify_center()
            .items_center()
            .text_color(cx.theme().muted_foreground)
            .child("Editor")
    }
}
