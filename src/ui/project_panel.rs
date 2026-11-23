use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use dbolt_project::{DirectoryEntry, read_directory};
use futures::StreamExt;
use futures::channel::mpsc::{UnboundedSender, unbounded};
use gpui_kit::assets::IconName as AssetIconName;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::dock::{BasePanel, Panel, PanelEvent};
use gpui_kit::component::list::ListItem;
use gpui_kit::component::menu::{ContextMenuExt, PopupMenu};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::tree::{TreeEntry, TreeEvent, TreeItem, TreeState, tree};
use gpui_kit::component::{ActiveTheme, Icon, IconName, Sizable, WindowExt};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use notify::event::ModifyKind;
use notify::{EventKind, RecommendedWatcher, RecursiveMode, Watcher};

use crate::components::button::{Button, ButtonStyle};
use crate::components::drag_preview::DragPreview;
use crate::components::inline_name_editor::{InlineNameEditor, InlineNameEvent};
use crate::components::menu::entity_menu_item;
use crate::project::{Project, ProjectEvent, prompt_open_folder};
use crate::theme::{ICON_SIZE_SMALL, PANEL_FONT_SIZE, PANEL_FONT_WEIGHT, UI_FONT_FAMILY};

const WATCH_DEBOUNCE: Duration = Duration::from_millis(150);
const INDENT_PER_DEPTH: Pixels = px(12.);
const ROW_LEFT_PADDING: Pixels = px(8.);
const ROW_HEIGHT: Pixels = px(24.);
const PENDING_ID: &str = "\0pending";
const LOADING_SUFFIX: &str = "\0loading";
const EMPTY_SUFFIX: &str = "\0empty";
const ERROR_SUFFIX: &str = "\0error";

enum DirectoryListing {
    Loading,
    Loaded(Vec<DirectoryEntry>),
    Failed(SharedString),
}

#[derive(Clone)]
enum EditTarget {
    NewFile { directory: PathBuf },
    NewFolder { directory: PathBuf },
    Rename { path: PathBuf },
}

impl EditTarget {
    fn new_item_directory(&self) -> Option<&Path> {
        match self {
            Self::NewFile { directory } | Self::NewFolder { directory } => Some(directory),
            Self::Rename { .. } => None,
        }
    }
}

struct DraggedPath {
    path: PathBuf,
    name: SharedString,
    is_directory: bool,
}

struct InlineEdit {
    target: EditTarget,
    editor: Entity<InlineNameEditor>,
    _subscription: Subscription,
}

enum WatchMessage {
    Changed(PathBuf),
    Failed(String),
}

pub struct ProjectPanel {
    focus_handle: FocusHandle,
    project: Entity<Project>,
    tree_state: Entity<TreeState>,
    root: Option<PathBuf>,
    directories: HashMap<PathBuf, DirectoryListing>,
    expanded: HashSet<PathBuf>,
    directory_ids: HashMap<SharedString, PathBuf>,
    watcher: Option<Arc<Mutex<RecommendedWatcher>>>,
    watch_error: Option<SharedString>,
    load_tasks: HashMap<PathBuf, Task<()>>,
    edit: Option<InlineEdit>,
    _watch_task: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

impl ProjectPanel {
    pub fn new(project: Entity<Project>, cx: &mut Context<Self>) -> Self {
        let tree_state = cx.new(|cx| TreeState::new(cx));
        let subscriptions = vec![
            cx.subscribe(&project, |panel, _, event, cx| {
                if let ProjectEvent::Opened = event {
                    panel.open_root(cx);
                }
            }),
            cx.subscribe(&tree_state, |panel, _, event, cx| match event {
                TreeEvent::Expanded(id) => panel.directory_expanded(id, cx),
                TreeEvent::Collapsed(id) => {
                    if let Some(path) = panel.directory_ids.get(id) {
                        panel.expanded.remove(path);
                    }
                }
            }),
        ];

        let mut panel = Self {
            focus_handle: cx.focus_handle(),
            project,
            tree_state,
            root: None,
            directories: HashMap::new(),
            expanded: HashSet::new(),
            directory_ids: HashMap::new(),
            watcher: None,
            watch_error: None,
            load_tasks: HashMap::new(),
            edit: None,
            _watch_task: None,
            _subscriptions: subscriptions,
        };
        panel.open_root(cx);
        panel
    }

    fn open_root(&mut self, cx: &mut Context<Self>) {
        let root = self
            .project
            .read(cx)
            .root()
            .map(Path::to_path_buf);
        self.directories.clear();
        self.expanded.clear();
        self.directory_ids.clear();
        self.load_tasks.clear();
        self.edit = None;
        self.watcher = None;
        self.watch_error = None;
        self._watch_task = None;
        self.root = root.clone();
        if let Some(root) = root {
            self.start_watching(cx);
            self.load_directory(root, cx);
        }
        self.rebuild_tree(cx);
    }

    fn start_watching(&mut self, cx: &mut Context<Self>) {
        let (sender, mut receiver) = unbounded::<WatchMessage>();
        match notify::recommended_watcher(move |result| forward_watch_event(&sender, result)) {
            Ok(watcher) => self.watcher = Some(Arc::new(Mutex::new(watcher))),
            Err(error) => {
                self.watch_error = Some(format!("File watching is off: {error}").into());
                return;
            }
        }

        self._watch_task = Some(cx.spawn(async move |this, cx| {
            while let Some(message) = receiver.next().await {
                let mut changed = HashSet::new();
                let mut failure = None;
                collect_watch_message(message, &mut changed, &mut failure);
                cx.background_executor()
                    .timer(WATCH_DEBOUNCE)
                    .await;
                while let Ok(message) = receiver.try_recv() {
                    collect_watch_message(message, &mut changed, &mut failure);
                }
                let update_result = this.update(cx, |panel, cx| {
                    if let Some(failure) = failure {
                        panel.watch_error = Some(format!("File watching failed: {failure}").into());
                        cx.notify();
                    }
                    for directory in changed {
                        if panel
                            .directories
                            .contains_key(&directory)
                        {
                            panel.load_directory(directory, cx);
                        }
                    }
                });
                if update_result.is_err() {
                    break;
                }
            }
        }));
    }

    fn directory_expanded(&mut self, id: &SharedString, cx: &mut Context<Self>) {
        let Some(path) = self
            .directory_ids
            .get(id)
            .cloned()
        else {
            return;
        };
        self.expanded
            .insert(path.clone());
        let needs_load = !matches!(
            self.directories.get(&path),
            Some(DirectoryListing::Loaded(_))
        );
        if needs_load {
            self.load_directory(path, cx);
        }
    }

    fn load_directory(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        self.directories
            .entry(path.clone())
            .or_insert(DirectoryListing::Loading);
        let watcher = self.watcher.clone();
        let task = cx.spawn({
            let path = path.clone();
            async move |this, cx| {
                let (listing, watch_result) = cx
                    .background_spawn({
                        let path = path.clone();
                        async move {
                            let listing = read_directory(&path);
                            let watch_result = match (&listing, watcher) {
                                (Ok(_), Some(watcher)) => watcher
                                    .lock()
                                    .unwrap_or_else(PoisonError::into_inner)
                                    .watch(&path, RecursiveMode::NonRecursive),
                                _ => Ok(()),
                            };
                            (listing, watch_result)
                        }
                    })
                    .await;
                let update_result = this.update(cx, |panel, cx| {
                    panel.load_tasks.remove(&path);
                    let listing = match listing {
                        Ok(entries) => DirectoryListing::Loaded(entries),
                        Err(error) => DirectoryListing::Failed(
                            format!("Could not read {}: {error}", path.display()).into(),
                        ),
                    };
                    panel
                        .directories
                        .insert(path, listing);
                    if let Err(error) = watch_result {
                        panel.watch_error = Some(format!("File watching failed: {error}").into());
                    }
                    panel.rebuild_tree(cx);
                });
                if let Err(error) = update_result {
                    eprintln!("project panel dropped before a folder loaded: {error:#}");
                }
            }
        });
        // Replacing an in-flight read cancels it, so only the newest listing
        // of a folder is applied.
        self.load_tasks
            .insert(path, task);
    }

    fn rebuild_tree(&mut self, cx: &mut Context<Self>) {
        let selected_id = self
            .tree_state
            .read(cx)
            .selected_item()
            .map(|item| item.id.clone());

        let mut directory_ids = HashMap::new();
        let mut reachable = HashSet::new();
        let items = match &self.root {
            Some(root) => {
                reachable.insert(root.clone());
                self.child_items(root, &mut directory_ids, &mut reachable)
            }
            None => Vec::new(),
        };
        self.directory_ids = directory_ids;
        self.directories
            .retain(|path, _| reachable.contains(path));
        self.expanded
            .retain(|path| reachable.contains(path));

        self.tree_state
            .update(cx, |state, cx| {
                state.set_items(items, cx);
                if let Some(selected_id) = selected_id {
                    let index = state.index_of(&selected_id);
                    state.set_selected_index(index, cx);
                }
            });
        cx.notify();
    }

    fn child_items(
        &self,
        directory: &Path,
        directory_ids: &mut HashMap<SharedString, PathBuf>,
        reachable: &mut HashSet<PathBuf>,
    ) -> Vec<TreeItem> {
        let Some(DirectoryListing::Loaded(entries)) = self
            .directories
            .get(directory)
        else {
            return Vec::new();
        };
        let pending = self
            .edit
            .as_ref()
            .filter(|edit| {
                edit.target
                    .new_item_directory()
                    == Some(directory)
            })
            .map(|_| TreeItem::new(PENDING_ID, ""));
        pending
            .into_iter()
            .chain(entries.iter().map(|entry| {
                let id = SharedString::from(
                    entry
                        .path
                        .to_string_lossy()
                        .into_owned(),
                );
                let item = TreeItem::new(id.clone(), entry.name.clone());
                if !entry.is_directory {
                    return item;
                }
                directory_ids.insert(id.clone(), entry.path.clone());
                let is_expanded = self
                    .expanded
                    .contains(&entry.path);
                if is_expanded
                    || self
                        .directories
                        .contains_key(&entry.path)
                {
                    reachable.insert(entry.path.clone());
                }
                let children = match self
                    .directories
                    .get(&entry.path)
                {
                    Some(DirectoryListing::Loaded(entries))
                        if entries.is_empty() && !self.has_new_item_in(&entry.path) =>
                    {
                        vec![placeholder_item(&id, EMPTY_SUFFIX, "Empty folder")]
                    }
                    Some(DirectoryListing::Loaded(_)) => {
                        self.child_items(&entry.path, directory_ids, reachable)
                    }
                    Some(DirectoryListing::Failed(message)) => {
                        vec![placeholder_item(&id, ERROR_SUFFIX, message.clone())]
                    }
                    Some(DirectoryListing::Loading) | None => {
                        vec![placeholder_item(&id, LOADING_SUFFIX, "Loading…")]
                    }
                };
                item.children(children)
                    .expanded(is_expanded)
            }))
            .collect()
    }

    fn has_new_item_in(&self, directory: &Path) -> bool {
        self.edit
            .as_ref()
            .is_some_and(|edit| {
                edit.target
                    .new_item_directory()
                    == Some(directory)
            })
    }

    fn start_edit(&mut self, target: EditTarget, window: &mut Window, cx: &mut Context<Self>) {
        let initial_name = match &target {
            EditTarget::Rename { path } => path
                .file_name()
                .map(|name| {
                    name.to_string_lossy()
                        .into_owned()
                })
                .unwrap_or_default(),
            EditTarget::NewFile { .. } | EditTarget::NewFolder { .. } => String::new(),
        };
        if let Some(directory) = target.new_item_directory() {
            if self.root.as_deref() != Some(directory) {
                self.expanded
                    .insert(directory.to_path_buf());
            }
            if !matches!(
                self.directories
                    .get(directory),
                Some(DirectoryListing::Loaded(_))
            ) {
                self.load_directory(directory.to_path_buf(), cx);
            }
        }
        let editor = cx.new(|cx| InlineNameEditor::new(&initial_name, window, cx));
        let subscription = cx.subscribe_in(&editor, window, |this, _, event, window, cx| {
            let Some(edit) = this.edit.take() else {
                return;
            };
            if let InlineNameEvent::Confirm(name) = event {
                this.apply_edit(edit.target, name.clone(), window, cx);
            }
            this.rebuild_tree(cx);
        });
        self.edit = Some(InlineEdit {
            target,
            editor,
            _subscription: subscription,
        });
        self.rebuild_tree(cx);
    }

    fn apply_edit(
        &mut self,
        target: EditTarget,
        name: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Err(message) = validate_file_name(&name) {
            show_error(message, window, cx);
            return;
        }
        let refresh = match &target {
            EditTarget::NewFile { directory } | EditTarget::NewFolder { directory } => {
                directory.clone()
            }
            EditTarget::Rename { path } => match path.parent() {
                Some(parent) => parent.to_path_buf(),
                None => return,
            },
        };
        self.run_file_operation(vec![refresh], window, cx, move || match target {
            EditTarget::NewFile { directory } => std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(directory.join(&name))
                .map(|_| ())
                .map_err(|error| format!("Could not create file \"{name}\": {error}")),
            EditTarget::NewFolder { directory } => std::fs::create_dir(directory.join(&name))
                .map_err(|error| format!("Could not create folder \"{name}\": {error}")),
            EditTarget::Rename { path } => rename_entry(&path, &name),
        });
    }

    fn move_path(
        &mut self,
        path: PathBuf,
        directory: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(source_directory) = path
            .parent()
            .map(Path::to_path_buf)
        else {
            return;
        };
        if source_directory == directory {
            return;
        }
        if directory.starts_with(&path) {
            show_error("A folder cannot be moved into itself", window, cx);
            return;
        }
        let Some(name) = path
            .file_name()
            .map(|name| name.to_os_string())
        else {
            return;
        };
        if self.root.as_deref() != Some(directory.as_path()) {
            self.expanded
                .insert(directory.clone());
        }
        let target = directory.join(name);
        self.run_file_operation(vec![source_directory, directory], window, cx, move || {
            move_entry(&path, &target)
        });
    }

    fn move_to_trash(&mut self, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        let Some(parent) = path
            .parent()
            .map(Path::to_path_buf)
        else {
            return;
        };
        self.run_file_operation(vec![parent], window, cx, move || {
            trash::delete(&path)
                .map_err(|error| format!("Could not move {} to the trash: {error}", path.display()))
        });
    }

    fn run_file_operation(
        &mut self,
        refresh: Vec<PathBuf>,
        window: &mut Window,
        cx: &mut Context<Self>,
        operation: impl FnOnce() -> Result<(), String> + Send + 'static,
    ) {
        cx.spawn_in(window, async move |this, cx| {
            let result = cx
                .background_spawn(async move { operation() })
                .await;
            let update_result = this.update_in(cx, |panel, window, cx| {
                if let Err(message) = result {
                    show_error(message, window, cx);
                }
                for directory in refresh {
                    // A folder that was expanded for this change but never
                    // read has no listing yet, and still needs its first read.
                    if panel
                        .directories
                        .contains_key(&directory)
                        || panel
                            .expanded
                            .contains(&directory)
                    {
                        panel.load_directory(directory, cx);
                    }
                }
            });
            if let Err(error) = update_result {
                eprintln!("project panel dropped before a file change finished: {error:#}");
            }
        })
        .detach();
    }

    fn relative_path(&self, path: &Path) -> PathBuf {
        self.root
            .as_deref()
            .and_then(|root| path.strip_prefix(root).ok())
            .map(Path::to_path_buf)
            .unwrap_or_else(|| path.to_path_buf())
    }

    fn render_empty_state(&self, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .size_full()
            .justify_center()
            .items_center()
            .gap_3()
            .p_4()
            .child(
                div()
                    .text_color(cx.theme().muted_foreground)
                    .child("No folder is open"),
            )
            .child(
                Button::new("open-folder")
                    .style(ButtonStyle::Filled)
                    .label("Open Folder")
                    .on_click({
                        let project = self.project.clone();
                        move |_, _, cx| prompt_open_folder(&project, cx)
                    }),
            )
    }

    fn render_message(&self, message: SharedString, is_error: bool, cx: &App) -> impl IntoElement {
        let color = if is_error {
            cx.theme().danger
        } else {
            cx.theme().muted_foreground
        };
        div()
            .px_3()
            .py_2()
            .text_color(color)
            .child(message)
    }
}

fn placeholder_item(
    parent_id: &SharedString,
    suffix: &str,
    label: impl Into<SharedString>,
) -> TreeItem {
    TreeItem::new(format!("{parent_id}{suffix}"), label).disabled(true)
}

fn render_tree_entry(
    index: usize,
    entry: &TreeEntry,
    edit: Option<(&EditTarget, &Entity<InlineNameEditor>)>,
    focused_selection: bool,
    panel: &WeakEntity<ProjectPanel>,
    cx: &App,
) -> ListItem {
    let item = entry.item();
    let is_pending = item.id.as_ref() == PENDING_ID;
    let editor = edit.and_then(|(target, editor)| {
        let edits_this_row = match target {
            EditTarget::Rename { path } => *item.id == *path.to_string_lossy(),
            EditTarget::NewFile { .. } | EditTarget::NewFolder { .. } => is_pending,
        };
        edits_this_row.then_some((target, editor))
    });
    let (icon, color) = if let Some((target, _)) = editor.filter(|_| is_pending) {
        let icon = match target {
            EditTarget::NewFolder { .. } => IconName::Folder,
            EditTarget::NewFile { .. } | EditTarget::Rename { .. } => IconName::File,
        };
        (Some(icon), cx.theme().foreground)
    } else if entry.is_disabled() {
        let icon = item
            .id
            .ends_with(ERROR_SUFFIX)
            .then_some(IconName::TriangleAlert);
        let color = if icon.is_some() {
            cx.theme().danger
        } else {
            cx.theme().muted_foreground
        };
        (icon, color)
    } else if entry.is_folder() {
        let icon = if entry.is_expanded() {
            IconName::FolderOpen
        } else {
            IconName::Folder
        };
        (Some(icon), cx.theme().foreground)
    } else {
        (Some(IconName::File), cx.theme().foreground)
    };
    let guide_color = cx.theme().border;
    // The border is always there, so the row never shifts when it is shown.
    let border_color = if editor.is_some() || focused_selection {
        cx.theme().ring
    } else {
        cx.theme().transparent
    };

    ListItem::new(index)
        .px_0()
        .py_0()
        .rounded_none()
        .border_1()
        .border_r_2()
        .border_color(border_color)
        .map(|this| {
            // Placeholder rows and the row being edited are not files.
            if item.id.contains('\0') || editor.is_some() {
                return this;
            }
            let path = PathBuf::from(item.id.as_ref());
            let is_directory = entry.is_folder();
            let directory = if is_directory {
                path.clone()
            } else {
                match path.parent() {
                    Some(parent) => parent.to_path_buf(),
                    None => return this,
                }
            };
            let drop_color = cx.theme().drop_target;
            let panel = panel.clone();
            this.on_drag(
                DraggedPath {
                    path,
                    name: item.label.clone(),
                    is_directory,
                },
                |dragged, _, _, cx| {
                    let icon = if dragged.is_directory {
                        IconName::Folder
                    } else {
                        IconName::File
                    };
                    cx.new(|_| DragPreview::new(icon, dragged.name.clone()))
                },
            )
            .drag_over::<DraggedPath>(move |style, _, _, _| style.bg(drop_color))
            .on_drop(move |dragged: &DraggedPath, window, cx| {
                let path = dragged.path.clone();
                let directory = directory.clone();
                if let Err(error) =
                    panel.update(cx, |this, cx| this.move_path(path, directory, window, cx))
                {
                    eprintln!("project panel dropped before a drop: {error:#}");
                }
            })
        })
        .child(
            h_flex()
                .h(ROW_HEIGHT)
                .pl(ROW_LEFT_PADDING)
                .children((0..entry.depth()).map(|_| {
                    div()
                        .flex_none()
                        .h_full()
                        .w(INDENT_PER_DEPTH)
                        .child(
                            div()
                                .h_full()
                                .w(px(1.))
                                .ml(ICON_SIZE_SMALL / 2. - px(0.5))
                                .bg(guide_color),
                        )
                }))
                .child(
                    h_flex()
                        .flex_1()
                        .min_w_0()
                        .gap_1p5()
                        .text_color(color)
                        .when_some(icon, |this, icon| {
                            this.child(
                                Icon::new(icon)
                                    .small()
                                    .flex_shrink_0()
                                    .text_color(cx.theme().muted_foreground),
                            )
                        })
                        .map(|this| match editor {
                            Some((_, editor)) => this.child(editor.clone()),
                            None => this.child(
                                div()
                                    .min_w_0()
                                    .truncate()
                                    .child(item.label.clone()),
                            ),
                        }),
                ),
        )
}

fn entry_menu(
    panel: &WeakEntity<ProjectPanel>,
    path: PathBuf,
    directory: PathBuf,
    menu: PopupMenu,
) -> PopupMenu {
    let menu = creation_items(panel, directory, menu).separator();
    let rename_path = path.clone();
    let trash_path = path.clone();
    path_items(panel, path, menu)
        .separator()
        .item(entity_menu_item(
            panel,
            "Rename",
            Some(AssetIconName::Pencil),
            move |this, window, cx| {
                this.start_edit(
                    EditTarget::Rename {
                        path: rename_path.clone(),
                    },
                    window,
                    cx,
                )
            },
        ))
        .item(entity_menu_item(
            panel,
            "Move to Trash",
            Some(AssetIconName::Trash),
            move |this, window, cx| this.move_to_trash(trash_path.clone(), window, cx),
        ))
}

fn creation_items(
    panel: &WeakEntity<ProjectPanel>,
    directory: PathBuf,
    menu: PopupMenu,
) -> PopupMenu {
    let folder_directory = directory.clone();
    menu.item(entity_menu_item(
        panel,
        "New File",
        Some(AssetIconName::FilePlus),
        move |this, window, cx| {
            this.start_edit(
                EditTarget::NewFile {
                    directory: directory.clone(),
                },
                window,
                cx,
            )
        },
    ))
    .item(entity_menu_item(
        panel,
        "New Folder",
        Some(AssetIconName::FolderPlus),
        move |this, window, cx| {
            this.start_edit(
                EditTarget::NewFolder {
                    directory: folder_directory.clone(),
                },
                window,
                cx,
            )
        },
    ))
}

fn path_items(panel: &WeakEntity<ProjectPanel>, path: PathBuf, menu: PopupMenu) -> PopupMenu {
    let reveal_path = path.clone();
    let copy_path = path.clone();
    menu.item(entity_menu_item(
        panel,
        "Reveal in File Manager",
        Some(AssetIconName::FolderSearch),
        move |_, _, cx| cx.reveal_path(&reveal_path),
    ))
    .item(entity_menu_item(
        panel,
        "Copy Path",
        Some(AssetIconName::Copy),
        move |_, _, cx| {
            cx.write_to_clipboard(ClipboardItem::new_string(
                copy_path
                    .to_string_lossy()
                    .into_owned(),
            ))
        },
    ))
    .item(entity_menu_item(
        panel,
        "Copy Relative Path",
        None,
        move |this, _, cx| {
            let relative = this.relative_path(&path);
            cx.write_to_clipboard(ClipboardItem::new_string(
                relative
                    .to_string_lossy()
                    .into_owned(),
            ))
        },
    ))
}

fn validate_file_name(name: &str) -> Result<(), &'static str> {
    if name == "." || name == ".." {
        return Err("\".\" and \"..\" are not valid names");
    }
    if name.contains('/') || name.contains('\\') {
        return Err("A name cannot contain \"/\" or \"\\\"");
    }
    Ok(())
}

fn rename_entry(path: &Path, name: &str) -> Result<(), String> {
    let Some(parent) = path.parent() else {
        return Err(format!("Cannot rename {}", path.display()));
    };
    let target = parent.join(name);
    if target == path {
        return Ok(());
    }
    move_entry(path, &target)
}

/// Moves `path` to `target`, refusing to replace an existing entry
/// (`std::fs::rename` would silently overwrite a file).
fn move_entry(path: &Path, target: &Path) -> Result<(), String> {
    let name = target
        .file_name()
        .map(|name| name.to_string_lossy())
        .unwrap_or_default();
    match target.try_exists() {
        Ok(true) => Err(format!("\"{name}\" already exists there")),
        Ok(false) => std::fs::rename(path, target)
            .map_err(|error| format!("Could not move \"{name}\": {error}")),
        Err(error) => Err(format!("Could not check \"{name}\": {error}")),
    }
}

fn show_error(message: impl Into<SharedString>, window: &mut Window, cx: &mut App) {
    window.push_notification(Notification::error(message.into()), cx);
}

fn forward_watch_event(
    sender: &UnboundedSender<WatchMessage>,
    result: notify::Result<notify::Event>,
) {
    let messages = match result {
        Ok(event) => {
            let changes_listing = matches!(
                event.kind,
                EventKind::Any
                    | EventKind::Create(_)
                    | EventKind::Remove(_)
                    | EventKind::Modify(ModifyKind::Name(_) | ModifyKind::Any)
                    | EventKind::Other
            );
            if !changes_listing {
                return;
            }
            event
                .paths
                .iter()
                .filter_map(|path| path.parent())
                .map(|parent| WatchMessage::Changed(parent.to_path_buf()))
                .collect()
        }
        Err(error) => vec![WatchMessage::Failed(error.to_string())],
    };
    for message in messages {
        // A closed channel means the panel is gone, so nothing is listening.
        if sender
            .unbounded_send(message)
            .is_err()
        {
            return;
        }
    }
}

fn collect_watch_message(
    message: WatchMessage,
    changed: &mut HashSet<PathBuf>,
    failure: &mut Option<String>,
) {
    match message {
        WatchMessage::Changed(path) => {
            changed.insert(path);
        }
        WatchMessage::Failed(error) => *failure = Some(error),
    }
}

impl EventEmitter<PanelEvent> for ProjectPanel {}

impl Focusable for ProjectPanel {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl BasePanel for ProjectPanel {
    fn panel_name(&self) -> &'static str {
        "ProjectPanel"
    }
}

impl Panel for ProjectPanel {
    fn title_bar(&self, _: &App) -> bool {
        false
    }
}

impl Render for ProjectPanel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let panel_focused = self
            .focus_handle
            .contains_focused(window, cx);
        div()
            .size_full()
            .track_focus(&self.focus_handle)
            .font_family(UI_FONT_FAMILY)
            .text_size(PANEL_FONT_SIZE)
            .font_weight(PANEL_FONT_WEIGHT)
            .child(self.render_content(panel_focused, cx))
    }
}

impl ProjectPanel {
    fn render_content(&self, panel_focused: bool, cx: &mut Context<Self>) -> AnyElement {
        let Some(root) = &self.root else {
            return self
                .render_empty_state(cx)
                .into_any_element();
        };

        let root_message = match self.directories.get(root) {
            Some(DirectoryListing::Loaded(entries))
                if entries.is_empty() && !self.has_new_item_in(root) =>
            {
                Some((SharedString::from("This folder is empty"), false))
            }
            Some(DirectoryListing::Loaded(_)) => None,
            Some(DirectoryListing::Failed(message)) => Some((message.clone(), true)),
            Some(DirectoryListing::Loading) | None => Some((SharedString::from("Loading…"), false)),
        };
        let panel = cx.weak_entity();
        let root = root.clone();
        let edit = self
            .edit
            .as_ref()
            .map(|edit| (edit.target.clone(), edit.editor.clone()));
        v_flex()
            .size_full()
            .when_some(self.watch_error.clone(), |this, message| {
                this.child(self.render_message(message, true, cx))
            })
            .map(|this| match root_message {
                Some((message, is_error)) => this.child(self.render_message(message, is_error, cx)),
                None => this.child({
                    let row_panel = panel.clone();
                    let tree_panel = panel.clone();
                    let drop_panel = panel.clone();
                    let drop_root = root.clone();
                    div()
                        .flex_1()
                        .min_h_0()
                        .child(
                            tree(
                                &self.tree_state,
                                move |index, entry, is_selected, _window, cx| {
                                    let edit = edit
                                        .as_ref()
                                        .map(|(target, editor)| (target, editor));
                                    render_tree_entry(
                                        index,
                                        entry,
                                        edit,
                                        is_selected && panel_focused,
                                        &tree_panel,
                                        cx,
                                    )
                                },
                            )
                            .context_menu(
                                move |_, entry, menu, _, _| {
                                    let id = &entry.item().id;
                                    // Placeholder and pending rows are not files.
                                    if id.contains('\0') {
                                        return menu;
                                    }
                                    let path = PathBuf::from(id.as_ref());
                                    let directory = if entry.is_folder() {
                                        path.clone()
                                    } else {
                                        match path.parent() {
                                            Some(parent) => parent.to_path_buf(),
                                            None => return menu,
                                        }
                                    };
                                    entry_menu(&row_panel, path, directory, menu)
                                },
                            ),
                        )
                        // A drop on a row is handled (and consumed) by the row,
                        // so this only takes drops on the empty space: the root.
                        .on_drop(move |dragged: &DraggedPath, window, cx| {
                            let path = dragged.path.clone();
                            let root = drop_root.clone();
                            if let Err(error) = drop_panel
                                .update(cx, |this, cx| this.move_path(path, root, window, cx))
                            {
                                eprintln!("project panel dropped before a drop: {error:#}");
                            }
                        })
                        // Rows open their own menu and stop the event, so this
                        // one only opens on the empty space below them.
                        .context_menu(move |menu, _, _| {
                            let menu = creation_items(&panel, root.clone(), menu).separator();
                            path_items(&panel, root.clone(), menu)
                        })
                }),
            })
            .into_any_element()
    }
}
