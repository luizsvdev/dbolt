use gpui_kit::base::h_flex;
use gpui_kit::component::{ActiveTheme, Icon, IconName, Sizable};
use gpui_kit::*;

/// The label that follows the cursor while a tree row is dragged.
pub struct DragPreview {
    icon: IconName,
    label: SharedString,
}

impl DragPreview {
    pub fn new(icon: IconName, label: impl Into<SharedString>) -> Self {
        Self {
            icon,
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
                Icon::new(self.icon.clone())
                    .small()
                    .text_color(theme.muted_foreground),
            )
            .child(self.label.clone())
    }
}
