use gpui_kit::base::h_flex;
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::{ActiveTheme, Icon, IconName, Sizable};
use gpui_kit::*;

use crate::theme::PANEL_HEADER_HEIGHT;

#[derive(IntoElement)]
pub struct SearchBar {
    state: Entity<InputState>,
    suffix: Option<AnyElement>,
}

impl SearchBar {
    pub fn new(state: &Entity<InputState>) -> Self {
        Self {
            state: state.clone(),
            suffix: None,
        }
    }

    pub fn suffix(mut self, suffix: impl IntoElement) -> Self {
        self.suffix = Some(suffix.into_any_element());
        self
    }
}

impl RenderOnce for SearchBar {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        h_flex()
            .flex_shrink_0()
            .w_full()
            .h(PANEL_HEADER_HEIGHT)
            .items_center()
            .px_2()
            .gap_1p5()
            .border_b_1()
            .border_color(cx.theme().border)
            .child(
                Icon::new(IconName::Search)
                    .small()
                    .text_color(cx.theme().muted_foreground),
            )
            .child(
                Input::new(&self.state)
                    .appearance(false)
                    .small()
                    .px_0()
                    .text_size(px(14.)),
            )
            .children(self.suffix)
    }
}
