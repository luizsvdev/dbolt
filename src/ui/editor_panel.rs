use std::collections::HashMap;
use std::fs::File;
use std::io::{BufWriter, Write};
use std::ops::Range;
use std::path::{Path, PathBuf};

use dbolt_database::ConnectionId;
use gpui_kit::assets::IconName as AssetIconName;
use gpui_kit::base::Selectable;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::button::ButtonVariant;
use gpui_kit::component::dock::{BasePanel, DockArea, Panel, PanelEvent, ToggleZoom};
use gpui_kit::component::input::{Editor, EditorState, InputEvent};
use gpui_kit::component::menu::{PopupMenu, PopupMenuItem};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::popover::Popover;
use gpui_kit::component::{ActiveTheme, Icon, IconName, WindowExt};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;

use crate::components::button::{Button, ButtonStyle};
use crate::components::icon_button::IconButton;
use crate::components::styled_editor::StyledEditor;
use crate::line_edits::{self, LineEdit};
use crate::project::{Project, ProjectEvent};
use crate::query_execution::QueryRun;
use crate::sql_statements::{all_statements, statements_to_run};
use crate::theme::{
    EDITOR_FONT_FAMILY, EDITOR_FONT_SIZE, EDITOR_FONT_WEIGHT, EDITOR_LINE_HEIGHT, ICON_SIZE_XSMALL,
    LABEL_SIZE_SMALL, PANEL_HEADER_HEIGHT, editor_background, editor_font_features,
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
        /// Comments or uncomments the selected lines.
        ToggleComment,
        /// Moves the selected lines up.
        MoveLinesUp,
        /// Moves the selected lines down.
        MoveLinesDown,
        /// Duplicates the selected lines.
        DuplicateLines,
        /// Deletes the selected lines.
        DeleteLines,
        /// Goes back to the previously active query tab.
        GoBack,
        /// Goes forward again after going back.
        GoForward,
    ]
);

const KEY_CONTEXT: &str = "EditorPanel";
const STATUS_DOT_SIZE: Pixels = px(6.);
const TAB_BAR_HEIGHT: Pixels = px(32.);
const TAB_START_SLOT_SIZE: Pixels = px(12.);
const TAB_END_SLOT_SIZE: Pixels = px(14.);
const TAB_CONTENT_GAP: Pixels = px(4.);
const TAB_GROUP: &str = "editor-tab";

// `KeyBinding::new` cannot parse the `secondary` modifier, which needs the
// platform to resolve.
#[cfg(target_os = "macos")]
const KEYS: [&str; 9] = [
    "cmd-n",
    "cmd-w",
    "cmd-s",
    "cmd-enter",
    "cmd-shift-enter",
    "cmd-/",
    "cmd-d",
    "cmd-shift-k",
    "alt-up",
];
#[cfg(not(target_os = "macos"))]
const KEYS: [&str; 9] = [
    "ctrl-n",
    "ctrl-w",
    "ctrl-s",
    "ctrl-enter",
    "ctrl-shift-enter",
    "ctrl-/",
    "ctrl-d",
    "ctrl-shift-k",
    "alt-up",
];

#[cfg(target_os = "macos")]
const NAVIGATION_KEYS: [&str; 2] = ["ctrl--", "ctrl-_"];
#[cfg(not(target_os = "macos"))]
const NAVIGATION_KEYS: [&str; 2] = ["ctrl-alt--", "ctrl-alt-_"];

const TAB_BAR_SECTION_GAP: Pixels = px(4.);
const TAB_BAR_SECTION_PADDING: Pixels = px(6.);

pub fn init(cx: &mut App) {
    let [
        new_query,
        close_tab,
        save_file,
        run_statement,
        run_all,
        toggle_comment,
        duplicate_lines,
        delete_lines,
        move_lines_up,
    ] = KEYS;
    cx.bind_keys([
        KeyBinding::new(new_query, NewQuery, Some(KEY_CONTEXT)),
        KeyBinding::new(close_tab, CloseTab, Some(KEY_CONTEXT)),
        KeyBinding::new(save_file, SaveFile, Some(KEY_CONTEXT)),
        // The editor's own `Enter` binding for this key lets the event go on,
        // so this binding still runs while the editor has focus.
        KeyBinding::new(run_statement, RunStatement, Some(KEY_CONTEXT)),
        KeyBinding::new(run_all, RunAll, Some(KEY_CONTEXT)),
        KeyBinding::new(toggle_comment, ToggleComment, Some(KEY_CONTEXT)),
        KeyBinding::new(duplicate_lines, DuplicateLines, Some(KEY_CONTEXT)),
        KeyBinding::new(delete_lines, DeleteLines, Some(KEY_CONTEXT)),
        KeyBinding::new(move_lines_up, MoveLinesUp, Some(KEY_CONTEXT)),
        KeyBinding::new("alt-down", MoveLinesDown, Some(KEY_CONTEXT)),
        KeyBinding::new(NAVIGATION_KEYS[0], GoBack, Some(KEY_CONTEXT)),
        KeyBinding::new(NAVIGATION_KEYS[1], GoForward, Some(KEY_CONTEXT)),
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
    /// A run needs this connection, which is not connected yet.
    ConnectRequested(ConnectionId),
}

/// A run waiting for its connection to connect.
struct PendingRun {
    tab: TabId,
    connection: ConnectionId,
    whole_file: bool,
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

struct ConnectionMenu {
    tab: TabId,
    menu: Entity<PopupMenu>,
    /// Set when the menu opened because a run had no connection; picking one
    /// then runs it (`true` for the whole file).
    run_after_pick: Option<bool>,
    _dismiss: Subscription,
}

pub struct EditorPanel {
    focus_handle: FocusHandle,
    project: Entity<Project>,
    tabs: Vec<EditorTab>,
    active_tab: Option<TabId>,
    next_tab_id: usize,
    next_untitled_number: usize,
    open_tasks: HashMap<PathBuf, Task<()>>,
    connection_menu: Option<ConnectionMenu>,
    pending_run: Option<PendingRun>,
    /// Tabs that were active before, most recent last.
    nav_back: Vec<TabId>,
    nav_forward: Vec<TabId>,
    dock_area: WeakEntity<DockArea>,
    _subscriptions: Vec<Subscription>,
}

#[derive(Clone)]
struct DraggedEditorTab {
    id: TabId,
    index: usize,
    title: SharedString,
}

impl Render for DraggedEditorTab {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        h_flex()
            .h(TAB_BAR_HEIGHT)
            .px(TAB_CONTENT_GAP)
            .gap(TAB_CONTENT_GAP)
            .bg(theme.tab_active)
            .border_1()
            .border_color(theme.border)
            .text_color(theme.tab_active_foreground)
            .child(div().size(TAB_START_SLOT_SIZE))
            .child(self.title.clone())
            .child(div().size(TAB_END_SLOT_SIZE))
    }
}

impl EditorPanel {
    pub fn new(
        project: Entity<Project>,
        dock_area: &Entity<DockArea>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let subscriptions = vec![
            cx.subscribe(&project, |_, _, event, cx| match event {
                ProjectEvent::ConnectionsChanged | ProjectEvent::LiveConnectionsChanged => {
                    cx.notify()
                }
                ProjectEvent::Opened
                | ProjectEvent::RecentProjectsChanged
                | ProjectEvent::Error(_) => {}
            }),
            // Redraws the zoom button when the dock zooms in or out.
            cx.observe(dock_area, |_, _, cx| cx.notify()),
        ];
        Self {
            focus_handle: cx.focus_handle(),
            project,
            tabs: Vec::new(),
            active_tab: None,
            next_tab_id: 0,
            next_untitled_number: 1,
            open_tasks: HashMap::new(),
            connection_menu: None,
            pending_run: None,
            nav_back: Vec::new(),
            nav_forward: Vec::new(),
            dock_area: dock_area.downgrade(),
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
        if let Some(previous) = self
            .active_tab
            .filter(|previous| *previous != id)
        {
            self.nav_back.push(previous);
            self.nav_forward.clear();
        }
        self.show_tab(id, window, cx);
    }

    fn go_back(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(id) = self.nav_back.pop() {
            if let Some(current) = self.active_tab {
                self.nav_forward.push(current);
            }
            self.show_tab(id, window, cx);
        }
    }

    fn go_forward(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(id) = self.nav_forward.pop() {
            if let Some(current) = self.active_tab {
                self.nav_back.push(current);
            }
            self.show_tab(id, window, cx);
        }
    }

    /// Moves a tab to `index`, which is where it was dropped: the tabs from
    /// there on shift right.
    fn move_tab(&mut self, id: TabId, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(from) = self
            .tabs
            .iter()
            .position(|tab| tab.id == id)
        else {
            return;
        };
        let tab = self.tabs.remove(from);
        let to = index.min(self.tabs.len());
        self.tabs.insert(to, tab);
        self.activate_tab(id, window, cx);
    }

    fn toggle_zoom(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        // The dock's handler reads this panel, so the action must not be
        // dispatched while the panel is being updated.
        let focus_handle = self.focus_handle.clone();
        window.defer(cx, move |window, cx| {
            focus_handle.dispatch_action(&ToggleZoom, window, cx)
        });
    }

    fn is_zoomed(&self, cx: &App) -> bool {
        self.dock_area
            .upgrade()
            .is_some_and(|area| area.read(cx).is_zoomed())
    }

    fn show_tab(&mut self, id: TabId, window: &mut Window, cx: &mut Context<Self>) {
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
        self.nav_back
            .retain(|previous| *previous != id);
        self.nav_forward
            .retain(|next| *next != id);
        if let Some(run) = &tab.run {
            run.update(cx, |run, cx| run.cancel(cx));
        }
        if self.active_tab == Some(id) {
            self.active_tab = None;
            let neighbour = self
                .tabs
                .get(index)
                .or_else(|| self.tabs.last())
                .map(|tab| tab.id);
            match neighbour {
                Some(neighbour) => self.show_tab(neighbour, window, cx),
                None => window.focus(&self.focus_handle, cx),
            }
        }
        cx.notify();
    }

    /// Runs what was waiting for `connection`, or drops it when connecting
    /// failed (the connections panel reports the error).
    pub fn connection_finished(
        &mut self,
        connection: ConnectionId,
        connected: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(pending) = self
            .pending_run
            .take_if(|pending| pending.connection == connection)
        else {
            return;
        };
        if connected
            && self
                .tab(pending.tab)
                .is_some_and(|tab| tab.connection == Some(connection))
        {
            self.run_tab(pending.tab, pending.whole_file, window, cx);
        }
    }

    fn open_connection_menu(
        &mut self,
        run_after_pick: Option<bool>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(tab) = self.active() else {
            return;
        };
        let tab_id = tab.id;
        let current = tab.connection;
        let project = self.project.read(cx);
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
        let live_color = cx.theme().success;
        let idle_color = cx.theme().muted_foreground;
        let panel = cx.weak_entity();
        let menu = PopupMenu::build(window, cx, move |menu, _, _| {
            if choices.is_empty() {
                return menu.item(PopupMenuItem::label("No connections"));
            }
            choices
                .into_iter()
                .fold(menu, |menu, (connection, name, live)| {
                    let color = if live { live_color } else { idle_color };
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
                        .on_click(move |_, window, cx| {
                            panel
                                .update(cx, |this, cx| {
                                    this.pick_connection(tab_id, connection, window, cx)
                                })
                                .log_err();
                        }),
                    )
                })
        });
        menu.focus_handle(cx)
            .focus(window, cx);
        let dismiss = cx.subscribe_in(&menu, window, |this, _, _: &DismissEvent, window, cx| {
            this.close_connection_menu(window, cx)
        });
        self.connection_menu = Some(ConnectionMenu {
            tab: tab_id,
            menu,
            run_after_pick,
            _dismiss: dismiss,
        });
        cx.notify();
    }

    fn close_connection_menu(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self
            .connection_menu
            .take()
            .is_none()
        {
            return;
        }
        if let Some(tab) = self.active() {
            tab.editor
                .update(cx, |editor, cx| editor.focus(window, cx));
        }
        cx.notify();
    }

    fn pick_connection(
        &mut self,
        tab_id: TabId,
        connection: ConnectionId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let run_after_pick = self
            .connection_menu
            .as_ref()
            .filter(|menu| menu.tab == tab_id)
            .and_then(|menu| menu.run_after_pick);
        self.set_connection(tab_id, connection, cx);
        self.close_connection_menu(window, cx);
        if let Some(whole_file) = run_after_pick
            && self.active_tab == Some(tab_id)
        {
            self.run(whole_file, window, cx);
        }
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
        if let Some(id) = self.active_tab {
            self.run_tab(id, whole_file, window, cx);
        }
    }

    fn run_tab(
        &mut self,
        id: TabId,
        whole_file: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(tab) = self.tab(id) else {
            return;
        };
        if tab.is_running(cx) {
            show_error(
                "A statement is already running in this tab. Cancel it first.",
                window,
                cx,
            );
            return;
        }
        let Some(connection_id) = tab.connection else {
            self.open_connection_menu(Some(whole_file), window, cx);
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
            self.pending_run = Some(PendingRun {
                tab: id,
                connection: connection_id,
                whole_file,
            });
            cx.emit(EditorPanelEvent::ConnectRequested(connection_id));
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

    fn on_go_back(&mut self, _: &GoBack, window: &mut Window, cx: &mut Context<Self>) {
        self.go_back(window, cx);
    }

    fn on_go_forward(&mut self, _: &GoForward, window: &mut Window, cx: &mut Context<Self>) {
        self.go_forward(window, cx);
    }

    fn edit_lines(
        &mut self,
        edit: fn(&str, Range<usize>) -> Option<LineEdit>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(tab) = self.active() else {
            return;
        };
        let editor = tab.editor.clone();
        editor.update(cx, |editor, cx| {
            let text = editor.value();
            let Some(edit) = edit(&text, editor.selected_range()) else {
                return;
            };
            editor.set_selected_range(edit.replace, cx);
            editor.replace(edit.text, window, cx);
            editor.set_selected_range(edit.selection, cx);
        });
    }

    fn on_toggle_comment(
        &mut self,
        _: &ToggleComment,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.edit_lines(line_edits::toggle_comment, window, cx);
    }

    fn on_move_lines_up(&mut self, _: &MoveLinesUp, window: &mut Window, cx: &mut Context<Self>) {
        self.edit_lines(line_edits::move_lines_up, window, cx);
    }

    fn on_move_lines_down(
        &mut self,
        _: &MoveLinesDown,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.edit_lines(line_edits::move_lines_down, window, cx);
    }

    fn on_duplicate_lines(
        &mut self,
        _: &DuplicateLines,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.edit_lines(line_edits::duplicate_lines, window, cx);
    }

    fn on_delete_lines(&mut self, _: &DeleteLines, window: &mut Window, cx: &mut Context<Self>) {
        self.edit_lines(line_edits::delete_lines, window, cx);
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
            .on_action(cx.listener(Self::on_go_back))
            .on_action(cx.listener(Self::on_go_forward))
            .on_action(cx.listener(Self::on_toggle_comment))
            .on_action(cx.listener(Self::on_move_lines_up))
            .on_action(cx.listener(Self::on_move_lines_down))
            .on_action(cx.listener(Self::on_duplicate_lines))
            .on_action(cx.listener(Self::on_delete_lines))
            .on_action(cx.listener(Self::on_cancel))
            .when_some(self.active(), |this, tab| {
                this.child(self.render_tab_bar(cx))
                    .child(self.render_toolbar(tab, cx))
                    .child(
                        div()
                            .flex_1()
                            .min_h_0()
                            .bg(editor_background(cx))
                            .child(StyledEditor::new(
                                &tab.editor,
                                Editor::new(&tab.editor)
                                    .appearance(false)
                                    .bordered(false)
                                    .font_family(EDITOR_FONT_FAMILY)
                                    .text_size(EDITOR_FONT_SIZE)
                                    .font_weight(EDITOR_FONT_WEIGHT)
                                    .line_height(relative(EDITOR_LINE_HEIGHT))
                                    .font_features(editor_font_features())
                                    .size_full(),
                            )),
                    )
            })
    }
}

impl EditorPanel {
    fn render_tab_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let panel = cx.weak_entity();
        let border = cx.theme().border;
        let tab_bar_background = cx.theme().tab_bar;
        let selected_index = self
            .active_tab
            .and_then(|id| {
                self.tabs
                    .iter()
                    .position(|tab| tab.id == id)
            });
        let zoomed = self.is_zoomed(cx);
        let tab_count = self.tabs.len();
        let navigation = h_flex()
            .flex_none()
            .h_full()
            .gap(TAB_BAR_SECTION_GAP)
            .px(TAB_BAR_SECTION_PADDING)
            .border_b_1()
            .border_r_1()
            .border_color(border)
            .child(
                IconButton::new("navigate-backward", IconName::ArrowLeft)
                    .small()
                    .tooltip("Go Back")
                    .disabled(self.nav_back.is_empty())
                    .on_click(cx.listener(|this, _, window, cx| this.go_back(window, cx))),
            )
            .child(
                IconButton::new("navigate-forward", IconName::ArrowRight)
                    .small()
                    .tooltip("Go Forward")
                    .disabled(self.nav_forward.is_empty())
                    .on_click(cx.listener(|this, _, window, cx| this.go_forward(window, cx))),
            );
        let zoom = h_flex()
            .flex_none()
            .h_full()
            .gap(TAB_BAR_SECTION_GAP)
            .px(TAB_BAR_SECTION_PADDING)
            .border_b_1()
            .border_l_1()
            .border_color(border)
            .child(
                IconButton::new(
                    "toggle-zoom",
                    if zoomed {
                        IconName::Minimize
                    } else {
                        IconName::Maximize
                    },
                )
                .small()
                .selected(zoomed)
                .tooltip(if zoomed { "Zoom Out" } else { "Zoom In" })
                .on_click(cx.listener(|this, _, window, cx| this.toggle_zoom(window, cx))),
            );
        let drop_panel = panel.clone();
        let end_drop_target = div()
            .id("editor-tab-bar-drop-target")
            .min_w_6()
            .flex_1()
            .h_full()
            .drag_over::<DraggedEditorTab>(|style, _, _, cx| style.bg(cx.theme().drop_target))
            .on_drop(move |dragged: &DraggedEditorTab, window, cx| {
                drop_panel
                    .update(cx, |this, cx| {
                        this.move_tab(dragged.id, tab_count, window, cx)
                    })
                    .log_err();
            });
        // The bottom border runs under every tab; the active tab's background
        // covers it so that tab joins the toolbar below.
        let tabs = div()
            .relative()
            .flex_1()
            .h_full()
            .overflow_x_hidden()
            .child(
                div()
                    .absolute()
                    .top_0()
                    .left_0()
                    .size_full()
                    .border_b_1()
                    .border_color(border),
            )
            .child(
                h_flex()
                    .id("editor-tabs")
                    .size_full()
                    .overflow_x_scroll()
                    .children(
                        self.tabs
                            .iter()
                            .enumerate()
                            .map(|(index, tab)| {
                                self.render_tab(tab, index, selected_index, &panel, cx)
                            }),
                    )
                    .child(end_drop_target),
            );
        h_flex()
            .id("editor-tab-bar")
            .flex_none()
            .w_full()
            .h(TAB_BAR_HEIGHT)
            .bg(tab_bar_background)
            .child(navigation)
            .child(tabs)
            .child(zoom)
    }

    /// Borders follow the tab's place relative to the active tab, and a 1px
    /// padding stands in for each missing border so the content never moves.
    fn render_tab(
        &self,
        tab: &EditorTab,
        index: usize,
        active_index: Option<usize>,
        panel: &WeakEntity<Self>,
        cx: &App,
    ) -> impl IntoElement {
        let theme = cx.theme();
        let id = tab.id;
        let is_active = active_index == Some(index);
        let is_last = index + 1 == self.tabs.len();
        let (text_color, background) = if is_active {
            (theme.tab_active_foreground, theme.tab_active)
        } else {
            (theme.tab_foreground, theme.tab)
        };
        let close_panel = panel.clone();
        let click_panel = panel.clone();
        let drop_panel = panel.clone();
        let dragged = DraggedEditorTab {
            id,
            index,
            title: tab.title.clone(),
        };
        let dirty_color = theme.foreground;

        h_flex()
            .id(("editor-tab", id.0))
            .group(TAB_GROUP)
            .flex_none()
            .h(TAB_BAR_HEIGHT)
            .bg(background)
            .border_color(theme.border)
            .map(|this| match (index, is_last, is_active) {
                (0, _, true) => this
                    .pl(px(1.))
                    .border_r_1()
                    .pb(px(1.)),
                (0, _, false) => this
                    .pl(px(1.))
                    .pr(px(1.))
                    .border_b_1(),
                (_, true, true) => this
                    .border_l_1()
                    .border_r_1()
                    .pb(px(1.)),
                (_, true, false) => this
                    .pl(px(1.))
                    .border_b_1()
                    .border_r_1(),
                (_, false, true) => this
                    .border_l_1()
                    .border_r_1()
                    .pb(px(1.)),
                _ if active_index.is_some_and(|active| index < active) => this
                    .border_l_1()
                    .pr(px(1.))
                    .border_b_1(),
                _ => this
                    .border_r_1()
                    .pl(px(1.))
                    .border_b_1(),
            })
            .cursor_pointer()
            .on_click(move |_, window, cx| {
                click_panel
                    .update(cx, |this, cx| this.activate_tab(id, window, cx))
                    .log_err();
            })
            .on_drag(dragged, |dragged, _, _, cx| cx.new(|_| dragged.clone()))
            .drag_over::<DraggedEditorTab>(move |style, dragged, _, cx| {
                let style = style
                    .bg(cx.theme().drop_target)
                    .border_color(cx.theme().drag_border)
                    .border_0();
                match index.cmp(&dragged.index) {
                    std::cmp::Ordering::Less => style.border_l_2(),
                    std::cmp::Ordering::Greater => style.border_r_2(),
                    std::cmp::Ordering::Equal => style,
                }
            })
            .on_drop(move |dragged: &DraggedEditorTab, window, cx| {
                drop_panel
                    .update(cx, |this, cx| this.move_tab(dragged.id, index, window, cx))
                    .log_err();
            })
            .child(
                h_flex()
                    .h(TAB_BAR_HEIGHT - px(1.))
                    .px(TAB_CONTENT_GAP)
                    .gap(TAB_CONTENT_GAP)
                    .text_color(text_color)
                    .child(
                        h_flex()
                            .size(TAB_START_SLOT_SIZE)
                            .flex_none()
                            .justify_center()
                            .when(tab.is_dirty(), |this| {
                                this.child(
                                    div()
                                        .size(STATUS_DOT_SIZE)
                                        .rounded_full()
                                        .bg(dirty_color),
                                )
                            }),
                    )
                    .child(tab.title.clone())
                    .child(
                        h_flex()
                            .size(TAB_END_SLOT_SIZE)
                            .flex_none()
                            .justify_center()
                            .child(
                                IconButton::new(("close-tab", id.0), IconName::Close)
                                    .small()
                                    .compact()
                                    .muted()
                                    .tooltip("Close Tab")
                                    .invisible()
                                    .group_hover(TAB_GROUP, |style| style.visible())
                                    .on_click(move |_, window, cx| {
                                        cx.stop_propagation();
                                        close_panel
                                            .update(cx, |this, cx| this.close_tab(id, window, cx))
                                            .log_err();
                                    }),
                            ),
                    ),
            )
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
        let live_color = theme.success;
        let idle_color = theme.muted_foreground;
        let open_menu = self
            .connection_menu
            .as_ref()
            .map(|menu| menu.menu.clone());
        let menu_focus = open_menu
            .as_ref()
            .map(|menu| menu.focus_handle(cx));

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
                Popover::new("connection-menu")
                    .appearance(false)
                    .overlay_closable(false)
                    .anchor(Anchor::TopRight)
                    .open(open_menu.is_some())
                    // The popover focuses a handle of its own when it opens;
                    // giving it the menu's keeps the arrow keys and Enter there.
                    .when_some(menu_focus, |this, focus| this.track_focus(&focus))
                    .on_open_change(move |open, window, cx| {
                        let open = *open;
                        panel
                            .update(cx, |this, cx| {
                                if open {
                                    this.open_connection_menu(None, window, cx);
                                } else {
                                    this.close_connection_menu(window, cx);
                                }
                            })
                            .log_err();
                    })
                    .trigger(
                        Button::new("connection-selector")
                            .label(connection_label)
                            .label_size(LABEL_SIZE_SMALL)
                            .style(ButtonStyle::Subtle)
                            .prefix(status_dot(if connection_live {
                                live_color
                            } else {
                                idle_color
                            }))
                            .suffix(
                                Icon::new(IconName::ChevronDown)
                                    .size(ICON_SIZE_XSMALL)
                                    .text_color(idle_color),
                            ),
                    )
                    .content(move |_, _, _| match &open_menu {
                        Some(menu) => menu
                            .clone()
                            .into_any_element(),
                        None => div().into_any_element(),
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
