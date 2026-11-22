use gpui_kit::base::v_flex;
use gpui_kit::component::ActiveTheme;
use gpui_kit::component::dock::{BasePanel, Panel, PanelEvent};
use gpui_kit::*;

/// Holds a dock slot until the real panel for it exists.
pub struct PlaceholderPanel {
    focus_handle: FocusHandle,
    name: &'static str,
    label: SharedString,
}

impl PlaceholderPanel {
    pub fn new(name: &'static str, label: impl Into<SharedString>, cx: &mut Context<Self>) -> Self {
        Self {
            focus_handle: cx.focus_handle(),
            name,
            label: label.into(),
        }
    }
}

impl EventEmitter<PanelEvent> for PlaceholderPanel {}

impl Focusable for PlaceholderPanel {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl BasePanel for PlaceholderPanel {
    fn panel_name(&self) -> &'static str {
        self.name
    }
}

impl Panel for PlaceholderPanel {
    fn title_bar(&self, _: &App) -> bool {
        false
    }
}

impl Render for PlaceholderPanel {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .size_full()
            .justify_center()
            .items_center()
            .text_color(cx.theme().muted_foreground)
            .child(self.label.clone())
    }
}
