use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use bitflags::bitflags;
use thiserror::Error;

use crate::query::{QueryRequest, QueryResult};

#[derive(Debug, Error)]
pub enum DatabaseError {
    #[error("Invalid connection settings: {0}")]
    InvalidConfig(String),
    #[error("Could not connect: {0}")]
    Connection(String),
    #[error("Query failed: {0}")]
    Query(String),
    #[error("Not supported by this database: {0}")]
    NotSupported(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DatabaseCategory {
    Relational,
    Document,
    KeyValue,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum QueryLanguage {
    Sql,
    MongoQuery,
    RedisCommands,
}

bitflags! {
    /// What a driver can do. The UI checks these flags instead of checking
    /// which driver it is talking to.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct DriverCapabilities: u32 {
        const SCHEMAS = 1 << 0;
        const MULTIPLE_DATABASES = 1 << 1;
        const QUERY_CANCELLATION = 1 << 2;
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DriverMetadata {
    pub id: &'static str,
    pub display_name: &'static str,
    pub category: DatabaseCategory,
    pub query_language: QueryLanguage,
    pub capabilities: DriverCapabilities,
    pub default_port: Option<u16>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FormFieldKind {
    Text,
    Number,
    /// Password fields must be routed to the OS keyring by whatever persists
    /// connection profiles, never written to the profile file.
    Password,
    FilePath,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FormField {
    pub id: &'static str,
    pub label: &'static str,
    pub kind: FormFieldKind,
    pub required: bool,
    pub default: Option<&'static str>,
}

pub type ConnectionForm = [FormField];
pub type FormValues = HashMap<String, String>;

pub trait Driver: Send + Sync {
    fn metadata(&self) -> &DriverMetadata;

    /// Fields the connection dialog renders for this driver.
    fn connection_form(&self) -> &ConnectionForm;

    /// Blocks on network I/O; call from a background task, never from the
    /// foreground thread.
    fn connect(&self, values: &FormValues) -> Result<Box<dyn Connection>, DatabaseError>;
}

/// Operations every backend supports. Model-specific operations live in
/// smaller traits reached through the `as_*` accessors, so a backend only
/// implements what its data model has.
///
/// All methods block on network I/O and must run on a background task.
pub trait Connection: Send + Sync {
    fn ping(&self) -> Result<(), DatabaseError>;

    fn execute(&self, request: &QueryRequest) -> Result<QueryResult, DatabaseError>;

    fn as_relational(&self) -> Option<&dyn RelationalConnection> {
        None
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TableKind {
    Table,
    View,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TableInfo {
    pub name: String,
    pub kind: TableKind,
}

pub trait RelationalConnection: Send + Sync {
    fn list_schemas(&self) -> Result<Vec<String>, DatabaseError>;

    fn list_tables(&self, schema: &str) -> Result<Vec<TableInfo>, DatabaseError>;
}

#[derive(Default)]
pub struct DriverRegistry {
    drivers: BTreeMap<&'static str, Arc<dyn Driver>>,
}

impl DriverRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns the driver previously registered under the same id, if any.
    pub fn register(&mut self, driver: Arc<dyn Driver>) -> Option<Arc<dyn Driver>> {
        self.drivers
            .insert(driver.metadata().id, driver)
    }

    pub fn get(&self, id: &str) -> Option<&Arc<dyn Driver>> {
        self.drivers.get(id)
    }

    pub fn iter(&self) -> impl Iterator<Item = &Arc<dyn Driver>> {
        self.drivers.values()
    }
}
