// SPDX-License-Identifier: AGPL-3.0-only
//! Real GIO enumeration against disposable local folders, without a GTK display.

use gio::prelude::*;
use ox_core::entry::{enumerate, enumerate_blocking, Entry, EnumerateError};
use std::fs;
use std::sync::{Arc, Mutex};

#[test]
fn listing_batches_entries_and_preserves_metadata() {
    let folder = tempfile::tempdir().expect("temporary folder");
    fs::write(folder.path().join("Plan.txt"), b"draft").expect("fixture file");
    fs::write(folder.path().join(".hidden"), b"secret").expect("hidden fixture");
    fs::create_dir(folder.path().join("Projects")).expect("fixture directory");
    let uri = gio::File::for_path(folder.path()).uri();
    let mut batches = Vec::new();
    let summary =
        enumerate_blocking(&uri, false, None, 1, &mut |batch| batches.push(batch)).expect("listing");
    assert_eq!(summary.count, 2);
    assert_eq!(batches.len(), 2);
    assert!(batches.iter().all(|batch| batch.len() == 1));
    let entries: Vec<_> = batches.into_iter().flatten().collect();
    let file = entries
        .iter()
        .find(|entry| entry.name == "Plan.txt")
        .expect("file row");
    assert_eq!(file.size, Some(5));
    assert!(!file.is_dir);
    assert!(file.can_operate);
    let directory = entries
        .iter()
        .find(|entry| entry.name == "Projects")
        .expect("folder row");
    assert!(directory.is_dir);
    assert_eq!(directory.size, None);
    assert_eq!(directory.type_label, "File folder");
}

#[test]
fn hidden_toggle_changes_the_listing_and_empty_folders_finish() {
    let folder = tempfile::tempdir().expect("temporary folder");
    let uri = gio::File::for_path(folder.path()).uri();
    let mut entries = Vec::new();
    let summary =
        enumerate_blocking(&uri, false, None, 0, &mut |batch| entries.extend(batch)).expect("empty listing");
    assert_eq!(summary.count, 0);
    assert!(entries.is_empty());
    fs::write(folder.path().join(".hidden"), b"x").expect("fixture");
    let summary =
        enumerate_blocking(&uri, true, None, 0, &mut |batch| entries.extend(batch)).expect("hidden listing");
    assert_eq!(summary.count, 1);
    assert!(entries[0].hidden);
}

#[test]
fn cancelling_after_a_batch_stops_delivery() {
    let folder = tempfile::tempdir().expect("temporary folder");
    for name in ["one", "two", "three"] {
        fs::write(folder.path().join(name), b"x").expect("fixture");
    }
    let uri = gio::File::for_path(folder.path()).uri();
    let cancel = gio::Cancellable::new();
    let mut delivered = Vec::new();
    let result = enumerate_blocking(&uri, true, Some(&cancel), 1, &mut |batch| {
        delivered.extend(batch);
        cancel.cancel();
    });
    assert_eq!(result, Err(EnumerateError::Cancelled));
    assert_eq!(delivered.len(), 1);
}

#[test]
fn cancelled_before_start_delivers_no_rows() {
    let folder = tempfile::tempdir().expect("temporary folder");
    let cancel = gio::Cancellable::new();
    cancel.cancel();
    let mut delivered = Vec::<Entry>::new();
    let uri = gio::File::for_path(folder.path()).uri();
    let result = enumerate_blocking(&uri, true, Some(&cancel), 128, &mut |batch| {
        delivered.extend(batch)
    });
    assert_eq!(result, Err(EnumerateError::Cancelled));
    assert!(delivered.is_empty());
}

#[test]
fn worker_listing_completes_without_a_gtk_main_loop() {
    let folder = tempfile::tempdir().expect("temporary folder");
    fs::write(folder.path().join("Plan.txt"), b"draft").expect("fixture");
    let uri = gio::File::for_path(folder.path()).uri().to_string();
    let entries = Arc::new(Mutex::new(Vec::new()));
    let received = entries.clone();
    let task = enumerate(&uri, true, &gio::Cancellable::new(), 128, move |batch| {
        received.lock().expect("test callback lock").extend(batch);
    });
    assert_eq!(task.wait().expect("worker result").count, 1);
    assert_eq!(entries.lock().expect("test lock")[0].name, "Plan.txt");
}

#[test]
fn symlink_to_a_folder_is_browsable_without_recursing_into_it() {
    let folder = tempfile::tempdir().expect("temporary folder");
    fs::create_dir(folder.path().join("Projects")).expect("fixture directory");
    std::os::unix::fs::symlink("Projects", folder.path().join("Shortcut")).expect("symlink");
    let uri = gio::File::for_path(folder.path()).uri();
    let mut entries = Vec::new();
    enumerate_blocking(&uri, true, None, 128, &mut |batch| entries.extend(batch)).expect("listing");
    assert_eq!(entries.len(), 2);
    let link = entries
        .iter()
        .find(|entry| entry.name == "Shortcut")
        .expect("link");
    assert!(link.symlink);
    assert!(link.is_dir);
}
