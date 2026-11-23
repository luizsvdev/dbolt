use std::sync::Arc;

use dbolt_database::DriverRegistry;
use dbolt_postgres::PostgresDriver;
use gpui_kit::base::dock::DockPlacement;
use gpui_kit::base::v_flex;
use gpui_kit::component::dock::panel_handle;
use gpui_kit::component::notification::Notification;
use gpui_kit::component::{ActiveTheme, WindowExt};
use gpui_kit::*;
use gpui_kit::{
    Context, Entity, Window,
    component::dock::{DockArea, DockSkin},
};

use crate::project::{Project, ProjectEvent};
use crate::ui::connections_panel::ConnectionsPanel;
use crate::ui::editor_panel::{EditorPanel, EditorPanelEvent};
use crate::ui::project_panel::ProjectPanel;
use crate::ui::results_panel::ResultsPanel;
use crate::ui::status_bar::AppStatusBar;
use crate::ui::title_bar::AppTitleBar;

pub struct Workspace {
    dock_area: Entity<DockArea>,
    title_bar: Entity<AppTitleBar>,
    status_bar: Entity<AppStatusBar>,
    _subscriptions: Vec<Subscription>,
}

impl Workspace {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let project = cx.new(Project::new);
        let project_errors = cx.subscribe_in(&project, window, |_, _, event, window, cx| {
            if let ProjectEvent::Error(message) = event {
                window.push_notification(Notification::error(message.clone()), cx);
            }
        });
        let (dock_area, _dock_skin) = DockSkin::dock_area("main-dock", Some(1), window, cx);

        let mut registry = DriverRegistry::new();
        registry.register(Arc::new(PostgresDriver::new()));
        let registry = Arc::new(registry);

        let connections_panel =
            cx.new(|cx| ConnectionsPanel::new(registry, project.clone(), window, cx));
        let project_panel = cx.new(|cx| ProjectPanel::new(project.clone(), cx));
        let editor_panel = cx.new(|cx| EditorPanel::new(project.clone(), window, cx));
        let results_panel = cx.new(|cx| ResultsPanel::new(window, cx));
        let executions = cx.subscribe_in(&editor_panel, window, {
            let results_panel = results_panel.clone();
            move |_, _, event, window, cx| match event {
                EditorPanelEvent::ExecutionStarted(execution) => {
                    results_panel.update(cx, |panel, cx| {
                        panel.show_execution(execution.clone(), window, cx)
                    });
                }
            }
        });

        dock_area.update(cx, |area, cx| {
            const DOCK_SIZE: Pixels = px(240.);
            const BOTTOM_DOCK_SIZE: Pixels = px(260.);
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
            area.add_panel_view(
                panel_handle(editor_panel),
                DockPlacement::Center,
                None,
                window,
                cx,
            );
            area.add_panel_view(
                panel_handle(results_panel),
                DockPlacement::Bottom,
                Some(BOTTOM_DOCK_SIZE),
                window,
                cx,
            );
        });

        let title_bar = cx.new(|cx| AppTitleBar::new(project.clone(), window, cx));
        let status_bar = cx.new(|_| AppStatusBar::new(dock_area.clone()));
        Self {
            dock_area,
            title_bar,
            status_bar,
            _subscriptions: vec![project_errors, executions],
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
