use gpui_kit::base::dock::DockPlacement;
use gpui_kit::base::v_flex;
use gpui_kit::component::ActiveTheme;
use gpui_kit::component::dock::panel_handle;
use gpui_kit::*;
use gpui_kit::{
    Context, Entity, Window,
    component::dock::{DockArea, DockSkin},
};

use crate::ui::connections_panel::ConnectionsPanel;
use crate::ui::project_panel::ProjectPanel;
use crate::ui::status_bar::AppStatusBar;
use crate::ui::title_bar::AppTitleBar;

pub struct Workspace {
    dock_area: Entity<DockArea>,
    title_bar: Entity<AppTitleBar>,
    status_bar: Entity<AppStatusBar>,
}

impl Workspace {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let (dock_area, _dock_skin) = DockSkin::dock_area("main-dock", Some(1), window, cx);

        let connections_panel = cx.new(|cx| ConnectionsPanel::new(window, cx));
        let project_panel = cx.new(|cx| ProjectPanel::new(cx));

        dock_area.update(cx, |area, cx| {
            const DOCK_SIZE: Pixels = px(240.);
            area.add_panel_view(
                panel_handle(connections_panel),
                DockPlacement::Left,
                Some(DOCK_SIZE),
                window,
                cx,
            );
            area.add_panel_view(
                panel_handle(project_panel),
                DockPlacement::Right,
                Some(DOCK_SIZE),
                window,
                cx,
            );
        });

        let title_bar = cx.new(|cx| AppTitleBar::new(window, cx));
        let status_bar = cx.new(|_| AppStatusBar::new(dock_area.clone()));
        Self {
            dock_area,
            title_bar,
            status_bar,
        }
    }
}

impl Render for Workspace {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .size_full()
            .child(self.title_bar.clone())
            .child(
                div()
                    .flex_1()
                    .border_t_1()
                    .border_b_1()
                    .border_color(cx.theme().border)
                    .child(self.dock_area.clone()),
            )
            .child(self.status_bar.clone())
    }
}
