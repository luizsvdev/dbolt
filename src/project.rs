use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use dbolt_database::{
    Connection, ConnectionId, ConnectionProfile, ConnectionScope, ConnectionStore,
    MemoryConnectionStore, SecretStore, StoreError, StoredConnections,
};
use dbolt_project::{FileConnectionStore, KeyringSecretStore, RecentProjects};
use gpui_kit::{
    App, AppContext, Context, Entity, EventEmitter, PathPromptOptions, SharedString, Task,
    WeakEntity,
};

#[derive(Debug, Clone)]
pub enum ProjectEvent {
    Opened,
    ConnectionsChanged,
    LiveConnectionsChanged,
    RecentProjectsChanged,
    Error(SharedString),
}

pub enum SecretChange {
    Set {
        connection: ConnectionId,
        field: String,
        secret: String,
    },
    Copy {
        from: ConnectionId,
        to: ConnectionId,
        field: String,
    },
    Delete {
        connection: ConnectionId,
        field: String,
    },
}

impl SecretChange {
    fn apply(self, secrets: &dyn SecretStore) -> Result<(), StoreError> {
        match self {
            Self::Set {
                connection,
                field,
                secret,
            } => secrets.set(connection, &field, &secret),
            Self::Copy { from, to, field } => match secrets.get(from, &field)? {
                Some(secret) => secrets.set(to, &field, &secret),
                None => Ok(()),
            },
            Self::Delete { connection, field } => secrets.delete(connection, &field),
        }
    }
}

pub struct Project {
    root: Option<PathBuf>,
    project_store: Option<Arc<dyn ConnectionStore>>,
    global_store: Arc<dyn ConnectionStore>,
    secret_store: Arc<dyn SecretStore>,
    connections: Vec<ConnectionProfile>,
    live_connections: HashMap<ConnectionId, Arc<dyn Connection>>,
    folders: Vec<(ConnectionScope, String)>,
    /// False while the stores are being read. Changing profiles before the
    /// read finishes would save a list missing the stored ones.
    connections_loaded: bool,
    /// Scopes whose store failed to load; saving them would overwrite the
    /// profiles that could not be read.
    unavailable_scopes: HashSet<ConnectionScope>,
    recent_projects: Vec<PathBuf>,
    /// Store reads and writes run one after another through this chain, so
    /// an older save can never land after a newer one.
    store_task: Task<()>,
    recent_projects_task: Task<()>,
}

impl EventEmitter<ProjectEvent> for Project {}

impl Project {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let (global_store, global_store_error): (Arc<dyn ConnectionStore>, _) =
            match FileConnectionStore::global() {
                Ok(store) => (Arc::new(store), None),
                Err(error) => (
                    Arc::new(MemoryConnectionStore::new(ConnectionScope::Global)),
                    Some(error),
                ),
            };

        let mut project = Self {
            root: None,
            project_store: None,
            global_store,
            secret_store: Arc::new(KeyringSecretStore::new()),
            connections: Vec::new(),
            live_connections: HashMap::new(),
            folders: Vec::new(),
            connections_loaded: false,
            unavailable_scopes: HashSet::new(),
            recent_projects: Vec::new(),
            store_task: Task::ready(()),
            recent_projects_task: Task::ready(()),
        };
        if let Some(error) = global_store_error {
            project.store_task = cx.spawn(async move |this, cx| {
                if let Err(update_error) = this.update(cx, |project, cx| {
                    project.report_error(error.to_string(), cx);
                }) {
                    eprintln!("project dropped before reporting an error: {update_error:#}");
                }
            });
        }
        project.load_connections(cx);
        project.load_recent_projects(cx);
        project
    }

    pub fn root(&self) -> Option<&Path> {
        self.root.as_deref()
    }

    pub fn name(&self) -> Option<SharedString> {
        let root = self.root.as_ref()?;
        let name = root
            .file_name()
            .map(|name| {
                name.to_string_lossy()
                    .into_owned()
            })
            .unwrap_or_else(|| {
                root.to_string_lossy()
                    .into_owned()
            });
        Some(name.into())
    }

    pub fn connections(&self) -> &[ConnectionProfile] {
        &self.connections
    }

    pub fn live_connection(&self, id: ConnectionId) -> Option<Arc<dyn Connection>> {
        self.live_connections
            .get(&id)
            .cloned()
    }

    pub fn set_live_connection(
        &mut self,
        id: ConnectionId,
        connection: Option<Arc<dyn Connection>>,
        cx: &mut Context<Self>,
    ) {
        let changed = match connection {
            Some(connection) => {
                self.live_connections
                    .insert(id, connection);
                true
            }
            None => self
                .live_connections
                .remove(&id)
                .is_some(),
        };
        if changed {
            cx.emit(ProjectEvent::LiveConnectionsChanged);
        }
    }

    pub fn folders(&self) -> impl Iterator<Item = &str> {
        self.folders
            .iter()
            .map(|(_, path)| path.as_str())
    }

    pub fn rename_folder(&mut self, from: &str, to: &str, cx: &mut Context<Self>) -> bool {
        let mut scopes = HashSet::new();
        for (scope, path) in &self.folders {
            if renamed_folder_path(path, from, to).is_some() {
                scopes.insert(*scope);
            }
        }
        for profile in &self.connections {
            if profile
                .group
                .as_deref()
                .and_then(|group| renamed_folder_path(group, from, to))
                .is_some()
            {
                scopes.insert(profile.scope);
            }
        }
        if scopes.is_empty() {
            return false;
        }
        if !scopes
            .iter()
            .all(|scope| self.can_save(*scope, cx))
        {
            return false;
        }
        for (_, path) in &mut self.folders {
            if let Some(renamed) = renamed_folder_path(path, from, to) {
                *path = renamed;
            }
        }
        for profile in &mut self.connections {
            if let Some(renamed) = profile
                .group
                .as_deref()
                .and_then(|group| renamed_folder_path(group, from, to))
            {
                profile.group = Some(renamed);
            }
        }
        self.connections_changed(cx);
        for scope in scopes {
            self.save_scope(scope, cx);
        }
        true
    }

    pub fn add_folder(&mut self, path: String, cx: &mut Context<Self>) -> bool {
        let scope = if self.project_store.is_some() {
            ConnectionScope::Project
        } else {
            ConnectionScope::Global
        };
        if !self.can_save(scope, cx) {
            return false;
        }
        if !self
            .folders
            .iter()
            .any(|(_, existing)| *existing == path)
        {
            self.folders
                .push((scope, path));
        }
        self.connections_changed(cx);
        self.save_scope(scope, cx);
        true
    }

    pub fn connections_loaded(&self) -> bool {
        self.connections_loaded
    }

    pub fn recent_projects(&self) -> &[PathBuf] {
        &self.recent_projects
    }

    pub fn secret_store(&self) -> Arc<dyn SecretStore> {
        self.secret_store.clone()
    }

    pub fn open(&mut self, root: PathBuf, cx: &mut Context<Self>) {
        self.project_store = Some(Arc::new(FileConnectionStore::project(&root)));
        self.root = Some(root.clone());
        self.connections
            .retain(|profile| profile.scope == ConnectionScope::Global);
        self.folders
            .retain(|(scope, _)| *scope == ConnectionScope::Global);
        self.connections_loaded = false;
        cx.emit(ProjectEvent::Opened);
        cx.emit(ProjectEvent::ConnectionsChanged);
        cx.notify();
        self.load_connections(cx);
        self.record_recent_project(root, cx);
    }

    pub fn add_connection(&mut self, profile: ConnectionProfile, cx: &mut Context<Self>) -> bool {
        if !self.can_save(profile.scope, cx) {
            return false;
        }
        let scope = profile.scope;
        self.connections.push(profile);
        self.sort_connections();
        self.connections_changed(cx);
        self.save_scope(scope, cx);
        true
    }

    pub fn update_connection(
        &mut self,
        profile: ConnectionProfile,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(existing) = self
            .connections
            .iter_mut()
            .find(|existing| existing.id == profile.id)
        else {
            self.report_error(
                format!("Connection \"{}\" no longer exists", profile.name),
                cx,
            );
            return false;
        };
        let previous_scope = existing.scope;
        let scope = profile.scope;
        if !self.can_save(scope, cx) || !self.can_save(previous_scope, cx) {
            return false;
        }
        let Some(existing) = self
            .connections
            .iter_mut()
            .find(|existing| existing.id == profile.id)
        else {
            return false;
        };
        *existing = profile;
        self.sort_connections();
        self.connections_changed(cx);
        self.save_scope(scope, cx);
        if previous_scope != scope {
            self.save_scope(previous_scope, cx);
        }
        true
    }

    /// Keyring entries are left for the caller to delete.
    pub fn remove_connection(&mut self, id: ConnectionId, cx: &mut Context<Self>) -> bool {
        let Some(index) = self
            .connections
            .iter()
            .position(|profile| profile.id == id)
        else {
            return false;
        };
        let profile = self.connections.remove(index);
        if !self.can_save(profile.scope, cx) {
            self.connections
                .insert(index, profile);
            return false;
        }
        self.connections_changed(cx);
        self.save_scope(profile.scope, cx);
        true
    }

    pub fn change_secrets(&mut self, changes: Vec<SecretChange>, cx: &mut Context<Self>) {
        if changes.is_empty() {
            return;
        }
        let secret_store = self.secret_store.clone();
        self.chain_store_task(cx, move || {
            for change in changes {
                change.apply(secret_store.as_ref())?;
            }
            Ok(())
        });
    }

    fn can_save(&mut self, scope: ConnectionScope, cx: &mut Context<Self>) -> bool {
        let message = if !self.connections_loaded {
            "Connections are still loading. Try again in a moment."
        } else if self
            .unavailable_scopes
            .contains(&scope)
        {
            match scope {
                ConnectionScope::Project => {
                    "The project connections could not be loaded, so saving them now would overwrite them."
                }
                ConnectionScope::Global => {
                    "The global connections could not be loaded, so saving them now would overwrite them."
                }
            }
        } else {
            return true;
        };
        self.report_error(message, cx);
        false
    }

    pub fn report_error(&mut self, message: impl Into<SharedString>, cx: &mut Context<Self>) {
        cx.emit(ProjectEvent::Error(message.into()));
    }

    fn connections_changed(&mut self, cx: &mut Context<Self>) {
        cx.emit(ProjectEvent::ConnectionsChanged);
        cx.notify();
    }

    fn sort_connections(&mut self) {
        // A stable sort keeps the stores' own order within each scope.
        self.connections
            .sort_by_key(|profile| match profile.scope {
                ConnectionScope::Project => 0,
                ConnectionScope::Global => 1,
            });
    }

    fn store_for(&self, scope: ConnectionScope) -> Option<Arc<dyn ConnectionStore>> {
        match scope {
            ConnectionScope::Project => self.project_store.clone(),
            ConnectionScope::Global => Some(self.global_store.clone()),
        }
    }

    fn save_scope(&mut self, scope: ConnectionScope, cx: &mut Context<Self>) {
        let Some(store) = self.store_for(scope) else {
            self.report_error("Open a folder to save project connections", cx);
            return;
        };
        let stored = StoredConnections {
            connections: self
                .connections
                .iter()
                .filter(|profile| profile.scope == scope)
                .cloned()
                .collect(),
            folders: self
                .folders
                .iter()
                .filter(|(folder_scope, _)| *folder_scope == scope)
                .map(|(_, path)| path.clone())
                .collect(),
        };
        self.chain_store_task(cx, move || store.save(&stored));
    }

    fn chain_store_task(
        &mut self,
        cx: &mut Context<Self>,
        operation: impl FnOnce() -> Result<(), StoreError> + Send + 'static,
    ) {
        let previous = std::mem::replace(&mut self.store_task, Task::ready(()));
        self.store_task = cx.spawn(async move |this, cx| {
            previous.await;
            let result = cx
                .background_spawn(async move { operation() })
                .await;
            if let Err(error) = result {
                report_error_from_task(&this, error.to_string(), cx);
            }
        });
    }

    fn load_connections(&mut self, cx: &mut Context<Self>) {
        let root = self.root.clone();
        let project_store = self.project_store.clone();
        let global_store = self.global_store.clone();
        let previous = std::mem::replace(&mut self.store_task, Task::ready(()));
        self.store_task = cx.spawn(async move |this, cx| {
            previous.await;
            let (project_result, global_result) = cx
                .background_spawn(async move {
                    let project_result = project_store.map(|store| store.load());
                    (project_result, global_store.load())
                })
                .await;
            let update_result = this.update(cx, |project, cx| {
                if project.root != root {
                    return;
                }
                let mut connections = Vec::new();
                let mut folders = Vec::new();
                project
                    .unavailable_scopes
                    .clear();
                match project_result {
                    Some(Ok(stored)) => {
                        connections.extend(stored.connections);
                        folders.extend(
                            stored
                                .folders
                                .into_iter()
                                .map(|path| (ConnectionScope::Project, path)),
                        );
                    }
                    Some(Err(error)) => {
                        project
                            .unavailable_scopes
                            .insert(ConnectionScope::Project);
                        project.report_error(error.to_string(), cx);
                    }
                    None => {}
                }
                match global_result {
                    Ok(stored) => {
                        connections.extend(stored.connections);
                        folders.extend(
                            stored
                                .folders
                                .into_iter()
                                .map(|path| (ConnectionScope::Global, path)),
                        );
                    }
                    Err(error) => {
                        project
                            .unavailable_scopes
                            .insert(ConnectionScope::Global);
                        project.report_error(error.to_string(), cx);
                    }
                }
                project.connections = connections;
                project.folders = folders;
                project.connections_loaded = true;
                project.connections_changed(cx);
            });
            if let Err(error) = update_result {
                eprintln!("project dropped before connections loaded: {error:#}");
            }
        });
    }

    fn load_recent_projects(&mut self, cx: &mut Context<Self>) {
        self.recent_projects_task = cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move { RecentProjects::new()?.load() })
                .await;
            apply_recent_projects(&this, result, cx);
        });
    }

    fn record_recent_project(&mut self, root: PathBuf, cx: &mut Context<Self>) {
        let previous = std::mem::replace(&mut self.recent_projects_task, Task::ready(()));
        self.recent_projects_task = cx.spawn(async move |this, cx| {
            previous.await;
            let result = cx
                .background_spawn(async move { RecentProjects::new()?.record(&root) })
                .await;
            apply_recent_projects(&this, result, cx);
        });
    }
}

fn renamed_folder_path(path: &str, from: &str, to: &str) -> Option<String> {
    if path == from {
        return Some(to.to_string());
    }
    path.strip_prefix(from)
        .and_then(|rest| rest.strip_prefix('/'))
        .map(|rest| format!("{to}/{rest}"))
}

fn apply_recent_projects(
    this: &WeakEntity<Project>,
    result: std::io::Result<Vec<PathBuf>>,
    cx: &mut gpui_kit::AsyncApp,
) {
    let update_result = this.update(cx, |project, cx| match result {
        Ok(projects) => {
            project.recent_projects = projects;
            cx.emit(ProjectEvent::RecentProjectsChanged);
            cx.notify();
        }
        Err(error) => project.report_error(format!("Could not read recent projects: {error}"), cx),
    });
    if let Err(error) = update_result {
        eprintln!("project dropped before recent projects loaded: {error:#}");
    }
}

fn report_error_from_task(
    this: &WeakEntity<Project>,
    message: String,
    cx: &mut gpui_kit::AsyncApp,
) {
    if let Err(error) = this.update(cx, |project, cx| project.report_error(message, cx)) {
        eprintln!("project dropped before reporting an error: {error:#}");
    }
}

pub fn prompt_open_folder(project: &Entity<Project>, cx: &mut App) {
    let paths = cx.prompt_for_paths(PathPromptOptions {
        files: false,
        directories: true,
        multiple: false,
        prompt: Some("Open".into()),
    });
    let project = project.downgrade();
    cx.spawn(async move |cx| {
        let result = match paths.await {
            Ok(result) => result,
            Err(_canceled) => return,
        };
        match result {
            Ok(Some(paths)) => {
                if let Some(root) = paths.into_iter().next()
                    && let Err(error) = project.update(cx, |project, cx| project.open(root, cx))
                {
                    eprintln!("project dropped before opening a folder: {error:#}");
                }
            }
            Ok(None) => {}
            Err(error) => {
                report_error_from_task(&project, format!("Could not open folder: {error:#}"), cx)
            }
        }
    })
    .detach();
}

#[cfg(test)]
mod tests {
    use super::renamed_folder_path;

    #[test]
    fn renamed_folder_path_moves_the_folder_and_its_contents_only() {
        assert_eq!(
            renamed_folder_path("work", "work", "jobs"),
            Some("jobs".to_string())
        );
        assert_eq!(
            renamed_folder_path("work/prod", "work", "jobs"),
            Some("jobs/prod".to_string())
        );
        assert_eq!(renamed_folder_path("workshop", "work", "jobs"), None);
        assert_eq!(renamed_folder_path("home", "work", "jobs"), None);
    }
}
