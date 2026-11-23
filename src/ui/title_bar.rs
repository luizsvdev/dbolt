use std::path::{Path, PathBuf};

use gpui_kit::assets::IconName;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::ActiveTheme;
use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::component::popover::Popover;
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use gpui_kit::{
    Entity, ParentElement, Render,
    component::{TitleBar, menu::AppMenuBar},
    div,
};

use crate::components::button::Button;
use crate::components::icon_button::IconButton;
use crate::components::search_bar::SearchBar;
use crate::project::{Project, prompt_open_folder};
use crate::theme::LABEL_SIZE_SMALL;

const PICKER_WIDTH: Pixels = px(360.);
const PICKER_ROW_HEIGHT: Pixels = px(26.);

pub struct AppTitleBar {
    app_menu_bar: Entity<AppMenuBar>,
    project: Entity<Project>,
    search_input: Entity<InputState>,
    picker_open: bool,
    _subscriptions: Vec<Subscription>,
}

impl AppTitleBar {
    pub fn new(project: Entity<Project>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let app_menu_bar = AppMenuBar::new(cx);
        let search_input = cx.new(|cx| InputState::new(window, cx).placeholder("Search projects…"));
        let subscriptions = vec![
            cx.observe_window_activation(window, |_, _, cx| cx.notify()),
            cx.observe(&project, |_, _, cx| cx.notify()),
            cx.subscribe_in(
                &search_input,
                window,
                |this, _, event, window, cx| match event {
                    InputEvent::Change => cx.notify(),
                    InputEvent::PressEnter { .. } => {
                        if let Some(path) = this
                            .filtered_projects(cx)
                            .into_iter()
                            .next()
                        {
                            this.open_recent_project(path, window, cx);
                        }
                    }
                    InputEvent::Focus | InputEvent::Blur => {}
                },
            ),
        ];

        Self {
            app_menu_bar,
            project,
            search_input,
            picker_open: false,
            _subscriptions: subscriptions,
        }
    }

    fn filtered_projects(&self, cx: &App) -> Vec<PathBuf> {
        let query = self
            .search_input
            .read(cx)
            .value()
            .to_lowercase();
        self.project
            .read(cx)
            .recent_projects()
            .iter()
            .filter(|path| {
                query.is_empty()
                    || path
                        .to_string_lossy()
                        .to_lowercase()
                        .contains(&query)
            })
            .cloned()
            .collect()
    }

    fn set_picker_open(&mut self, open: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.picker_open = open;
        if open {
            self.search_input
                .update(cx, |input, cx| {
                    input.set_value("", window, cx);
                    input.focus(window, cx);
                });
        }
        cx.notify();
    }

    fn open_recent_project(&mut self, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        self.set_picker_open(false, window, cx);
        self.project
            .update(cx, |project, cx| project.open(path, cx));
    }

    fn render_project_picker(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let label = self
            .project
            .read(cx)
            .name()
            .unwrap_or_else(|| "Open Folder".into());
        let title_bar = cx.weak_entity();

        Popover::new("project-picker")
            .p_0()
            .overflow_hidden()
            .open(self.picker_open)
            .on_open_change({
                let title_bar = title_bar.clone();
                move |open, window, cx| {
                    if let Err(error) = title_bar.update(cx, |this, cx| {
                        this.set_picker_open(*open, window, cx)
                    }) {
                        eprintln!("title bar dropped before the project picker changed: {error:#}");
                    }
                }
            })
            .trigger(
                Button::new("project-menu")
                    .label(label)
                    .label_size(LABEL_SIZE_SMALL)
                    .tooltip("Open a Folder or a Recent Project"),
            )
            .content(move |_, _, cx| {
                let Some(title_bar) = title_bar.upgrade() else {
                    return div().into_any_element();
                };
                let this = title_bar.read(cx);
                let projects = this.filtered_projects(cx);
                let search_input = this.search_input.clone();
                let project = this.project.clone();

                let recent_projects = v_flex()
                    .p_1()
                    .when(projects.is_empty(), |this| {
                        this.child(
                            div()
                                .px_2()
                                .py_1()
                                .text_color(cx.theme().muted_foreground)
                                .child("No recent projects"),
                        )
                    })
                    .children(projects.into_iter().enumerate().map(|(index, path)| {
                        let title_bar = title_bar.downgrade();
                        render_project_row(index, &path, cx).on_click(move |_, window, cx| {
                            let path = path.clone();
                            if let Err(error) = title_bar.update(cx, |this, cx| {
                                this.open_recent_project(path, window, cx)
                            }) {
                                eprintln!("title bar dropped before opening a project: {error:#}");
                            }
                        })
                    }));

                v_flex()
                    .w(PICKER_WIDTH)
                    .child(SearchBar::new(&search_input))
                    .child(recent_projects)
                    .child(
                        div()
                            .p_1()
                            .border_t_1()
                            .border_color(cx.theme().border)
                            .child(
                        picker_row("open-folder", cx)
                            .child("Open Folder…")
                            .on_click({
                                let title_bar = title_bar.downgrade();
                                move |_, window, cx| {
                                    if let Err(error) = title_bar.update(cx, |this, cx| {
                                        this.set_picker_open(false, window, cx)
                                    }) {
                                        eprintln!("title bar dropped before closing the project picker: {error:#}");
                                    }
                                    prompt_open_folder(&project, cx);
                                }
                            }),
                            ),
                    )
                    .into_any_element()
            })
    }
}

fn picker_row(id: impl Into<ElementId>, cx: &App) -> Stateful<Div> {
    h_flex()
        .id(id)
        .h(PICKER_ROW_HEIGHT)
        .px_2()
        .gap_1()
        .rounded(cx.theme().radius)
        .cursor_pointer()
        .hover(|style| style.bg(cx.theme().accent))
}

fn render_project_row(index: usize, path: &Path, cx: &App) -> Stateful<Div> {
    let name = path
        .file_name()
        .map(|name| {
            name.to_string_lossy()
                .into_owned()
        })
        .unwrap_or_else(|| {
            path.to_string_lossy()
                .into_owned()
        });

    picker_row(("recent-project", index), cx)
        .overflow_hidden()
        .child(div().flex_none().child(name))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .truncate()
                .text_color(cx.theme().muted_foreground)
                .child(format!("· {}", display_path(path))),
        )
}

fn display_path(path: &Path) -> String {
    match std::env::home_dir().and_then(|home| {
        path.strip_prefix(&home)
            .ok()
            .map(Path::to_path_buf)
    }) {
        Some(relative)
            if relative
                .as_os_str()
                .is_empty() =>
        {
            "~".to_string()
        }
        Some(relative) => format!("~/{}", relative.display()),
        None => path.display().to_string(),
    }
}

impl Render for AppTitleBar {
    fn render(
        &mut self,
        window: &mut gpui_kit::Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
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
                        IconButton::new("menu", IconName::Menu)
                            .small()
                            .tooltip("Open Application Menu"),
                    )
                    .child(self.app_menu_bar.clone())
                    .child(self.render_project_picker(cx)),
            )
    }
}
