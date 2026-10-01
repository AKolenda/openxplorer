// SPDX-License-Identifier: AGPL-3.0-only
//! Real GIO listings of disposable local folders, awaited on a plain
//! `glib::MainContext` without GTK or a display.
//!
//! Complements the hand-filled `GFileInfo` tests in `ox-core/src/entry`,
//! which port `desktop/tests/test_gio_serialization.py`, with what the
//! local GIO backend actually reports.

use std::fs;
use std::path::Path;

use gio::prelude::*;
use ox_core::entry::{enumerate_folder, Entry, EntryError};
use tempfile::TempDir;

/// A temporary folder holding `files`, each written with `contents`.
fn folder_with_files(files: &[(&str, &[u8])]) -> TempDir {
    let folder = tempfile::tempdir().expect("temporary folder");
    for (name, contents) in files {
        fs::write(folder.path().join(name), contents).expect("fixture file");
    }
    folder
}

/// Lists `folder` to the end and returns every row delivered.
fn list(folder: &Path) -> Result<Vec<Entry>, EntryError> {
    let uri = gio::File::for_path(folder).uri();
    let mut entries = Vec::new();
    let listing = enumerate_folder(&uri, |batch| entries.extend(batch));
    glib::MainContext::new().block_on(listing)?;
    Ok(entries)
}

/// The row named `name`; the test fails when there is none.
fn find<'a>(entries: &'a [Entry], name: &str) -> &'a Entry {
    entries
        .iter()
        .find(|entry| entry.name == name)
        .unwrap_or_else(|| panic!("no row named {name}"))
}

/// parity: VIEW-002
#[test]
fn listing_preserves_file_and_folder_metadata() {
    let folder = folder_with_files(&[("Plan.txt", b"draft")]);
    fs::create_dir(folder.path().join("Projects")).expect("fixture directory");

    let entries = list(folder.path()).expect("listing");

    assert_eq!(entries.len(), 2);
    let file = find(&entries, "Plan.txt");
    assert_eq!(file.size, Some(5));
    assert!(!file.is_dir);
    assert!(file.can_operate);
    assert!(
        file.modified.is_some_and(|seconds| seconds > 0),
        "{:?}",
        file.modified
    );
    assert_eq!(file.content_type.as_deref(), Some("text/plain"));
    let directory = find(&entries, "Projects");
    assert!(directory.is_dir);
    assert_eq!(directory.size, None);
    assert_eq!(directory.type_label, "File folder");
}

/// A file of the demo Documents folder and the Type column text it must
/// show.
struct DocumentCase {
    name: &'static str,
    type_label: &'static str,
}

/// Regression: the Documents folder showed GIO's raw descriptions ("Word
/// 2007 document", "Excel 2007 spreadsheet", "Plain text document")
/// instead of the interface's names. GIO reads the content type from each
/// real file here, so the whole listing path is exercised, not only the
/// label table.
///
/// parity: VIEW-002
#[test]
fn documents_are_listed_with_the_interface_type_names() {
    let cases = [
        DocumentCase {
            name: "Meeting notes.docx",
            type_label: "Word document",
        },
        DocumentCase {
            name: "Project budget.xlsx",
            type_label: "Excel worksheet",
        },
        DocumentCase {
            name: "Q3 presentation.pptx",
            type_label: "PowerPoint presentation",
        },
        DocumentCase {
            name: "Read me.txt",
            type_label: "Text document",
        },
        DocumentCase {
            name: "Website assets.zip",
            type_label: "Compressed folder",
        },
    ];
    // Empty files would all be `application/x-zerosize`, so each holds a
    // few bytes; GIO then names the type by its extension.
    let files: Vec<(&str, &[u8])> = cases
        .iter()
        .map(|case| (case.name, b"sample".as_slice()))
        .collect();
    let folder = folder_with_files(&files);

    let entries = list(folder.path()).expect("listing");

    for case in cases {
        assert_eq!(
            find(&entries, case.name).type_label,
            case.type_label,
            "{}",
            case.name
        );
    }
}

/// parity: VIEW-024
#[test]
fn hidden_items_are_listed_and_flagged() {
    let folder = folder_with_files(&[("Plan.txt", b"draft"), (".secret", b"x")]);

    let entries = list(folder.path()).expect("listing");

    assert!(find(&entries, ".secret").is_hidden);
    assert!(!find(&entries, "Plan.txt").is_hidden);
}

/// parity: VIEW-024
#[test]
fn names_in_a_hidden_list_are_flagged_hidden() {
    let folder = folder_with_files(&[
        ("Notes.txt", b"x"),
        ("Plan.txt", b"x"),
        (".hidden", b"Notes.txt\n"),
    ]);

    let entries = list(folder.path()).expect("listing");

    assert!(find(&entries, "Notes.txt").is_hidden);
    assert!(!find(&entries, "Plan.txt").is_hidden);
}

/// Backup files (`name~`) are listed like any file, as Dolphin and the
/// Python app show them.
#[test]
fn backup_files_are_not_hidden() {
    let folder = folder_with_files(&[("Plan.txt", b"x"), ("Plan.txt~", b"x")]);

    let entries = list(folder.path()).expect("listing");

    assert!(!find(&entries, "Plan.txt~").is_hidden);
    assert!(!find(&entries, "Plan.txt").is_hidden);
}

#[test]
fn an_empty_folder_finishes_without_rows() {
    let folder = folder_with_files(&[]);
    assert_eq!(list(folder.path()), Ok(Vec::new()));
}

/// parity: VIEW-002, NAV-040
#[test]
fn symlink_to_a_folder_is_browsable_without_recursing_into_it() {
    let folder = folder_with_files(&[]);
    fs::create_dir(folder.path().join("Projects")).expect("fixture directory");
    std::os::unix::fs::symlink("Projects", folder.path().join("Shortcut")).expect("symlink");

    let entries = list(folder.path()).expect("listing");

    assert_eq!(entries.len(), 2);
    let link = find(&entries, "Shortcut");
    assert!(link.is_symlink);
    assert!(link.is_dir);
}

/// parity: OPS-037
#[test]
fn a_missing_folder_reports_not_found() {
    let folder = folder_with_files(&[]);
    let error = list(&folder.path().join("gone")).expect_err("nothing to list");
    assert_eq!(error.code(), "not-found");
    assert!(!error.needs_mount());
}

/// parity: OPS-037
#[test]
fn listing_a_file_reports_not_directory() {
    let folder = folder_with_files(&[("Plan.txt", b"draft")]);
    let file = folder.path().join("Plan.txt");
    let error = list(&file).expect_err("a file is not a folder");
    assert_eq!(error.code(), "not-directory");
}
