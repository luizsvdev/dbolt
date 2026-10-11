use gpui_kit::base::v_flex;
use gpui_kit::component::dock::{BasePanel, Panel, PanelEvent};
use gpui_kit::{
    App, Context, EventEmitter, FocusHandle, Focusable, IntoElement, ParentElement, Render, Styled,
    Window, div,
};

pub struct ProjectPanel {
    focus_handle: FocusHandle,
}

impl ProjectPanel {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let focus_handle = cx.focus_handle();
        Self { focus_handle }
    }
}

impl EventEmitter<PanelEvent> for ProjectPanel {}

impl Focusable for ProjectPanel {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl BasePanel for ProjectPanel {
    fn panel_name(&self) -> &'static str {
        "ProjectPanel"
    }
}

impl Panel for ProjectPanel {
    fn title_bar(&self, _: &App) -> bool {
        false
    }
}

impl Render for ProjectPanel {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        v_flex().size_full().child(
            div()
                .size_full()
                .flex()
                .justify_center()
                .items_center()
                .child("Project Files"),
        )
    }
}
