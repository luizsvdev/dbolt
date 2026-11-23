use gpui_kit::base::h_flex;
use gpui_kit::component::ActiveTheme;
use gpui_kit::*;

use crate::components::icon_button::IconSource;
use crate::theme::ICON_SIZE_SMALL;

pub struct DragPreview {
    icon: IconSource,
    label: SharedString,
}

impl DragPreview {
    pub fn new(icon: impl Into<IconSource>, label: impl Into<SharedString>) -> Self {
        Self {
            icon: icon.into(),
            label: label.into(),
        }
    }
}

impl Render for DragPreview {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        h_flex()
            .gap_1p5()
            .px_2()
            .py_0p5()
            .rounded(theme.radius)
            .border_1()
            .border_color(theme.border)
            .bg(theme.popover)
            .text_color(theme.popover_foreground)
            .text_sm()
            .shadow_md()
            .child(
                svg()
                    .path(self.icon.path())
                    .size(ICON_SIZE_SMALL)
                    .flex_none()
                    .text_color(theme.muted_foreground),
            )
            .child(self.label.clone())
    }
}
