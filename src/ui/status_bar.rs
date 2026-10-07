use gpui_kit::component::button::ButtonVariants;
use gpui_kit::component::separator::Separator;
use gpui_kit::{
    Context, IntoElement, Render,
    assets::IconName,
    component::{button::Button, status_bar::StatusBar, *},
};
use gpui_kit::{Styled, Window, px};

pub struct AppStatusBar;

impl Render for AppStatusBar {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        StatusBar::new()
            .h(px(30.))
            .border_0()
            .left(
                Button::new("toggle-left-panel")
                    .ghost()
                    .small()
                    .icon(IconName::PanelLeft),
            )
            .left(Separator::vertical())
            .left(
                Button::new("branch")
                    .ghost()
                    .small()
                    .icon(IconName::Github)
                    .label("main")
                    .on_click(|_, _window, _cx| { /* ... */ }),
            )
            .right(
                Button::new("go-to-line")
                    .ghost()
                    .small()
                    .label("36:29")
                    .tooltip("Go to Line/Column")
                    .on_click(cx.listener(|_this, _, _window, _cx| { /* ... */ })),
            )
            .right(Separator::vertical())
            .right(
                Button::new("toggle-right-panel")
                    .ghost()
                    .small()
                    .icon(IconName::PanelRight),
            )
    }
}
