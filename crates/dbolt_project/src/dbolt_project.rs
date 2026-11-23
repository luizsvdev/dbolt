mod connection_store;
mod directory;
mod recent_projects;
mod secret_store;

use std::io::{self, Write};
use std::path::{Path, PathBuf};

use serde::Serialize;
use tempfile::NamedTempFile;

pub use connection_store::FileConnectionStore;
pub use directory::{DirectoryEntry, is_ignored_name, read_directory};
pub use recent_projects::RecentProjects;
pub use secret_store::KeyringSecretStore;

pub const PROJECT_DIRECTORY_NAME: &str = ".dbolt";

const CONNECTIONS_FILE_NAME: &str = "connections.json";

pub fn config_directory() -> Option<PathBuf> {
    dirs::config_dir().map(|directory| directory.join("dbolt"))
}

/// Writes `value` as pretty JSON to a temporary file next to `path`, then
/// renames it over `path`, so a crash mid-write never leaves a truncated file.
fn write_json_atomically(path: &Path, value: &impl Serialize) -> io::Result<()> {
    let directory = path
        .parent()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "path has no parent"))?;
    std::fs::create_dir_all(directory)?;
    let mut file = NamedTempFile::new_in(directory)?;
    serde_json::to_writer_pretty(&mut file, value)?;
    file.write_all(b"\n")?;
    file.as_file().sync_all()?;
    file.persist(path)
        .map_err(|error| error.error)?;
    Ok(())
}
