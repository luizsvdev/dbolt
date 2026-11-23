use std::io;
use std::path::{Path, PathBuf};

use crate::{config_directory, write_json_atomically};

const RECENT_PROJECTS_FILE_NAME: &str = "recent_projects.json";
const MAXIMUM_RECENT_PROJECTS: usize = 10;

/// The recently opened project folders, most recent first. Blocks on file
/// I/O; call it from a background task.
pub struct RecentProjects {
    path: PathBuf,
}

impl RecentProjects {
    pub fn new() -> io::Result<Self> {
        let directory = config_directory().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::NotFound,
                "the user configuration directory is unknown",
            )
        })?;
        Ok(Self::in_directory(&directory))
    }

    pub fn in_directory(config_directory: &Path) -> Self {
        Self {
            path: config_directory.join(RECENT_PROJECTS_FILE_NAME),
        }
    }

    pub fn load(&self) -> io::Result<Vec<PathBuf>> {
        match std::fs::read(&self.path) {
            Ok(contents) => serde_json::from_slice(&contents).map_err(io::Error::from),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(Vec::new()),
            Err(error) => Err(error),
        }
    }

    /// Moves `project` to the front of the list and saves it. Returns the
    /// updated list.
    pub fn record(&self, project: &Path) -> io::Result<Vec<PathBuf>> {
        let mut projects = self.load()?;
        projects.retain(|existing| existing != project);
        projects.insert(0, project.to_path_buf());
        projects.truncate(MAXIMUM_RECENT_PROJECTS);
        write_json_atomically(&self.path, &projects)?;
        Ok(projects)
    }
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::*;

    #[test]
    fn missing_file_is_empty() -> io::Result<()> {
        let directory = TempDir::new()?;
        let recent = RecentProjects::in_directory(directory.path());

        assert!(recent.load()?.is_empty());
        Ok(())
    }

    #[test]
    fn record_puts_most_recent_first_without_duplicates() -> io::Result<()> {
        let directory = TempDir::new()?;
        let recent = RecentProjects::in_directory(directory.path());

        recent.record(Path::new("/projects/first"))?;
        recent.record(Path::new("/projects/second"))?;
        recent.record(Path::new("/projects/first"))?;

        assert_eq!(
            recent.load()?,
            vec![
                PathBuf::from("/projects/first"),
                PathBuf::from("/projects/second")
            ]
        );
        Ok(())
    }

    #[test]
    fn record_keeps_at_most_the_maximum() -> io::Result<()> {
        let directory = TempDir::new()?;
        let recent = RecentProjects::in_directory(directory.path());

        for index in 0..MAXIMUM_RECENT_PROJECTS + 5 {
            recent.record(&PathBuf::from(format!("/projects/{index}")))?;
        }
        let projects = recent.load()?;

        assert_eq!(projects.len(), MAXIMUM_RECENT_PROJECTS);
        assert_eq!(
            projects.first(),
            Some(&PathBuf::from(format!(
                "/projects/{}",
                MAXIMUM_RECENT_PROJECTS + 4
            )))
        );
        Ok(())
    }
}
