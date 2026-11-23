use std::io;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirectoryEntry {
    pub path: PathBuf,
    pub name: String,
    pub is_directory: bool,
}

/// Names hidden from the project tree because they are tool internals, not
/// project content.
pub fn is_ignored_name(name: &str) -> bool {
    name == ".git"
}

/// Lists the direct children of `directory`, folders first, each group sorted
/// by name ignoring case. Blocks on file I/O; call it from a background task.
pub fn read_directory(directory: &Path) -> io::Result<Vec<DirectoryEntry>> {
    let mut entries = Vec::new();
    for entry in std::fs::read_dir(directory)? {
        let entry = entry?;
        let name = entry
            .file_name()
            .to_string_lossy()
            .into_owned();
        if is_ignored_name(&name) {
            continue;
        }
        let path = entry.path();
        let file_type = entry.file_type()?;
        // Symlinks are shown as what they point to; a broken one is a file.
        let is_directory = if file_type.is_symlink() {
            std::fs::metadata(&path).is_ok_and(|metadata| metadata.is_dir())
        } else {
            file_type.is_dir()
        };
        entries.push(DirectoryEntry {
            path,
            name,
            is_directory,
        });
    }
    entries.sort_by(|left, right| {
        right
            .is_directory
            .cmp(&left.is_directory)
            .then_with(|| {
                left.name
                    .to_lowercase()
                    .cmp(&right.name.to_lowercase())
            })
    });
    Ok(entries)
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::*;

    #[test]
    fn lists_folders_first_and_skips_git() -> io::Result<()> {
        let directory = TempDir::new()?;
        std::fs::create_dir(directory.path().join(".git"))?;
        std::fs::create_dir(
            directory
                .path()
                .join("queries"),
        )?;
        std::fs::write(directory.path().join("b.sql"), "")?;
        std::fs::write(directory.path().join("A.sql"), "")?;

        let entries = read_directory(directory.path())?;
        let names: Vec<&str> = entries
            .iter()
            .map(|entry| entry.name.as_str())
            .collect();

        assert_eq!(names, ["queries", "A.sql", "b.sql"]);
        assert!(entries[0].is_directory);
        Ok(())
    }
}
