use std::collections::HashMap;
use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};

use dbolt_database::ConnectionId;
use gpui_kit::assets::IconName as AssetIconName;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::button::ButtonVariant;
use gpui_kit::component::dock::{BasePanel, Panel, PanelEvent};
use gpui_kit::component::input::{Editor, EditorState, InputEvent};
use gpui_kit::component::menu::{DropdownMenu, PopupMenu, PopupMenuItem};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::tab::{Tab, TabBar};
use gpui_kit::component::{ActiveTheme, IconName, WindowExt};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;

use crate::components::button::{Button, ButtonStyle};
use crate::components::icon_button::IconButton;
use crate::project::{Project, ProjectEvent};
use crate::query_execution::QueryRun;
use crate::sql_statements::{all_statements, statements_to_run};
use crate::theme::{
    EDITOR_FONT_FAMILY, EDITOR_FONT_SIZE, EDITOR_FONT_WEIGHT, EDITOR_LINE_HEIGHT, LABEL_SIZE_SMALL,
    PANEL_HEADER_HEIGHT, editor_background, editor_font_features,
};
use crate::util::ResultExt;

actions!(
    editor_panel,
    [
        /// Opens a new query tab.
        NewQuery,
        /// Closes the active query tab.
        CloseTab,
        /// Saves the active query tab.
        SaveFile,
        /// Runs the selected statements, or the statement under the cursor.
        RunStatement,
        /// Runs every statement in the active query tab.
        RunAll,
        /// Cancels the statement running in the active tab.
        CancelExecution,
    ]
);

const KEY_CONTEXT: &str = "EditorPanel";
const STATUS_DOT_SIZE: Pixels = px(6.);
const TAB_SLOT_SIZE: Pixels = px(18.);

// `KeyBinding::new` cannot parse the `secondary` modifier, which needs the
// platform to resolve.
#[cfg(target_os = "macos")]
const KEYS: [&str; 5] = ["cmd-n", "cmd-w", "cmd-s", "cmd-enter", "cmd-shift-enter"];
#[cfg(not(target_os = "macos"))]
const KEYS: [&str; 5] = [
    "ctrl-n",
    "ctrl-w",
    "ctrl-s",
    "ctrl-enter",
    "ctrl-shift-enter",
];

pub fn init(cx: &mut App) {
    let [new_query, close_tab, save_file, run_statement, run_all] = KEYS;
    cx.bind_keys([
        KeyBinding::new(new_query, NewQuery, Some(KEY_CONTEXT)),
        KeyBinding::new(close_tab, CloseTab, Some(KEY_CONTEXT)),
        KeyBinding::new(save_file, SaveFile, Some(KEY_CONTEXT)),
        // The editor's own `Enter` binding for this key lets the event go on,
        // so this binding still runs while the editor has focus.
        KeyBinding::new(run_statement, RunStatement, Some(KEY_CONTEXT)),
        KeyBinding::new(run_all, RunAll, Some(KEY_CONTEXT)),
    ]);
}

pub struct ConsoleRun {
    pub console: TabId,
    pub console_name: SharedString,
    pub connection_name: SharedString,
    pub database: Option<SharedString>,
    pub run: Entity<QueryRun>,
}

pub enum EditorPanelEvent {
    RunStarted(ConsoleRun),
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct TabId(usize);

impl TabId {
    pub fn index(self) -> usize {
        self.0
    }
}

struct EditorTab {
    id: TabId,
    editor: Entity<EditorState>,
    title: SharedString,
    path: Option<PathBuf>,
    connection: Option<ConnectionId>,
    /// Bumped on every edit. A save records the version it wrote, so edits
    /// made while the file was being written keep the tab dirty.
    edit_version: u64,
    saved_version: u64,
    run: Option<Entity<QueryRun>>,
    save_task: Option<Task<()>>,
    _run_subscription: Option<Subscription>,
    _editor_subscription: Subscription,
}

impl EditorTab {
    fn is_dirty(&self) -> bool {
        self.edit_version != self.saved_version
    }

    fn is_running(&self, cx: &App) -> bool {
        self.run
            .as_ref()
            .is_some_and(|run| run.read(cx).is_running())
    }
}

pub struct EditorPanel {
    focus_handle: FocusHandle,
    project: Entity<Project>,
    tabs: Vec<EditorTab>,
    active_tab: Option<TabId>,
    hovered_tab: Option<TabId>,
    next_tab_id: usize,
    next_untitled_number: usize,
    open_tasks: HashMap<PathBuf, Task<()>>,
    _subscriptions: Vec<Subscription>,
}

impl EditorPanel {
    pub fn new(project: Entity<Project>, _window: &mut Window, cx: &mut Context<Self>) -> Self {
        let subscriptions = vec![cx.subscribe(&project, |_, _, event, cx| match event {
            ProjectEvent::ConnectionsChanged | ProjectEvent::LiveConnectionsChanged => cx.notify(),
            ProjectEvent::Opened | ProjectEvent::RecentProjectsChanged | ProjectEvent::Error(_) => {
            }
        })];
        Self {
            focus_handle: cx.focus_handle(),
            project,
            tabs: Vec::new(),
            active_tab: None,
            hovered_tab: None,
            next_tab_id: 0,
            next_untitled_number: 1,
            open_tasks: HashMap::new(),
            _subscriptions: subscriptions,
        }
    }

    pub fn new_query(
        &mut self,
        connection: Option<ConnectionId>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let title = format!("Query {}", self.next_untitled_number);
        self.next_untitled_number += 1;
        let connection = connection.or_else(|| {
            self.active()
                .and_then(|tab| tab.connection)
        });
        self.add_tab(title.into(), None, String::new(), connection, window, cx);
    }

    pub fn open_file(&mut self, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(tab) = self
            .tabs
            .iter()
            .find(|tab| tab.path.as_deref() == Some(path.as_path()))
        {
            let id = tab.id;
            self.activate_tab(id, window, cx);
            return;
        }
        if self
            .open_tasks
            .contains_key(&path)
        {
            return;
        }
        let task = cx.spawn_in(window, {
            let path = path.clone();
            async move |this, cx| {
                let result = cx
                    .background_spawn({
                        let path = path.clone();
                        async move { std::fs::read_to_string(&path) }
                    })
                    .await;
                this.update_in(cx, |this, window, cx| {
                    this.open_tasks.remove(&path);
                    match result {
                        Ok(text) => {
                            let title = file_title(&path);
                            let connection = this
                                .active()
                                .and_then(|tab| tab.connection);
                            this.add_tab(title, Some(path), text, connection, window, cx);
                        }
                        Err(error) => window.push_notification(
                            Notification::error(format!(
                                "Could not open {}: {error}",
                                path.display()
                            )),
                            cx,
                        ),
                    }
                })
                .log_err();
            }
        });
        self.open_tasks
            .insert(path, task);
    }

    fn add_tab(
        &mut self,
        title: SharedString,
        path: Option<PathBuf>,
        text: String,
        connection: Option<ConnectionId>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let id = TabId(self.next_tab_id);
        self.next_tab_id += 1;
        let editor = cx.new(|cx| {
            EditorState::new(window, cx)
                .language("sql")
                .default_value(text)
        });
        let editor_subscription = cx.subscribe(&editor, move |this, _, event, cx| {
            if let InputEvent::Change = event
                && let Some(tab) = this.tab_mut(id)
            {
                let was_dirty = tab.is_dirty();
                tab.edit_version += 1;
                if !was_dirty {
                    cx.notify();
                }
            }
        });
        self.tabs.push(EditorTab {
            id,
            editor,
            title,
            path,
            connection,
            edit_version: 0,
            saved_version: 0,
            run: None,
            save_task: None,
            _run_subscription: None,
            _editor_subscription: editor_subscription,
        });
        self.activate_tab(id, window, cx);
    }

    fn active(&self) -> Option<&EditorTab> {
        let id = self.active_tab?;
        self.tab(id)
    }

    fn tab(&self, id: TabId) -> Option<&EditorTab> {
        self.tabs
            .iter()
            .find(|tab| tab.id == id)
    }

    fn tab_mut(&mut self, id: TabId) -> Option<&mut EditorTab> {
        self.tabs
            .iter_mut()
            .find(|tab| tab.id == id)
    }

    fn activate_tab(&mut self, id: TabId, window: &mut Window, cx: &mut Context<Self>) {
        let Some(tab) = self.tab(id) else {
            return;
        };
        tab.editor
            .update(cx, |editor, cx| editor.focus(window, cx));
        self.active_tab = Some(id);
        cx.notify();
    }

    fn close_tab(&mut self, id: TabId, window: &mut Window, cx: &mut Context<Self>) {
        let Some(tab) = self.tab(id) else {
            return;
        };
        if !tab.is_dirty() {
            self.remove_tab(id, window, cx);
            return;
        }
        let description = format!("\"{}\" has unsaved changes that will be lost.", tab.title);
        let panel = cx.weak_entity();
        window.open_alert_dialog(cx, move |alert, _, _| {
            let panel = panel.clone();
            alert
                .title("Close without saving?")
                .description(description.clone())
                .show_cancel(true)
                .ok_text("Discard")
                .ok_variant(ButtonVariant::Danger)
                .on_ok(move |_, window, cx| {
                    panel
                        .update(cx, |this, cx| this.remove_tab(id, window, cx))
                        .log_err();
                    true
                })
        });
    }

    fn remove_tab(&mut self, id: TabId, window: &mut Window, cx: &mut Context<Self>) {
        let Some(index) = self
            .tabs
            .iter()
            .position(|tab| tab.id == id)
        else {
            return;
        };
        let tab = self.tabs.remove(index);
        if let Some(run) = &tab.run {
            run.update(cx, |run, cx| run.cancel(cx));
        }
        if self.hovered_tab == Some(id) {
            self.hovered_tab = None;
        }
        if self.active_tab == Some(id) {
            self.active_tab = None;
            let neighbour = self
                .tabs
                .get(index)
                .or_else(|| self.tabs.last())
                .map(|tab| tab.id);
            match neighbour {
                Some(neighbour) => self.activate_tab(neighbour, window, cx),
                None => window.focus(&self.focus_handle, cx),
            }
        }
        cx.notify();
    }

    fn set_connection(&mut self, id: TabId, connection: ConnectionId, cx: &mut Context<Self>) {
        if let Some(tab) = self.tab_mut(id) {
            tab.connection = Some(connection);
            cx.notify();
        }
    }

    fn save(&mut self, id: TabId, window: &mut Window, cx: &mut Context<Self>) {
        let Some(tab) = self.tab(id) else {
            return;
        };
        match tab.path.clone() {
            Some(path) => self.write_file(id, path, window, cx),
            None => {
                let directory = self
                    .project
                    .read(cx)
                    .root()
                    .map(Path::to_path_buf)
                    .or_else(|| std::env::current_dir().ok())
                    .unwrap_or_default();
                let suggested_name = format!(
                    "{}.sql",
                    tab.title
                        .to_lowercase()
                        .replace(' ', "_")
                );
                let prompt = cx.prompt_for_new_path(&directory, Some(&suggested_name));
                let task = cx.spawn_in(window, async move |this, cx| {
                    let path = match prompt.await {
                        Ok(Ok(Some(path))) => path,
                        Ok(Ok(None)) | Err(_) => return,
                        Ok(Err(error)) => {
                            this.update_in(cx, |_, window, cx| {
                                window.push_notification(
                                    Notification::error(format!("Could not save: {error:#}")),
                                    cx,
                                )
                            })
                            .log_err();
                            return;
                        }
                    };
                    this.update_in(cx, |this, window, cx| this.write_file(id, path, window, cx))
                        .log_err();
                });
                if let Some(tab) = self.tab_mut(id) {
                    tab.save_task = Some(task);
                }
            }
        }
    }

    fn write_file(
        &mut self,
        id: TabId,
        path: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(tab) = self.tab_mut(id) else {
            return;
        };
        let version = tab.edit_version;
        // Cloning the rope shares its storage, so the text is copied only on
        // the background thread as it is written.
        let text = tab
            .editor
            .read(cx)
            .text()
            .clone();
        let task = cx.spawn_in(window, async move |this, cx| {
            let result = cx
                .background_spawn({
                    let path = path.clone();
                    async move {
                        let mut writer = BufWriter::new(File::create(&path)?);
                        text.write_to(&mut writer)?;
                        writer.flush()
                    }
                })
                .await;
            this.update_in(cx, |this, window, cx| match result {
                Ok(()) => {
                    if let Some(tab) = this.tab_mut(id) {
                        tab.saved_version = version;
                        if tab.path.as_deref() != Some(path.as_path()) {
                            tab.title = file_title(&path);
                            tab.path = Some(path);
                        }
                        cx.notify();
                    }
                }
                Err(error) => window.push_notification(
                    Notification::error(format!("Could not save {}: {error}", path.display())),
                    cx,
                ),
            })
            .log_err();
        });
        tab.save_task = Some(task);
    }

    fn run(&mut self, whole_file: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(tab) = self.active() else {
            return;
        };
        let id = tab.id;
        if tab.is_running(cx) {
            show_error(
                "A statement is already running in this tab. Cancel it first.",
                window,
                cx,
            );
            return;
        }
        let Some(connection_id) = tab.connection else {
            show_error("Choose a connection for this tab first.", window, cx);
            return;
        };
        let project = self.project.read(cx);
        let Some(profile) = project
            .connections()
            .iter()
            .find(|profile| profile.id == connection_id)
        else {
            show_error(
                "The connection for this tab no longer exists. Choose another one.",
                window,
                cx,
            );
            return;
        };
        let connection_name = SharedString::from(profile.name.clone());
        let database = profile
            .values
            .get("database")
            .filter(|database| !database.is_empty())
            .map(|database| SharedString::from(database.clone()));
        let Some(connection) = project.live_connection(connection_id) else {
            show_error(
                format!(
                    "\"{connection_name}\" is not connected. Connect it in the Connections panel first."
                ),
                window,
                cx,
            );
            return;
        };
        let console_name = tab.title.clone();
        let editor = tab.editor.read(cx);
        // Splitting is a single linear scan, and runs only when the user asks.
        let text = editor.value();
        let statements: Vec<SharedString> = if whole_file {
            all_statements(&text)
        } else {
            statements_to_run(&text, editor.selected_range())
        }
        .into_iter()
        .map(|statement| SharedString::from(statement.to_string()))
        .collect();
        if statements.is_empty() {
            show_error("There is no statement to run.", window, cx);
            return;
        }

        let run = cx.new(|cx| QueryRun::new(connection, statements, cx));
        let subscription = cx.observe(&run, |_, _, cx| cx.notify());
        if let Some(tab) = self.tab_mut(id) {
            tab.run = Some(run.clone());
            tab._run_subscription = Some(subscription);
        }
        cx.emit(EditorPanelEvent::RunStarted(ConsoleRun {
            console: id,
            console_name,
            connection_name,
            database,
            run,
        }));
        cx.notify();
    }

    fn cancel_execution(&mut self, cx: &mut Context<Self>) {
        if let Some(run) = self
            .active()
            .and_then(|tab| tab.run.clone())
        {
            run.update(cx, |run, cx| run.cancel(cx));
        }
    }

    fn on_new_query(&mut self, _: &NewQuery, window: &mut Window, cx: &mut Context<Self>) {
        self.new_query(None, window, cx);
    }

    fn on_close_tab(&mut self, _: &CloseTab, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(id) = self.active_tab {
            self.close_tab(id, window, cx);
        }
    }

    fn on_save(&mut self, _: &SaveFile, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(id) = self.active_tab {
            self.save(id, window, cx);
        }
    }

    fn on_run(&mut self, _: &RunStatement, window: &mut Window, cx: &mut Context<Self>) {
        self.run(false, window, cx);
    }

    fn on_run_all(&mut self, _: &RunAll, window: &mut Window, cx: &mut Context<Self>) {
        self.run(true, window, cx);
    }

    fn on_cancel(&mut self, _: &CancelExecution, _: &mut Window, cx: &mut Context<Self>) {
        self.cancel_execution(cx);
    }
}

fn file_title(path: &Path) -> SharedString {
    path.file_name()
        .unwrap_or(path.as_os_str())
        .to_string_lossy()
        .into_owned()
        .into()
}

fn show_error(message: impl Into<SharedString>, window: &mut Window, cx: &mut App) {
    window.push_notification(Notification::error(message.into()), cx);
}

impl EventEmitter<PanelEvent> for EditorPanel {}
impl EventEmitter<EditorPanelEvent> for EditorPanel {}

impl Focusable for EditorPanel {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl BasePanel for EditorPanel {
    fn panel_name(&self) -> &'static str {
        "EditorPanel"
    }
}

impl Panel for EditorPanel {
    fn title_bar(&self, _: &App) -> bool {
        false
    }
}

impl Render for EditorPanel {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .size_full()
            .key_context(KEY_CONTEXT)
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(Self::on_new_query))
            .on_action(cx.listener(Self::on_close_tab))
            .on_action(cx.listener(Self::on_save))
            .on_action(cx.listener(Self::on_run))
            .on_action(cx.listener(Self::on_run_all))
            .on_action(cx.listener(Self::on_cancel))
            .when_some(self.active(), |this, tab| {
                this.child(self.render_tab_bar(cx))
                    .child(self.render_toolbar(tab, cx))
                    .child(
                        div()
                            .flex_1()
                            .min_h_0()
                            .bg(editor_background(cx))
                            .child(
                                Editor::new(&tab.editor)
                                    .appearance(false)
                                    .bordered(false)
                                    .font_family(EDITOR_FONT_FAMILY)
                                    .text_size(EDITOR_FONT_SIZE)
                                    .font_weight(EDITOR_FONT_WEIGHT)
                                    .line_height(relative(EDITOR_LINE_HEIGHT))
                                    .font_features(editor_font_features())
                                    .size_full(),
                            ),
                    )
            })
    }
}

impl EditorPanel {
    fn render_tab_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let panel = cx.weak_entity();
        let selected_index = self
            .active_tab
            .and_then(|id| {
                self.tabs
                    .iter()
                    .position(|tab| tab.id == id)
            });
        TabBar::new("editor-tabs")
            .children(
                self.tabs
                    .iter()
                    .map(|tab| self.render_tab(tab, &panel, cx)),
            )
            .when_some(selected_index, |this, index| this.selected_index(index))
    }

    fn render_tab(&self, tab: &EditorTab, panel: &WeakEntity<Self>, cx: &App) -> Tab {
        let id = tab.id;
        let show_dirty_dot = tab.is_dirty() && self.hovered_tab != Some(id);
        let slot = div()
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .size(TAB_SLOT_SIZE)
            .map(|this| {
                if show_dirty_dot {
                    this.child(
                        div()
                            .size(STATUS_DOT_SIZE)
                            .rounded_full()
                            .bg(cx.theme().foreground),
                    )
                } else {
                    let panel = panel.clone();
                    this.child(
                        IconButton::new(("close-tab", id.0), IconName::Close)
                            .small()
                            .tooltip("Close")
                            .on_click(move |_, window, cx| {
                                cx.stop_propagation();
                                panel
                                    .update(cx, |this, cx| this.close_tab(id, window, cx))
                                    .log_err();
                            }),
                    )
                }
            });
        let click_panel = panel.clone();
        let hover_panel = panel.clone();
        Tab::new()
            .child(tab.title.clone())
            .suffix(slot.mr_1())
            .on_click(move |_, window, cx| {
                click_panel
                    .update(cx, |this, cx| this.activate_tab(id, window, cx))
                    .log_err();
            })
            .on_hover(move |hovered, _, cx| {
                hover_panel
                    .update(cx, |this, cx| {
                        let hovered_tab = if *hovered {
                            Some(id)
                        } else if this.hovered_tab == Some(id) {
                            None
                        } else {
                            this.hovered_tab
                        };
                        if hovered_tab != this.hovered_tab {
                            this.hovered_tab = hovered_tab;
                            cx.notify();
                        }
                    })
                    .log_err();
            })
    }

    fn render_toolbar(&self, tab: &EditorTab, cx: &mut Context<Self>) -> impl IntoElement {
        let panel = cx.weak_entity();
        let project = self.project.read(cx);
        let theme = cx.theme();
        let selected = tab
            .connection
            .and_then(|connection| {
                project
                    .connections()
                    .iter()
                    .find(|profile| profile.id == connection)
                    .map(|profile| {
                        (
                            SharedString::from(profile.name.clone()),
                            project
                                .live_connection(connection)
                                .is_some(),
                        )
                    })
            });
        let (connection_label, connection_live) = match selected {
            Some((name, live)) => (name, live),
            None => (SharedString::from("No connection"), false),
        };
        let choices: Vec<(ConnectionId, SharedString, bool)> = project
            .connections()
            .iter()
            .map(|profile| {
                (
                    profile.id,
                    SharedString::from(profile.name.clone()),
                    project
                        .live_connection(profile.id)
                        .is_some(),
                )
            })
            .collect();
        let tab_id = tab.id;
        let current = tab.connection;
        let live_color = theme.success;
        let idle_color = theme.muted_foreground;

        let run_buttons = if tab.is_running(cx) {
            h_flex().child(
                IconButton::new("cancel-statement", IconName::Square)
                    .small()
                    .tooltip("Cancel")
                    .on_click(cx.listener(|this, _, _, cx| this.cancel_execution(cx))),
            )
        } else {
            h_flex()
                .gap_1()
                .child(
                    IconButton::new("run-statement", IconName::Play)
                        .small()
                        .tooltip("Run Statement (Ctrl+Enter)")
                        .on_click(cx.listener(|this, _, window, cx| this.run(false, window, cx))),
                )
                .child(
                    IconButton::new("run-all", AssetIconName::ListVideo)
                        .small()
                        .tooltip("Run All (Ctrl+Shift+Enter)")
                        .on_click(cx.listener(|this, _, window, cx| this.run(true, window, cx))),
                )
        };

        h_flex()
            .flex_shrink_0()
            .w_full()
            .h(PANEL_HEADER_HEIGHT)
            .bg(editor_background(cx))
            .items_center()
            .justify_between()
            .px_2()
            .border_b_1()
            .border_color(theme.border)
            .child(run_buttons)
            .child(
                Button::new("connection-selector")
                    .label(connection_label)
                    .label_size(LABEL_SIZE_SMALL)
                    .style(ButtonStyle::Subtle)
                    .prefix(status_dot(if connection_live {
                        live_color
                    } else {
                        idle_color
                    }))
                    .dropdown_menu(move |menu: PopupMenu, _, _| {
                        if choices.is_empty() {
                            return menu.item(PopupMenuItem::label("No connections"));
                        }
                        choices
                            .iter()
                            .fold(menu, |menu, (connection, name, live)| {
                                let connection = *connection;
                                let name = name.clone();
                                let color = if *live { live_color } else { idle_color };
                                let panel = panel.clone();
                                menu.item(
                                    PopupMenuItem::element(move |_, _| {
                                        h_flex()
                                            .gap_2()
                                            .items_center()
                                            .child(status_dot(color))
                                            .child(name.clone())
                                    })
                                    .checked(current == Some(connection))
                                    .on_click(move |_, _, cx| {
                                        panel
                                            .update(cx, |this, cx| {
                                                this.set_connection(tab_id, connection, cx)
                                            })
                                            .log_err();
                                    }),
                                )
                            })
                    }),
            )
    }
}

fn status_dot(color: Hsla) -> impl IntoElement {
    div()
        .flex_none()
        .size(STATUS_DOT_SIZE)
        .rounded_full()
        .bg(color)
}
