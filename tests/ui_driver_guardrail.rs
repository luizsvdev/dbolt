//! The UI must never learn a driver's name: it adapts through
//! `DriverMetadata` and `DriverCapabilities` instead. Driver ids are only
//! allowed where drivers are registered.

use std::fs;
use std::path::{Path, PathBuf};

const DRIVER_ID_LITERALS: [&str; 6] = [
    "\"postgres\"",
    "\"mysql\"",
    "\"mariadb\"",
    "\"mongodb\"",
    "\"redis\"",
    "\"sqlite\"",
];

const EXEMPTION_MARKER: &str = "guardrail-allow";

fn violations_in(path: &Path, source: &str) -> Vec<String> {
    source
        .lines()
        .enumerate()
        .filter(|(_, line)| !line.contains(EXEMPTION_MARKER))
        .filter_map(|(index, line)| {
            let literal = DRIVER_ID_LITERALS
                .iter()
                .find(|literal| line.contains(**literal))?;
            Some(format!(
                "{}:{}: driver id {literal}",
                path.display(),
                index + 1
            ))
        })
        .collect()
}

fn rust_files_under(directory: &Path, files: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(directory) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            rust_files_under(&path, files);
        } else if path
            .extension()
            .is_some_and(|extension| extension == "rs")
        {
            files.push(path);
        }
    }
}

#[test]
fn ui_code_has_no_driver_id_literals() {
    let source_directory = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = Vec::new();
    rust_files_under(&source_directory, &mut files);
    assert!(
        !files.is_empty(),
        "no source files found under {source_directory:?}"
    );

    let violations: Vec<String> = files
        .iter()
        .filter_map(|path| Some((path, fs::read_to_string(path).ok()?)))
        .flat_map(|(path, source)| violations_in(path, &source))
        .collect();

    assert!(
        violations.is_empty(),
        "UI code must adapt through DriverMetadata and DriverCapabilities, not driver ids \
         (add a `{EXEMPTION_MARKER}` comment only where drivers are registered):\n{}",
        violations.join("\n")
    );
}

#[test]
fn the_scanner_flags_driver_ids_and_honors_the_exemption_marker() {
    let path = Path::new("example.rs");

    let flagged = violations_in(path, "if id == \"postgres\" { }");
    assert_eq!(flagged.len(), 1);

    let exempt = violations_in(path, "register(\"postgres\"); // guardrail-allow: registry");
    assert!(exempt.is_empty());

    assert!(violations_in(path, "let id = driver.metadata().id;").is_empty());
}
