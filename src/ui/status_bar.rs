use gpui_kit::base::dock::{DockArea, DockPlacement};
use gpui_kit::{Context, IntoElement, Render, assets::IconName, component::status_bar::StatusBar};
use gpui_kit::{Entity, Styled, Window, px};

use crate::components::icon_button::IconButton;

pub struct AppStatusBar {
    dock_area: Entity<DockArea>,
}

impl AppStatusBar {
    pub fn new(dock_area: Entity<DockArea>) -> Self {
        Self { dock_area }
    }
}

impl Render for AppStatusBar {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        StatusBar::new()
            .h(px(30.))
            .border_0()
            .left(
                IconButton::new("toggle-left-panel", IconName::PanelLeft)
                    .small()
                    .on_click({
                        let dock_area = self.dock_area.clone();
                        move |_, window, cx| {
                            dock_area.update(cx, |area, cx| {
                                area.toggle_dock(DockPlacement::Left, window, cx)
                            })
                        }
                    }),
            )
            .right(
                IconButton::new("toggle-bottom-panel", IconName::PanelBottom)
                    .small()
                    .on_click({
                        let dock_area = self.dock_area.clone();
                        move |_, window, cx| {
                            dock_area.update(cx, |area, cx| {
                                area.toggle_dock(DockPlacement::Bottom, window, cx)
                            })
                        }
                    }),
            )
            .right(
                IconButton::new("toggle-right-panel", IconName::PanelRight)
                    .small()
                    .on_click({
                        let dock_area = self.dock_area.clone();
                        move |_, window, cx| {
                            dock_area.update(cx, |area, cx| {
                                area.toggle_dock(DockPlacement::Right, window, cx)
                            })
                        }
                    }),
            )
    }
}
