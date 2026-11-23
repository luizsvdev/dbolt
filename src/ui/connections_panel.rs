use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::Arc;

use dbolt_database::{
    Connection, ConnectionId, ConnectionProfile, ConnectionScope, DriverCapabilities,
    DriverRegistry, FormFieldKind, TableKind,
};
use gpui_kit::assets::IconName as AssetIconName;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::button::ButtonVariant;
use gpui_kit::component::dock::{BasePanel, Panel, PanelEvent};
use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::component::list::ListItem;
use gpui_kit::component::menu::{ContextMenuExt, DropdownMenu, PopupMenu, PopupMenuItem};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::tree::{TreeEntry, TreeEvent, TreeItem, TreeState, tree};
use gpui_kit::component::{ActiveTheme, Icon, IconName, Sizable, WindowExt};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;

use crate::components::button::{Button, ButtonStyle};
use crate::components::drag_preview::DragPreview;
use crate::components::icon_button::IconButton;
use crate::components::inline_name_editor::{InlineNameEditor, InlineNameEvent};
use crate::components::menu::entity_menu_item;
use crate::components::search_bar::SearchBar;
use crate::project::{Project, ProjectEvent, SecretChange};
use crate::ui::connection_dialog::{ConnectionDialog, ConnectionDialogEvent, SavedConnection};
use crate::ui::connection_tree::{
    ConnectionContent, ConnectionEntry, ConnectionStatus, Load, NodeKind, SchemaEntry, TreeNode,
    build_tree, connection_node_id, folder_path, group_node_id, group_segments,
    insert_pending_folder,
};
use crate::util::ResultExt;

const INDENT: Pixels = px(12.);

#[derive(Clone, PartialEq, Eq, Hash)]
enum TaskKey {
    Connect(ConnectionId),
    Content(ConnectionId),
    Tables(ConnectionId, String),
}

impl TaskKey {
    fn connection(&self) -> ConnectionId {
        match self {
            Self::Connect(connection) | Self::Content(connection) => *connection,
            Self::Tables(connection, _) => *connection,
        }
    }
}

/// What is carried while a connection row is dragged.
struct DraggedConnection {
    id: ConnectionId,
    name: SharedString,
}

#[derive(Clone)]
enum FolderEditTarget {
    /// A new folder, named in a row added inside `parent`.
    New { parent: Option<String> },
    /// The folder at `path`, renamed in its own row.
    Rename { path: String },
}

/// A folder name being typed in the tree.
struct FolderEdit {
    target: FolderEditTarget,
    editor: Entity<InlineNameEditor>,
    _subscription: Subscription,
}

pub struct ConnectionsPanel {
    focus_handle: FocusHandle,
    search_input: Entity<InputState>,
    tree_state: Entity<TreeState>,
    registry: Arc<DriverRegistry>,
    project: Entity<Project>,
    connections: Vec<ConnectionEntry>,
    live_connections: HashMap<ConnectionId, Arc<dyn Connection>>,
    expanded: HashSet<SharedString>,
    filter: String,
    /// Kind of every visible node, shared with the tree's row renderer.
    node_kinds: Rc<HashMap<SharedString, NodeKind>>,
    profiles_loaded: bool,
    tasks: HashMap<TaskKey, Task<()>>,
    /// Saved folder paths, including empty ones.
    folders: Vec<String>,
    folder_edit: Option<FolderEdit>,
    dialog_subscription: Option<Subscription>,
    _subscriptions: Vec<Subscription>,
}

impl ConnectionsPanel {
    pub fn new(
        registry: Arc<DriverRegistry>,
        project: Entity<Project>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let search_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("Search connections..."));
        let tree_state = cx.new(|cx| TreeState::new(cx));

        let subscriptions = vec![
            cx.subscribe_in(&search_input, window, |this, input, event, _, cx| {
                if let InputEvent::Change = event {
                    this.filter = input
                        .read(cx)
                        .value()
                        .to_string();
                    this.rebuild_tree(cx);
                }
            }),
            cx.subscribe_in(&tree_state, window, |this, _, event, window, cx| {
                this.on_tree_event(event, window, cx)
            }),
            cx.subscribe(&project, |this, _, event, cx| {
                if let ProjectEvent::ConnectionsChanged = event {
                    this.sync_profiles(cx);
                }
            }),
        ];

        let mut panel = Self {
            focus_handle: cx.focus_handle(),
            search_input,
            tree_state,
            registry,
            project,
            connections: Vec::new(),
            live_connections: HashMap::new(),
            expanded: HashSet::new(),
            filter: String::new(),
            node_kinds: Rc::new(HashMap::new()),
            profiles_loaded: false,
            tasks: HashMap::new(),
            folders: Vec::new(),
            folder_edit: None,
            dialog_subscription: None,
            _subscriptions: subscriptions,
        };
        panel.sync_profiles(cx);
        panel
    }

    /// Mirrors the project's profiles, keeping the live state of the
    /// connections that are still there.
    fn sync_profiles(&mut self, cx: &mut Context<Self>) {
        let project = self.project.read(cx);
        let profiles = project.connections().to_vec();
        self.folders = project
            .folders()
            .map(str::to_string)
            .collect();
        self.profiles_loaded = project.connections_loaded();

        let mut previous: HashMap<ConnectionId, ConnectionEntry> = self
            .connections
            .drain(..)
            .map(|entry| (entry.profile.id, entry))
            .collect();
        for profile in profiles {
            let entry = match previous.remove(&profile.id) {
                Some(mut entry) => {
                    entry.profile = profile;
                    entry
                }
                None => ConnectionEntry::new(profile),
            };
            self.connections.push(entry);
        }
        for connection in previous.into_keys() {
            self.live_connections
                .remove(&connection);
            self.tasks
                .retain(|key, _| key.connection() != connection);
        }
        self.rebuild_tree(cx);
    }

    fn entry(&self, connection: ConnectionId) -> Option<&ConnectionEntry> {
        self.connections
            .iter()
            .find(|entry| entry.profile.id == connection)
    }

    fn entry_mut(&mut self, connection: ConnectionId) -> Option<&mut ConnectionEntry> {
        self.connections
            .iter_mut()
            .find(|entry| entry.profile.id == connection)
    }

    fn rebuild_tree(&mut self, cx: &mut Context<Self>) {
        let mut nodes = build_tree(
            &self.connections,
            &self.folders,
            &self.expanded,
            &self.filter,
        );
        if let Some(FolderEdit {
            target: FolderEditTarget::New { parent },
            ..
        }) = &self.folder_edit
        {
            insert_pending_folder(&mut nodes, parent.as_deref());
        }
        let mut node_kinds = HashMap::new();
        let items: Vec<TreeItem> = nodes
            .into_iter()
            .map(|node| into_tree_item(node, &mut node_kinds))
            .collect();
        self.node_kinds = Rc::new(node_kinds);

        self.tree_state
            .update(cx, |tree_state, cx| {
                // `set_items` clears the selection, so restore it by id.
                let selected = tree_state
                    .selected_item()
                    .map(|item| item.id.clone());
                tree_state.set_items(items, cx);
                if let Some(selected) = selected {
                    let index = tree_state.index_of(&selected);
                    tree_state.set_selected_index(index, cx);
                }
            });
        cx.notify();
    }

    fn on_tree_event(&mut self, event: &TreeEvent, window: &mut Window, cx: &mut Context<Self>) {
        match event {
            TreeEvent::Expanded(id) => {
                self.expanded
                    .insert(id.clone());
                match self
                    .node_kinds
                    .get(id)
                    .cloned()
                {
                    Some(NodeKind::Connection { id, .. }) => self.open_connection(id, window, cx),
                    Some(NodeKind::Schema { connection, name }) => {
                        let needs_load = self
                            .entry_mut(connection)
                            .and_then(|entry| entry.schema_mut(&name))
                            .is_some_and(|schema| schema.tables.needs_load());
                        if needs_load {
                            self.load_tables(connection, name, window, cx);
                        }
                    }
                    _ => {}
                }
            }
            TreeEvent::Collapsed(id) => {
                self.expanded.remove(id);
            }
        }
    }

    /// Connects if needed, then loads the connection's children if they are
    /// not loaded yet.
    fn open_connection(
        &mut self,
        connection: ConnectionId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(entry) = self.entry(connection) else {
            return;
        };
        match entry.status {
            ConnectionStatus::Disconnected | ConnectionStatus::Failed(_) => {
                self.connect(connection, window, cx)
            }
            ConnectionStatus::Connected if entry.content.needs_load() => {
                self.load_content(connection, window, cx)
            }
            ConnectionStatus::Connected | ConnectionStatus::Connecting => {}
        }
    }

    fn connect(&mut self, connection: ConnectionId, window: &mut Window, cx: &mut Context<Self>) {
        let registry = self.registry.clone();
        let secret_store = self
            .project
            .read(cx)
            .secret_store();
        let Some(entry) = self.entry_mut(connection) else {
            return;
        };
        if matches!(
            entry.status,
            ConnectionStatus::Connecting | ConnectionStatus::Connected
        ) {
            return;
        }
        let profile = entry.profile.clone();
        let Some(driver) = registry
            .get(&profile.driver_id)
            .cloned()
        else {
            let message = format!("No driver named \"{}\" is installed", profile.driver_id);
            entry.status = ConnectionStatus::Failed(message.clone());
            self.rebuild_tree(cx);
            notify_error(
                &format!("Could not connect to {}", profile.name),
                message,
                window,
                cx,
            );
            return;
        };
        entry.status = ConnectionStatus::Connecting;
        entry.content = Load::NotLoaded;

        let task = cx.spawn_in(window, async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    let values = profile
                        .form_values(driver.connection_form(), secret_store.as_ref())
                        .map_err(|error| error.to_string())?;
                    let connection = driver
                        .connect(&values)
                        .map_err(|error| error.to_string())?;
                    Ok::<Arc<dyn Connection>, String>(Arc::from(connection))
                })
                .await;
            this.update_in(cx, |this, window, cx| {
                this.finish_connect(connection, result, window, cx)
            })
            .log_err();
        });
        self.tasks
            .insert(TaskKey::Connect(connection), task);
        self.rebuild_tree(cx);
    }

    fn finish_connect(
        &mut self,
        connection: ConnectionId,
        result: Result<Arc<dyn Connection>, String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(entry) = self.entry_mut(connection) else {
            return;
        };
        match result {
            Ok(live_connection) => {
                entry.status = ConnectionStatus::Connected;
                self.live_connections
                    .insert(connection, live_connection);
                if self
                    .expanded
                    .contains(&connection_node_id(connection))
                {
                    self.load_content(connection, window, cx);
                } else {
                    self.rebuild_tree(cx);
                }
            }
            Err(message) => {
                entry.status = ConnectionStatus::Failed(message.clone());
                let title = format!("Could not connect to {}", entry.profile.name);
                self.rebuild_tree(cx);
                notify_error(&title, message, window, cx);
            }
        }
    }

    fn disconnect(&mut self, connection: ConnectionId, cx: &mut Context<Self>) {
        self.tasks
            .retain(|key, _| key.connection() != connection);
        self.live_connections
            .remove(&connection);
        if let Some(entry) = self.entry_mut(connection) {
            entry.status = ConnectionStatus::Disconnected;
            entry.content = Load::NotLoaded;
        }
        self.rebuild_tree(cx);
    }

    fn load_content(
        &mut self,
        connection: ConnectionId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(live_connection) = self
            .live_connections
            .get(&connection)
            .cloned()
        else {
            return;
        };
        let registry = self.registry.clone();
        let Some(entry) = self.entry_mut(connection) else {
            return;
        };
        let supports_schemas = registry
            .get(&entry.profile.driver_id)
            .is_some_and(|driver| {
                driver
                    .metadata()
                    .capabilities
                    .contains(DriverCapabilities::SCHEMAS)
            });
        entry.content = Load::Loading;
        let name = entry.profile.name.clone();
        self.tasks
            .retain(|key, _| !matches!(key, TaskKey::Tables(id, _) if *id == connection));

        let task = cx.spawn_in(window, async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    let relational = live_connection
                        .as_relational()
                        .ok_or_else(|| "This database cannot be browsed yet".to_string())?;
                    let content = if supports_schemas {
                        relational
                            .list_schemas()
                            .map(|schemas| {
                                ConnectionContent::Schemas(
                                    schemas
                                        .into_iter()
                                        .map(SchemaEntry::new)
                                        .collect(),
                                )
                            })
                    } else {
                        // Without schemas the whole database is one
                        // namespace, which the driver lists under "".
                        relational
                            .list_tables("")
                            .map(ConnectionContent::Tables)
                    };
                    content.map_err(|error| error.to_string())
                })
                .await;
            this.update_in(cx, |this, window, cx| {
                let Some(entry) = this.entry_mut(connection) else {
                    return;
                };
                let failure = match result {
                    Ok(content) => {
                        entry.content = Load::Loaded(content);
                        None
                    }
                    Err(message) => {
                        entry.content = Load::Failed(message.clone());
                        Some(message)
                    }
                };
                this.rebuild_tree(cx);
                if let Some(message) = failure {
                    notify_error(&format!("Could not load {name}"), message, window, cx);
                }
            })
            .log_err();
        });
        self.tasks
            .insert(TaskKey::Content(connection), task);
        self.rebuild_tree(cx);
    }

    fn load_tables(
        &mut self,
        connection: ConnectionId,
        schema: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(live_connection) = self
            .live_connections
            .get(&connection)
            .cloned()
        else {
            return;
        };
        let Some(schema_entry) = self
            .entry_mut(connection)
            .and_then(|entry| entry.schema_mut(&schema))
        else {
            return;
        };
        schema_entry.tables = Load::Loading;

        let task = cx.spawn_in(window, {
            let schema = schema.clone();
            async move |this, cx| {
                let result = cx
                    .background_spawn({
                        let schema = schema.clone();
                        async move {
                            live_connection
                                .as_relational()
                                .ok_or_else(|| "This database cannot be browsed yet".to_string())?
                                .list_tables(&schema)
                                .map_err(|error| error.to_string())
                        }
                    })
                    .await;
                this.update_in(cx, |this, window, cx| {
                    let Some(schema_entry) = this
                        .entry_mut(connection)
                        .and_then(|entry| entry.schema_mut(&schema))
                    else {
                        return;
                    };
                    let failure = match result {
                        Ok(tables) => {
                            schema_entry.tables = Load::Loaded(tables);
                            None
                        }
                        Err(message) => {
                            schema_entry.tables = Load::Failed(message.clone());
                            Some(message)
                        }
                    };
                    this.rebuild_tree(cx);
                    if let Some(message) = failure {
                        notify_error(
                            &format!("Could not load schema {schema}"),
                            message,
                            window,
                            cx,
                        );
                    }
                })
                .log_err();
            }
        });
        self.tasks
            .insert(TaskKey::Tables(connection, schema), task);
        self.rebuild_tree(cx);
    }

    fn refresh_connection(
        &mut self,
        connection: ConnectionId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(entry) = self.entry(connection) else {
            return;
        };
        match entry.status {
            ConnectionStatus::Connected => self.load_content(connection, window, cx),
            ConnectionStatus::Disconnected | ConnectionStatus::Failed(_) => {
                self.connect(connection, window, cx)
            }
            ConnectionStatus::Connecting => {}
        }
    }

    /// Every folder path in the tree: saved folders, the folders holding a
    /// connection, and their parents.
    fn known_folders(&self) -> Vec<String> {
        let mut folders: HashSet<String> = self
            .folders
            .iter()
            .cloned()
            .collect();
        for entry in &self.connections {
            let segments = group_segments(entry.profile.group.as_deref());
            for length in 1..=segments.len() {
                folders.insert(segments[..length].join("/"));
            }
        }
        folders.into_iter().collect()
    }

    /// Shows a name field in the tree: a new row for a new folder, or the
    /// folder's own row for a rename.
    fn start_folder_edit(
        &mut self,
        target: FolderEditTarget,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.profiles_loaded {
            return;
        }
        let initial_name = match &target {
            FolderEditTarget::Rename { path } => path
                .rsplit('/')
                .next()
                .unwrap_or_default()
                .to_string(),
            FolderEditTarget::New { .. } => String::new(),
        };
        let editor = cx.new(|cx| InlineNameEditor::new(&initial_name, window, cx));
        let subscription = cx.subscribe_in(&editor, window, |this, _, event, window, cx| {
            let Some(edit) = this.folder_edit.take() else {
                return;
            };
            if let InlineNameEvent::Confirm(name) = event {
                match edit.target {
                    FolderEditTarget::New { parent } => {
                        this.create_folder(parent, name, window, cx)
                    }
                    FolderEditTarget::Rename { path } => {
                        this.rename_folder(&path, name, window, cx)
                    }
                }
            }
            this.rebuild_tree(cx);
        });
        self.folder_edit = Some(FolderEdit {
            target,
            editor,
            _subscription: subscription,
        });
        self.rebuild_tree(cx);
    }

    fn create_folder(
        &mut self,
        parent: Option<String>,
        name: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match folder_path(parent.as_deref(), name, &self.known_folders()) {
            Ok(path) => {
                self.project
                    .update(cx, |project, cx| project.add_folder(path, cx));
            }
            Err(message) => notify_error("Folder not created", message.to_string(), window, cx),
        }
    }

    fn rename_folder(
        &mut self,
        path: &str,
        name: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let parent = path
            .rsplit_once('/')
            .map(|(parent, _)| parent);
        let unchanged = match parent {
            Some(parent) => format!("{parent}/{}", name.trim()) == path,
            None => name.trim() == path,
        };
        if unchanged {
            return;
        }
        match folder_path(parent, name, &self.known_folders()) {
            Ok(new_path) => {
                let old_id = group_node_id(path);
                if self.expanded.remove(&old_id) {
                    self.expanded
                        .insert(group_node_id(&new_path));
                }
                self.project
                    .update(cx, |project, cx| project.rename_folder(path, &new_path, cx));
            }
            Err(message) => notify_error("Folder not renamed", message.to_string(), window, cx),
        }
    }

    /// The folder a drop on `kind`'s row puts a connection in: the folder
    /// itself, or the folder holding the connection the row belongs to.
    /// `None` when the row is not a drop target.
    fn drop_folder(&self, kind: &NodeKind) -> Option<Option<String>> {
        match kind {
            NodeKind::Group { path } => Some(Some(path.clone())),
            NodeKind::Connection { id, .. } | NodeKind::Schema { connection: id, .. } => self
                .entry(*id)
                .map(|entry| entry.profile.group.clone()),
            NodeKind::Table { .. }
            | NodeKind::Loading
            | NodeKind::Error(_)
            | NodeKind::Empty
            | NodeKind::PendingFolder => None,
        }
    }

    fn move_connection(
        &mut self,
        connection: ConnectionId,
        folder: Option<String>,
        cx: &mut Context<Self>,
    ) {
        let Some(entry) = self.entry(connection) else {
            return;
        };
        if entry.profile.group == folder {
            return;
        }
        let mut profile = entry.profile.clone();
        profile.group = folder;
        if let Some(folder) = &profile.group {
            self.expanded
                .insert(group_node_id(folder));
        }
        self.project
            .update(cx, |project, cx| project.update_connection(profile, cx));
    }

    fn open_dialog(
        &mut self,
        existing: Option<ConnectionProfile>,
        default_group: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // A profile created now would be merged into a list that is still
        // being read, and saving it could overwrite the stored ones.
        if !self.profiles_loaded {
            return;
        }
        let title = if existing.is_some() {
            "Edit Connection"
        } else {
            "New Connection"
        };
        let secret_store = self
            .project
            .read(cx)
            .secret_store();
        let dialog = cx.new(|cx| {
            ConnectionDialog::new(
                self.registry.clone(),
                secret_store,
                existing,
                default_group,
                window,
                cx,
            )
        });
        self.dialog_subscription = Some(cx.subscribe_in(
            &dialog,
            window,
            |this, _, event: &ConnectionDialogEvent, _, cx| match event {
                ConnectionDialogEvent::Saved(saved) => this.save_connection(saved.clone(), cx),
            },
        ));
        window.open_dialog(cx, move |dialog_shell, _, _| {
            dialog_shell
                .title(title)
                .width(px(480.))
                .child(dialog.clone())
        });
    }

    fn save_connection(&mut self, saved: SavedConnection, cx: &mut Context<Self>) {
        let SavedConnection { profile, secrets } = saved;
        let connection = profile.id;
        let exists = self
            .entry(connection)
            .is_some();
        if exists {
            // The settings changed, so a live session no longer matches.
            self.disconnect(connection, cx);
        }
        let secret_changes = secrets
            .into_iter()
            .map(|(field, secret)| SecretChange::Set {
                connection,
                field,
                secret,
            })
            .collect();
        self.project
            .update(cx, |project, cx| {
                let accepted = if exists {
                    project.update_connection(profile, cx)
                } else {
                    project.add_connection(profile, cx)
                };
                if accepted {
                    project.change_secrets(secret_changes, cx);
                }
            });
    }

    fn duplicate_connection(&mut self, connection: ConnectionId, cx: &mut Context<Self>) {
        let Some(source) = self.entry(connection) else {
            return;
        };
        let mut profile = source.profile.clone();
        profile.id = ConnectionId::new();
        profile.name = format!("{} (copy)", profile.name);
        let secret_changes = self
            .password_fields(&profile)
            .into_iter()
            .map(|field| SecretChange::Copy {
                from: connection,
                to: profile.id,
                field,
            })
            .collect();
        self.project
            .update(cx, |project, cx| {
                if project.add_connection(profile, cx) {
                    project.change_secrets(secret_changes, cx);
                }
            });
    }

    fn confirm_delete(
        &mut self,
        connection: ConnectionId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(entry) = self.entry(connection) else {
            return;
        };
        let description = format!("\"{}\" will be removed permanently.", entry.profile.name);
        let panel = cx.weak_entity();
        window.open_alert_dialog(cx, move |alert, _, _| {
            let panel = panel.clone();
            alert
                .title("Delete connection?")
                .description(description.clone())
                .show_cancel(true)
                .ok_text("Delete")
                .ok_variant(ButtonVariant::Danger)
                .on_ok(move |_, _, cx| {
                    panel
                        .update(cx, |this, cx| this.delete_connection(connection, cx))
                        .log_err();
                    true
                })
        });
    }

    fn delete_connection(&mut self, connection: ConnectionId, cx: &mut Context<Self>) {
        let Some(entry) = self.entry(connection) else {
            return;
        };
        let secret_changes = self
            .password_fields(&entry.profile)
            .into_iter()
            .map(|field| SecretChange::Delete { connection, field })
            .collect();
        self.disconnect(connection, cx);
        self.project
            .update(cx, |project, cx| {
                if project.remove_connection(connection, cx) {
                    project.change_secrets(secret_changes, cx);
                }
            });
    }

    fn password_fields(&self, profile: &ConnectionProfile) -> Vec<String> {
        self.registry
            .get(&profile.driver_id)
            .map(|driver| {
                driver
                    .connection_form()
                    .iter()
                    .filter(|field| field.kind == FormFieldKind::Password)
                    .map(|field| field.id.to_string())
                    .collect()
            })
            .unwrap_or_default()
    }

    fn render_empty_state(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let message = if self.profiles_loaded {
            "No connections yet"
        } else {
            "Loading connections…"
        };
        v_flex()
            .size_full()
            .justify_center()
            .items_center()
            .gap_2()
            .text_color(cx.theme().muted_foreground)
            .child(message)
            .when(self.profiles_loaded, |this| {
                this.child(
                    Button::new("new-connection-empty")
                        .style(ButtonStyle::Filled)
                        .label("New Connection")
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.open_dialog(None, None, window, cx)
                        })),
                )
            })
    }
}

fn notify_error(title: &str, message: String, window: &mut Window, cx: &mut App) {
    window.push_notification(Notification::error(message).title(title.to_string()), cx);
}

fn into_tree_item(node: TreeNode, node_kinds: &mut HashMap<SharedString, NodeKind>) -> TreeItem {
    node_kinds.insert(node.id.clone(), node.kind);
    TreeItem::new(node.id, node.label)
        .expanded(node.expanded)
        .children(
            node.children
                .into_iter()
                .map(|child| into_tree_item(child, node_kinds)),
        )
}

fn render_tree_row(
    index: usize,
    entry: &TreeEntry,
    kind: Option<&NodeKind>,
    folder_edit: Option<(&FolderEditTarget, &Entity<InlineNameEditor>)>,
    focused_selection: bool,
    panel: &WeakEntity<ConnectionsPanel>,
    cx: &App,
) -> ListItem {
    let theme = cx.theme();
    let item = entry.item();
    let muted = theme.muted_foreground;

    let (icon, icon_color) = match kind {
        Some(NodeKind::Group { .. }) if entry.is_expanded() => (IconName::FolderOpen, muted),
        Some(NodeKind::Group { .. } | NodeKind::PendingFolder) => (IconName::Folder, muted),
        Some(NodeKind::Connection { status, .. }) => match status {
            ConnectionStatus::Disconnected => (IconName::HardDrive, muted),
            ConnectionStatus::Connecting => (IconName::LoaderCircle, theme.warning),
            ConnectionStatus::Connected => (IconName::HardDrive, theme.success),
            ConnectionStatus::Failed(_) => (IconName::CircleAlert, theme.danger),
        },
        Some(NodeKind::Schema { .. }) => (IconName::Network, theme.info),
        Some(NodeKind::Table {
            kind: TableKind::Table,
        }) => (IconName::LayoutDashboard, theme.foreground),
        Some(NodeKind::Table {
            kind: TableKind::View,
        }) => (IconName::Eye, theme.foreground),
        Some(NodeKind::Loading) => (IconName::LoaderCircle, muted),
        Some(NodeKind::Error(_)) => (IconName::CircleAlert, theme.danger),
        Some(NodeKind::Empty) | None => (IconName::Minus, muted),
    };
    let label_color = match kind {
        Some(NodeKind::Error(_)) => theme.danger,
        Some(kind) if kind.is_status() => muted,
        _ => theme.foreground,
    };
    let is_global = matches!(
        kind,
        Some(NodeKind::Connection {
            scope: ConnectionScope::Global,
            ..
        })
    );
    let chevron = entry.is_folder().then(|| {
        Icon::new(if entry.is_expanded() {
            IconName::ChevronDown
        } else {
            IconName::ChevronRight
        })
        .xsmall()
        .text_color(muted)
    });

    let editor = folder_edit.and_then(|(target, editor)| {
        let edits_this_row = match (target, kind) {
            (FolderEditTarget::New { .. }, Some(NodeKind::PendingFolder)) => true,
            (FolderEditTarget::Rename { path }, Some(NodeKind::Group { path: row_path })) => {
                path == row_path
            }
            _ => false,
        };
        edits_this_row.then_some(editor)
    });
    let is_editing = editor.is_some();
    // As in Zed's project panel, every row keeps a border so its content
    // never moves, and only its colour changes: the focus colour on the
    // active row (selected or being edited) while the panel holds focus.
    // A name field only has focus inside its panel, so editing implies it.
    let border_color = if is_editing || focused_selection {
        theme.ring
    } else {
        theme.transparent
    };

    ListItem::new(index)
        .py_0p5()
        .pl(px(4.) + INDENT * entry.depth() as f32)
        .rounded_none()
        .border_1()
        .border_r_2()
        .border_color(border_color)
        .when_some(
            match kind {
                Some(NodeKind::Connection { id, .. }) => Some(*id),
                _ => None,
            },
            |this, id| {
                this.on_drag(
                    DraggedConnection {
                        id,
                        name: item.label.clone(),
                    },
                    |dragged, _, _, cx| {
                        cx.new(|_| DragPreview::new(IconName::HardDrive, dragged.name.clone()))
                    },
                )
            },
        )
        .when_some(kind.cloned(), |this, kind| {
            let drop_color = theme.drop_target;
            let panel = panel.clone();
            this.drag_over::<DraggedConnection>(move |style, _, _, _| style.bg(drop_color))
                .on_drop(move |dragged: &DraggedConnection, _, cx| {
                    let id = dragged.id;
                    let result = panel.update(cx, |this, cx| {
                        if let Some(folder) = this.drop_folder(&kind) {
                            this.move_connection(id, folder, cx);
                        }
                    });
                    if let Err(error) = result {
                        eprintln!("connections panel dropped before a drop: {error:#}");
                    }
                })
        })
        .child(
            h_flex()
                .w_full()
                .gap_1()
                .overflow_hidden()
                .child(
                    div()
                        .flex_shrink_0()
                        .w(px(14.))
                        .when_some(chevron, |this, chevron| this.child(chevron)),
                )
                .child(
                    Icon::new(icon)
                        .small()
                        .flex_shrink_0()
                        .text_color(icon_color),
                )
                .map(|this| match editor {
                    Some(editor) => this.child(editor.clone()),
                    None => this.child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_color(label_color)
                            .child(item.label.clone()),
                    ),
                })
                .when(is_global, |this| {
                    this.child(
                        Icon::new(IconName::Globe)
                            .xsmall()
                            .flex_shrink_0()
                            .text_color(muted),
                    )
                }),
        )
}

/// "New Connection…" and "New Folder…", creating inside `folder` when set.
fn creation_menu(
    panel: &WeakEntity<ConnectionsPanel>,
    folder: Option<String>,
    menu: PopupMenu,
) -> PopupMenu {
    let connection_folder = folder.clone();
    menu.item(entity_menu_item(
        panel,
        "New Connection…",
        Some(AssetIconName::Plug),
        move |this, window, cx| this.open_dialog(None, connection_folder.clone(), window, cx),
    ))
    .item(entity_menu_item(
        panel,
        "New Folder",
        Some(AssetIconName::FolderPlus),
        move |this, window, cx| {
            this.start_folder_edit(
                FolderEditTarget::New {
                    parent: folder.clone(),
                },
                window,
                cx,
            )
        },
    ))
}

fn context_menu(
    panel: &WeakEntity<ConnectionsPanel>,
    kind: &NodeKind,
    menu: PopupMenu,
) -> PopupMenu {
    // The SQL editor does not exist yet, so the entry is shown disabled.
    let new_query = PopupMenuItem::new("New Query")
        .icon(AssetIconName::FileCode)
        .disabled(true);

    match kind {
        NodeKind::Group { path } => {
            let rename_path = path.clone();
            creation_menu(panel, Some(path.clone()), menu)
                .separator()
                .item(entity_menu_item(
                    panel,
                    "Rename",
                    Some(AssetIconName::Pencil),
                    move |this, window, cx| {
                        this.start_folder_edit(
                            FolderEditTarget::Rename {
                                path: rename_path.clone(),
                            },
                            window,
                            cx,
                        )
                    },
                ))
        }
        NodeKind::Connection { id, status, .. } => {
            let id = *id;
            let is_live = matches!(
                status,
                ConnectionStatus::Connected | ConnectionStatus::Connecting
            );
            menu.item(
                entity_menu_item(
                    panel,
                    "Connect",
                    Some(AssetIconName::Plug),
                    move |this, window, cx| this.connect(id, window, cx),
                )
                .disabled(is_live),
            )
            .item(
                entity_menu_item(
                    panel,
                    "Disconnect",
                    Some(AssetIconName::Unplug),
                    move |this, _, cx| this.disconnect(id, cx),
                )
                .disabled(!is_live),
            )
            .separator()
            .item(new_query)
            .item(
                entity_menu_item(
                    panel,
                    "Refresh",
                    Some(AssetIconName::RefreshCw),
                    move |this, window, cx| this.refresh_connection(id, window, cx),
                )
                .disabled(matches!(status, ConnectionStatus::Connecting)),
            )
            .separator()
            .item(entity_menu_item(
                panel,
                "Edit…",
                Some(AssetIconName::Pencil),
                move |this, window, cx| {
                    let profile = this
                        .entry(id)
                        .map(|entry| entry.profile.clone());
                    if let Some(profile) = profile {
                        this.open_dialog(Some(profile), None, window, cx);
                    }
                },
            ))
            .item(entity_menu_item(
                panel,
                "Duplicate",
                Some(AssetIconName::Copy),
                move |this, _, cx| this.duplicate_connection(id, cx),
            ))
            .item(entity_menu_item(
                panel,
                "Delete…",
                Some(AssetIconName::Trash),
                move |this, window, cx| this.confirm_delete(id, window, cx),
            ))
        }
        NodeKind::Schema { connection, name } => {
            let connection = *connection;
            let name = name.clone();
            menu.item(new_query)
                .item(entity_menu_item(
                    panel,
                    "Refresh",
                    Some(AssetIconName::RefreshCw),
                    move |this, window, cx| this.load_tables(connection, name.clone(), window, cx),
                ))
        }
        NodeKind::Table { .. } => menu.item(new_query),
        NodeKind::Loading | NodeKind::Error(_) | NodeKind::Empty | NodeKind::PendingFolder => menu,
    }
}

impl EventEmitter<PanelEvent> for ConnectionsPanel {}

impl Focusable for ConnectionsPanel {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl BasePanel for ConnectionsPanel {
    fn panel_name(&self) -> &'static str {
        "ConnectionsPanel"
    }
}

impl Panel for ConnectionsPanel {
    fn title_bar(&self, _cx: &App) -> bool {
        false
    }
}

impl Render for ConnectionsPanel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let panel = cx.weak_entity();
        let panel_focused = self
            .focus_handle
            .contains_focused(window, cx);
        let new_item = IconButton::new("new-item", AssetIconName::Plus)
            .small()
            .tooltip("New Connection or Folder")
            .dropdown_menu({
                let panel = panel.clone();
                move |menu, _, _| creation_menu(&panel, None, menu)
            });

        let body =
            if self.connections.is_empty() && self.folders.is_empty() && self.folder_edit.is_none()
            {
                self.render_empty_state(cx)
                    .into_any_element()
            } else {
                let node_kinds = self.node_kinds.clone();
                let folder_edit = self
                    .folder_edit
                    .as_ref()
                    .map(|edit| (edit.target.clone(), edit.editor.clone()));
                let menu_kinds = self.node_kinds.clone();
                let panel = panel.clone();
                let row_panel = panel.clone();
                tree(
                    &self.tree_state,
                    move |index, entry, is_selected, _window, cx| {
                        render_tree_row(
                            index,
                            entry,
                            node_kinds.get(&entry.item().id),
                            folder_edit
                                .as_ref()
                                .map(|(target, editor)| (target, editor)),
                            is_selected && panel_focused,
                            &row_panel,
                            cx,
                        )
                    },
                )
                .context_menu(
                    move |_, entry, menu, _, _| match menu_kinds.get(&entry.item().id) {
                        Some(kind) => context_menu(&panel, kind, menu),
                        None => menu,
                    },
                )
                .into_any_element()
            };

        v_flex()
            .flex_1()
            .size_full()
            .track_focus(&self.focus_handle)
            .child(SearchBar::new(&self.search_input).suffix(new_item))
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .child(body)
                    // A drop on a row is handled (and consumed) by the row, so
                    // this only takes drops on the empty space: the top level.
                    .on_drop({
                        let panel = panel.clone();
                        move |dragged: &DraggedConnection, _, cx| {
                            let id = dragged.id;
                            if let Err(error) =
                                panel.update(cx, |this, cx| this.move_connection(id, None, cx))
                            {
                                eprintln!("connections panel dropped before a drop: {error:#}");
                            }
                        }
                    })
                    // Rows open their own menu and stop the event, so this one
                    // only opens on the empty space around them.
                    .context_menu(move |menu, _, _| creation_menu(&panel, None, menu)),
            )
    }
}
