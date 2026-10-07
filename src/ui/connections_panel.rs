use gpui_kit::base::v_flex;
use gpui_kit::component::dock::{BasePanel, Panel, PanelEvent};
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::{ActiveTheme as _, Icon, IconName};
use gpui_kit::*;

pub struct ConnectionsPanel {
    focus_handle: FocusHandle,
    search_input: Entity<InputState>,
}

impl ConnectionsPanel {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let input =
            cx.new(|cx| InputState::new(window, cx).placeholder("Search connection names..."));

        Self {
            focus_handle: cx.focus_handle(),
            search_input: input,
        }
    }

    fn render_search_input(&self, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .border_b_1()
            .p_0()
            .border_color(cx.theme().border)
            .child(
                Input::new(&self.search_input)
                    .line_height(relative(1.))
                    .prefix(Icon::new(IconName::Search))
                    .appearance(false),
            )
    }
}

impl EventEmitter<PanelEvent> for ConnectionsPanel {}

impl Focusable for ConnectionsPanel {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl BasePanel for ConnectionsPanel {
    fn panel_name(&self) -> &'static str {
        "ConnectionsPanel"
    }
}

impl Panel for ConnectionsPanel {
    fn title_bar(&self, _cx: &App) -> bool {
        false
    }
}

impl Render for ConnectionsPanel {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .size_full()
            .child(self.render_search_input(cx))
            .child(
                div()
                    .size_full()
                    .flex()
                    .justify_center()
                    .items_center()
                    .child("Project Connections"),
            )
    }
}
