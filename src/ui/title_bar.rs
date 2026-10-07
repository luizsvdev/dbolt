use gpui_kit::component::button::{ButtonCustomVariant, ButtonVariants};
use gpui_kit::component::{ActiveTheme, IconName};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use gpui_kit::{
    Entity, ParentElement, Render,
    component::{TitleBar, button::Button, menu::AppMenuBar},
    div,
};

pub struct AppTitleBar {
    app_menu_bar: Entity<AppMenuBar>,
    _subscriptions: Vec<Subscription>,
}

impl AppTitleBar {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let app_menu_bar = AppMenuBar::new(cx);
        let activation = cx.observe_window_activation(window, |_, _, cx| cx.notify());

        Self {
            app_menu_bar,
            _subscriptions: vec![activation],
        }
    }
}

impl Render for AppTitleBar {
    fn render(
        &mut self,
        window: &mut gpui_kit::Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let menu_style = ButtonCustomVariant::new(cx)
            .color(cx.theme().transparent)
            .foreground(cx.theme().foreground)
            .hover(cx.theme().secondary_hover)
            .active(cx.theme().secondary_active);

        let background = if window.is_window_active() {
            cx.theme().title_bar
        } else {
            cx.theme().background
        };

        TitleBar::new()
            .bg(background)
            .border_b_0()
            .when(!cfg!(target_os = "macos"), |this| this.pl_2())
            .child(
                div()
                    .flex()
                    .py(px(1.))
                    .items_center()
                    .gap_2()
                    .child(
                        Button::new("menu")
                            .custom(menu_style)
                            .tooltip("Open Application Menu")
                            .icon(IconName::Menu),
                    )
                    .child(self.app_menu_bar.clone()),
            )
    }
}
