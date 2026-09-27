// SPDX-License-Identifier: AGPL-3.0-only
//! Real GIO listings of disposable local folders, awaited on a plain
//! `glib::MainContext` without GTK or a display.

use std::fs;
use std::path::Path;

use gio::prelude::*;
use ox_core::entry::{enumerate_folder, Entry, EnumerateError};

/// Lists `folder` to the end and returns every row delivered.
fn list(folder: &Path) -> Result<Vec<Entry>, EnumerateError> {
    let uri = gio::File::for_path(folder).uri();
    let mut entries = Vec::new();
    let listing = enumerate_folder(&uri, |batch| entries.extend(batch));
    glib::MainContext::new().block_on(listing)?;
    Ok(entries)
}

fn find<'a>(entries: &'a [Entry], name: &str) -> &'a Entry {
    entries
        .iter()
        .find(|entry| entry.name == name)
        .unwrap_or_else(|| panic!("no row named {name}"))
}

/// parity: VIEW-002
#[test]
fn listing_preserves_file_and_folder_metadata() {
    let folder = tempfile::tempdir().expect("temporary folder");
    fs::write(folder.path().join("Plan.txt"), b"draft").expect("fixture file");
    fs::create_dir(folder.path().join("Projects")).expect("fixture directory");

    let entries = list(folder.path()).expect("listing");

    assert_eq!(entries.len(), 2);
    let file = find(&entries, "Plan.txt");
    assert_eq!(file.size, Some(5));
    assert!(!file.is_dir);
    assert!(file.can_operate);
    assert!(file.modified > 0);
    assert_eq!(file.content_type.as_deref(), Some("text/plain"));
    let directory = find(&entries, "Projects");
    assert!(directory.is_dir);
    assert_eq!(directory.size, None);
    assert_eq!(directory.type_label, "File folder");
}

/// parity: VIEW-024
#[test]
fn hidden_items_are_listed_and_flagged() {
    let folder = tempfile::tempdir().expect("temporary folder");
    fs::write(folder.path().join("Plan.txt"), b"draft").expect("fixture file");
    fs::write(folder.path().join(".secret"), b"x").expect("dot-file fixture");

    let entries = list(folder.path()).expect("listing");

    assert!(find(&entries, ".secret").hidden);
    assert!(!find(&entries, "Plan.txt").hidden);
}

/// parity: VIEW-024
#[test]
fn names_in_a_hidden_list_are_flagged_hidden() {
    let folder = tempfile::tempdir().expect("temporary folder");
    fs::write(folder.path().join("Notes.txt"), b"x").expect("fixture file");
    fs::write(folder.path().join("Plan.txt"), b"x").expect("fixture file");
    fs::write(folder.path().join(".hidden"), b"Notes.txt\n").expect("hidden list");

    let entries = list(folder.path()).expect("listing");

    assert!(find(&entries, "Notes.txt").hidden);
    assert!(!find(&entries, "Plan.txt").hidden);
}

#[test]
fn an_empty_folder_finishes_without_rows() {
    let folder = tempfile::tempdir().expect("temporary folder");
    assert_eq!(list(folder.path()), Ok(Vec::new()));
}

#[test]
fn symlink_to_a_folder_is_browsable_without_recursing_into_it() {
    let folder = tempfile::tempdir().expect("temporary folder");
    fs::create_dir(folder.path().join("Projects")).expect("fixture directory");
    std::os::unix::fs::symlink("Projects", folder.path().join("Shortcut")).expect("symlink");

    let entries = list(folder.path()).expect("listing");

    assert_eq!(entries.len(), 2);
    let link = find(&entries, "Shortcut");
    assert!(link.symlink);
    assert!(link.is_dir);
}

/// parity: OPS-037
#[test]
fn a_missing_folder_reports_not_found() {
    let folder = tempfile::tempdir().expect("temporary folder");
    let error = list(&folder.path().join("gone")).expect_err("nothing to list");
    assert_eq!(error.code(), "not-found");
    assert!(!error.needs_mount());
}

/// parity: OPS-037
#[test]
fn listing_a_file_reports_not_directory() {
    let folder = tempfile::tempdir().expect("temporary folder");
    let file = folder.path().join("Plan.txt");
    fs::write(&file, b"draft").expect("fixture file");
    let error = list(&file).expect_err("a file is not a folder");
    assert_eq!(error.code(), "not-directory");
}
