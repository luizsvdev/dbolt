use std::io;
use std::path::{Path, PathBuf};

use dbolt_database::{ConnectionScope, ConnectionStore, StoreError, StoredConnections};

use crate::{
    CONNECTIONS_FILE_NAME, PROJECT_DIRECTORY_NAME, config_directory, write_json_atomically,
};

/// Stores the profiles of one scope in a JSON file. Secrets are never part of
/// a profile, so the file is safe to commit.
pub struct FileConnectionStore {
    scope: ConnectionScope,
    path: PathBuf,
}

impl FileConnectionStore {
    /// Profiles of the project at `project_root`, in `.dbolt/connections.json`.
    pub fn project(project_root: &Path) -> Self {
        Self {
            scope: ConnectionScope::Project,
            path: project_root
                .join(PROJECT_DIRECTORY_NAME)
                .join(CONNECTIONS_FILE_NAME),
        }
    }

    /// Profiles shared by every project, in the user configuration directory.
    pub fn global() -> Result<Self, StoreError> {
        let directory = config_directory().ok_or_else(|| {
            StoreError::Read("the user configuration directory is unknown".to_string())
        })?;
        Ok(Self::global_in(&directory))
    }

    /// Like [`Self::global`], with an explicit configuration directory.
    pub fn global_in(config_directory: &Path) -> Self {
        Self {
            scope: ConnectionScope::Global,
            path: config_directory.join(CONNECTIONS_FILE_NAME),
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl ConnectionStore for FileConnectionStore {
    fn scope(&self) -> ConnectionScope {
        self.scope
    }

    fn load(&self) -> Result<StoredConnections, StoreError> {
        let contents = match std::fs::read(&self.path) {
            Ok(contents) => contents,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                return Ok(StoredConnections::default());
            }
            Err(error) => {
                return Err(StoreError::Read(format!(
                    "{}: {error}",
                    self.path.display()
                )));
            }
        };
        let mut stored: StoredConnections = serde_json::from_slice(&contents)
            .map_err(|error| StoreError::Read(format!("{}: {error}", self.path.display())))?;
        for profile in &mut stored.connections {
            profile.scope = self.scope;
        }
        Ok(stored)
    }

    fn save(&self, stored: &StoredConnections) -> Result<(), StoreError> {
        write_json_atomically(&self.path, stored)
            .map_err(|error| StoreError::Write(format!("{}: {error}", self.path.display())))
    }
}

#[cfg(test)]
mod tests {
    use dbolt_database::ConnectionProfile;
    use tempfile::TempDir;

    use super::*;

    #[test]
    fn missing_file_loads_as_empty() -> Result<(), Box<dyn std::error::Error>> {
        let directory = TempDir::new()?;
        let store = FileConnectionStore::project(directory.path());

        assert_eq!(store.load()?, StoredConnections::default());
        Ok(())
    }

    #[test]
    fn saved_profiles_round_trip_with_scope() -> Result<(), Box<dyn std::error::Error>> {
        let directory = TempDir::new()?;
        let store = FileConnectionStore::project(directory.path());
        let mut profile = ConnectionProfile::new("local", "postgres", ConnectionScope::Global);
        profile
            .values
            .insert("host".into(), "localhost".into());

        store.save(&StoredConnections {
            connections: vec![profile.clone()],
            folders: vec!["work/prod".into()],
        })?;
        let loaded = store.load()?;

        assert_eq!(
            store.path(),
            directory
                .path()
                .join(".dbolt/connections.json")
        );
        assert_eq!(loaded.folders, vec!["work/prod".to_string()]);
        assert_eq!(loaded.connections.len(), 1);
        let loaded_profile = &loaded.connections[0];
        assert_eq!(loaded_profile.id, profile.id);
        assert_eq!(loaded_profile.values, profile.values);
        assert_eq!(loaded_profile.scope, ConnectionScope::Project);
        Ok(())
    }

    #[test]
    fn save_replaces_previous_profiles() -> Result<(), Box<dyn std::error::Error>> {
        let directory = TempDir::new()?;
        let store = FileConnectionStore::global_in(directory.path());
        let first = ConnectionProfile::new("first", "postgres", ConnectionScope::Global);
        let second = ConnectionProfile::new("second", "postgres", ConnectionScope::Global);

        store.save(&StoredConnections {
            connections: vec![first, second.clone()],
            folders: Vec::new(),
        })?;
        store.save(&StoredConnections {
            connections: vec![second.clone()],
            folders: Vec::new(),
        })?;
        let loaded = store.load()?;

        assert_eq!(loaded.connections.len(), 1);
        assert_eq!(loaded.connections[0].id, second.id);
        assert_eq!(loaded.connections[0].scope, ConnectionScope::Global);
        Ok(())
    }

    #[test]
    fn invalid_file_is_a_read_error() -> Result<(), Box<dyn std::error::Error>> {
        let directory = TempDir::new()?;
        let store = FileConnectionStore::global_in(directory.path());
        std::fs::write(store.path(), "not json")?;

        assert!(matches!(store.load(), Err(StoreError::Read(_))));
        Ok(())
    }
}
