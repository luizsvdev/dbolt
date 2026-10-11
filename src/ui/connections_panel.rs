use gpui_kit::base::v_flex;
use gpui_kit::component::dock::{BasePanel, Panel, PanelEvent};
use gpui_kit::component::input::InputState;
use gpui_kit::*;

use crate::components::search_bar::SearchBar;

pub struct ConnectionsPanel {
    focus_handle: FocusHandle,
    search_input: Entity<InputState>,
}

impl ConnectionsPanel {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let input = cx.new(|cx| InputState::new(window, cx).placeholder("Search connections..."));

        Self {
            focus_handle: cx.focus_handle(),
            search_input: input,
        }
    }

    fn render_search_input(&self) -> impl IntoElement {
        SearchBar::new(&self.search_input)
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
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .flex_1()
            .size_full()
            .child(self.render_search_input())
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
