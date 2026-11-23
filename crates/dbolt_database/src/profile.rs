use std::collections::{BTreeMap, HashMap};
use std::fmt;
use std::sync::{Mutex, PoisonError};

use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

use crate::driver::{ConnectionForm, FormFieldKind, FormValues};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ConnectionId(Uuid);

impl ConnectionId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for ConnectionId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for ConnectionId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

/// Where a profile is stored. New profiles belong to the open project unless
/// the user makes them global, as in DataGrip.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum ConnectionScope {
    #[default]
    Project,
    Global,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConnectionProfile {
    pub id: ConnectionId,
    pub name: String,
    pub driver_id: String,
    /// Non-secret form values only. Values of `FormFieldKind::Password`
    /// fields live in a `SecretStore`. A `BTreeMap` keeps the saved file's
    /// key order stable, so it diffs cleanly when committed.
    #[serde(default)]
    pub values: BTreeMap<String, String>,
    /// Folder path in the connections tree, segments separated by `/`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group: Option<String>,
    /// Implied by the file the profile was read from, so it is not saved.
    #[serde(skip)]
    pub scope: ConnectionScope,
}

impl ConnectionProfile {
    pub fn new(
        name: impl Into<String>,
        driver_id: impl Into<String>,
        scope: ConnectionScope,
    ) -> Self {
        Self {
            id: ConnectionId::new(),
            name: name.into(),
            driver_id: driver_id.into(),
            values: BTreeMap::new(),
            group: None,
            scope,
        }
    }

    /// Builds the values a `Driver::connect` call needs, reading password
    /// fields from `secrets`. Blocks on the secret store, so call it from a
    /// background task.
    pub fn form_values(
        &self,
        form: &ConnectionForm,
        secrets: &dyn SecretStore,
    ) -> Result<FormValues, StoreError> {
        let mut values: FormValues = self
            .values
            .iter()
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect();
        for field in form {
            if field.kind == FormFieldKind::Password {
                values.remove(field.id);
                if let Some(secret) = secrets.get(self.id, field.id)? {
                    values.insert(field.id.to_string(), secret);
                }
            }
        }
        Ok(values)
    }
}

/// Everything one scope stores: its profiles and the folders of the
/// connections tree.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoredConnections {
    #[serde(default)]
    pub connections: Vec<ConnectionProfile>,
    /// Folder paths, segments separated by `/`. Kept so that folders without
    /// connections are not lost; a profile's `group` names its folder.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub folders: Vec<String>,
}

#[derive(Debug, Error)]
pub enum StoreError {
    #[error("Could not read connections: {0}")]
    Read(String),
    #[error("Could not save connections: {0}")]
    Write(String),
    #[error("Could not access the secret store: {0}")]
    Secret(String),
}

/// Persists the profiles and folders of one scope. Implementations block on
/// I/O; call them from a background task.
pub trait ConnectionStore: Send + Sync {
    fn scope(&self) -> ConnectionScope;

    /// Returned profiles have `scope` set to `self.scope()`.
    fn load(&self) -> Result<StoredConnections, StoreError>;

    /// Replaces everything stored for this scope.
    fn save(&self, stored: &StoredConnections) -> Result<(), StoreError>;
}

/// Holds password-like form values outside the profile files. Implementations
/// block (the OS keyring may prompt); call them from a background task.
pub trait SecretStore: Send + Sync {
    fn get(&self, connection: ConnectionId, field: &str) -> Result<Option<String>, StoreError>;

    fn set(&self, connection: ConnectionId, field: &str, secret: &str) -> Result<(), StoreError>;

    fn delete(&self, connection: ConnectionId, field: &str) -> Result<(), StoreError>;
}

/// Keeps profiles in memory only; for tests and for UI work before a file
/// store is wired in.
pub struct MemoryConnectionStore {
    scope: ConnectionScope,
    stored: Mutex<StoredConnections>,
}

impl MemoryConnectionStore {
    pub fn new(scope: ConnectionScope) -> Self {
        Self {
            scope,
            stored: Mutex::new(StoredConnections::default()),
        }
    }
}

impl ConnectionStore for MemoryConnectionStore {
    fn scope(&self) -> ConnectionScope {
        self.scope
    }

    fn load(&self) -> Result<StoredConnections, StoreError> {
        // A poisoned lock only means another thread panicked mid-update;
        // the data itself is still valid.
        let mut stored = self
            .stored
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        for profile in &mut stored.connections {
            profile.scope = self.scope;
        }
        Ok(stored)
    }

    fn save(&self, stored: &StoredConnections) -> Result<(), StoreError> {
        *self
            .stored
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = stored.clone();
        Ok(())
    }
}

#[derive(Default)]
pub struct MemorySecretStore {
    secrets: Mutex<HashMap<(ConnectionId, String), String>>,
}

impl MemorySecretStore {
    pub fn new() -> Self {
        Self::default()
    }
}

impl SecretStore for MemorySecretStore {
    fn get(&self, connection: ConnectionId, field: &str) -> Result<Option<String>, StoreError> {
        let secrets = self
            .secrets
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        Ok(secrets
            .get(&(connection, field.to_string()))
            .cloned())
    }

    fn set(&self, connection: ConnectionId, field: &str, secret: &str) -> Result<(), StoreError> {
        let mut secrets = self
            .secrets
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        secrets.insert((connection, field.to_string()), secret.to_string());
        Ok(())
    }

    fn delete(&self, connection: ConnectionId, field: &str) -> Result<(), StoreError> {
        let mut secrets = self
            .secrets
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        secrets.remove(&(connection, field.to_string()));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::driver::FormField;

    const FORM: [FormField; 2] = [
        FormField {
            id: "host",
            label: "Host",
            kind: FormFieldKind::Text,
            required: true,
            default: None,
        },
        FormField {
            id: "password",
            label: "Password",
            kind: FormFieldKind::Password,
            required: false,
            default: None,
        },
    ];

    #[test]
    fn form_values_reads_passwords_from_secret_store() -> Result<(), StoreError> {
        let mut profile = ConnectionProfile::new("local", "postgres", ConnectionScope::Project);
        profile
            .values
            .insert("host".into(), "localhost".into());
        profile
            .values
            .insert("password".into(), "stale".into());
        let secrets = MemorySecretStore::new();
        secrets.set(profile.id, "password", "secret")?;

        let values = profile.form_values(&FORM, &secrets)?;

        assert_eq!(
            values
                .get("host")
                .map(String::as_str),
            Some("localhost")
        );
        assert_eq!(
            values
                .get("password")
                .map(String::as_str),
            Some("secret")
        );
        Ok(())
    }

    #[test]
    fn memory_store_sets_scope_on_load() -> Result<(), StoreError> {
        let store = MemoryConnectionStore::new(ConnectionScope::Global);
        let profile = ConnectionProfile::new("shared", "postgres", ConnectionScope::Project);
        store.save(&StoredConnections {
            connections: vec![profile],
            folders: vec!["work".into()],
        })?;

        let loaded = store.load()?;

        assert_eq!(loaded.connections.len(), 1);
        assert!(
            loaded
                .connections
                .iter()
                .all(|profile| profile.scope == ConnectionScope::Global)
        );
        assert_eq!(loaded.folders, vec!["work".to_string()]);
        Ok(())
    }
}
