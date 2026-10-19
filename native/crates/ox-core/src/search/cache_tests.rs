// SPDX-License-Identifier: AGPL-3.0-only
//! Ports `v2.0.0:desktop/tests/test_v05.py::IndexTests`: how scans and live
//! updates write the cache, and what a search finds afterwards.
//!
//! Every test starts from [`ScannedShare`], an SMB root whose scan has
//! begun, as `IndexTests.setUp` does. `test_command_queue_deduplicates` is
//! ported next to the request queue, in `requests.rs`.

use std::fs::{self, DirBuilder, Permissions};
use std::os::unix::fs::{DirBuilderExt, PermissionsExt};

use super::fixtures::{found_names, listed_file, listed_folder, root_state, search, ScannedShare, SHARE};
use super::index::SearchIndex;
use super::root::{Caching, HiddenItems, RootStatus};
use super::scan::ScanOutcome;
use crate::test_support::permission_bits;

/// Ported from `v2.0.0:desktop/tests/test_v05.py::IndexTests::test_cached_regular_not_directory`
///
/// parity: SRCH-009
#[test]
fn a_cached_regular_file_is_never_a_folder() {
    let share = ScannedShare::new();
    let mut file = listed_file(SHARE, "bank.pdf");
    file.is_dir = true;

    share.store(&[file]);

    let hits = search(&share.index, "bank");
    assert!(!hits[0].is_dir);
}

/// Ported from `v2.0.0:desktop/tests/test_v05.py::IndexTests::test_search_full_parent_path`
///
/// parity: SRCH-007, SRCH-009
#[test]
fn results_carry_their_full_parent_folder() {
    let share = ScannedShare::new();

    share.store(&[listed_file(&format!("{SHARE}/Nested"), "bank.pdf")]);

    let hits = search(&share.index, "bank");
    assert_eq!(hits[0].parent_uri, "smb://nas/share/Nested");
    assert_eq!(hits[0].path, "\\\\nas\\share\\Nested\\bank.pdf");
}

/// Ported from `v2.0.0:desktop/tests/test_v05.py::IndexTests::test_delta_add`
///
/// parity: SRCH-028
#[test]
fn a_live_update_adds_a_new_item() {
    let share = ScannedShare::new();

    share.replace(SHARE, &[listed_file(SHARE, "bank.pdf")]);

    assert_eq!(share.found_names("bank"), ["bank.pdf"]);
}

/// Ported from `v2.0.0:desktop/tests/test_v05.py::IndexTests::test_delta_remove`
///
/// parity: SRCH-028
#[test]
fn a_live_update_removes_a_deleted_item() {
    let share = ScannedShare::new();
    share.store(&[listed_file(SHARE, "bank.pdf")]);

    share.replace(SHARE, &[]);

    assert!(share.found_names("bank").is_empty());
}

/// Ported from `v2.0.0:desktop/tests/test_v05.py::IndexTests::test_delete_folder_prunes_descendants_only`
///
/// parity: SRCH-028
#[test]
fn deleting_a_folder_prunes_only_its_descendants() {
    let share = ScannedShare::new();
    let old = format!("{SHARE}/old");
    share.store(&[
        listed_folder(SHARE, "old"),
        listed_file(&old, "bank.pdf"),
        listed_file(SHARE, "keep.pdf"),
    ]);

    share.replace(SHARE, &[listed_file(SHARE, "keep.pdf")]);

    assert!(share.found_names("bank").is_empty());
    assert_eq!(share.found_names("keep"), ["keep.pdf"]);
}

/// The indexed prefix range keeps similarly named siblings and another
/// root's copy of the same descendants, including escaped URI characters.
#[test]
fn pruning_keeps_similar_prefixes_and_overlapping_roots() {
    for name in ["old", "r%C3%A9sum%C3%A9", "100%25_", "%5Bdraft%5D"] {
        let share = ScannedShare::new();
        let removed = format!("{SHARE}/{name}");
        let sibling_name = format!("{name}0");
        let sibling = format!("{SHARE}/{sibling_name}");
        let kept = [
            listed_folder(SHARE, &sibling_name),
            listed_file(SHARE, &format!("{name}.txt")),
        ];
        share.store(&[
            listed_folder(SHARE, name),
            listed_file(&removed, "removed.pdf"),
            listed_file(&format!("{removed}/nested"), "nested.pdf"),
            kept[0].clone(),
            kept[1].clone(),
            listed_file(&sibling, "kept.pdf"),
        ]);
        share
            .index
            .configure(&removed, Caching::Enabled, "", HiddenItems::Skip)
            .unwrap();
        let generation = share.index.begin_scan(&removed).unwrap();
        share
            .index
            .store_scanned(&removed, &generation, &[listed_file(&removed, "removed.pdf")])
            .unwrap();

        share.replace(SHARE, &kept);

        assert_eq!(root_state(&share.index, SHARE).entry_count, 3, "{name}");
        assert_eq!(root_state(&share.index, &removed).entry_count, 1, "{name}");
        assert_eq!(share.found_names("kept.pdf"), ["kept.pdf"], "{name}");
        assert!(share.found_names("nested.pdf").is_empty(), "{name}");
    }
}

/// Ported from `v2.0.0:desktop/tests/test_v05.py::IndexTests::test_new_directory_traversal_hint`
///
/// parity: SRCH-028
#[test]
fn a_new_folder_is_returned_for_reading() {
    let share = ScannedShare::new();

    let new_folders = share.replace(SHARE, &[listed_folder(SHARE, "new")]);

    assert_eq!(new_folders, ["smb://nas/share/new"]);
}

/// Ported from `v2.0.0:desktop/tests/test_v05.py::IndexTests::test_failed_scan_preserves_last_data`
///
/// parity: SRCH-024
#[test]
fn a_failed_scan_keeps_the_last_results() {
    let share = ScannedShare::new();
    share.store(&[listed_file(SHARE, "bank.pdf")]);
    let offline = ScanOutcome::Incomplete {
        error: "offline".to_owned(),
    };

    let generation = share.index.begin_scan(SHARE).unwrap();
    share.index.finish_scan(SHARE, &generation, &offline).unwrap();

    assert_eq!(share.found_names("bank"), ["bank.pdf"]);
    let root = root_state(&share.index, SHARE);
    assert_eq!(root.status, RootStatus::Incomplete);
    assert_eq!(root.error.as_deref(), Some("offline"));
}

/// Ported from `v2.0.0:desktop/tests/test_v05.py::IndexTests::test_disable_clears_metadata`
///
/// parity: SRCH-019
#[test]
fn disabling_a_root_deletes_its_entries() {
    let share = ScannedShare::new();
    share.store(&[listed_file(SHARE, "bank.pdf")]);

    share
        .index
        .configure(SHARE, Caching::Disabled, "", HiddenItems::Skip)
        .unwrap();

    assert!(share.found_names("bank").is_empty());
    let root = root_state(&share.index, SHARE);
    assert_eq!(root.status, RootStatus::Disabled);
    assert_eq!(root.entry_count, 0);
}

/// Ported from `v2.0.0:desktop/tests/test_v05.py::IndexTests::test_directory_to_file_prunes_old_children`
///
/// parity: SRCH-028
#[test]
fn a_folder_that_became_a_file_loses_its_children() {
    let share = ScannedShare::new();
    let archive = format!("{SHARE}/archive");
    share.store(&[listed_folder(SHARE, "archive"), listed_file(&archive, "bank.pdf")]);

    share.replace(SHARE, &[listed_file(SHARE, "archive")]);

    assert!(share.found_names("bank").is_empty());
    assert_eq!(share.found_names("archive"), ["archive"]);
}

/// Ported from `v2.0.0:desktop/tests/test_v05.py::IndexTests::test_wrong_parent_ignored`
///
/// parity: SRCH-028
#[test]
fn items_of_another_folder_are_ignored() {
    let share = ScannedShare::new();

    share.replace(SHARE, &[listed_file("smb://other/share", "bank.pdf")]);

    assert!(share.found_names("bank").is_empty());
}

/// Ported from `v2.0.0:desktop/tests/test_v05.py::IndexTests::test_separate_process_connection_reads_updates`
///
/// parity: SRCH-027, PERF-007
#[test]
fn another_connection_reads_what_a_scan_stored() {
    let share = ScannedShare::new();
    let other = SearchIndex::open(share.directory.path()).unwrap();

    share.store(&[listed_file(SHARE, "bank.pdf")]);

    assert_eq!(found_names(&other, "bank"), ["bank.pdf"]);
}

/// Ported from `v2.0.0:desktop/tests/test_v05.py::IndexTests::test_private_database_modes`
///
/// The directory starts out readable by others, so the test also shows
/// that opening the cache makes it private.
///
/// parity: SAFE-009, SAFE-019
#[test]
fn the_database_and_its_directory_are_private() {
    let base = tempfile::tempdir().unwrap();
    let directory = base.path().join("db");
    DirBuilder::new().mode(0o755).create(&directory).unwrap();
    fs::set_permissions(&directory, Permissions::from_mode(0o755)).unwrap();

    let index = SearchIndex::open(&directory).unwrap();

    assert_eq!(permission_bits(index.database_path()), 0o600);
    assert_eq!(permission_bits(index.directory()), 0o700);
}

/// Ported from `v2.0.0:desktop/tests/test_v05.py::IndexTests::test_fts_special_characters_bound`
///
/// parity: SRCH-007, SRCH-008, SRCH-010
#[test]
fn query_syntax_in_a_search_is_plain_text() {
    let share = ScannedShare::new();
    share.store(&[listed_file(SHARE, "name.pdf")]);

    let injected = search(&share.index, "\" OR *; DROP TABLE entries;");

    assert!(injected.is_empty());
    assert_eq!(share.found_names("name"), ["name.pdf"]);
}

/// A batch of a scan that a newer scan replaced stores nothing, and the
/// old scan's end changes nothing, so it cannot prune the newer results.
///
/// parity: SRCH-024
#[test]
fn a_superseded_scan_neither_stores_nor_prunes() {
    let share = ScannedShare::new();
    let newer = share.index.begin_scan(SHARE).unwrap();
    share
        .index
        .store_scanned(SHARE, &newer, &[listed_file(SHARE, "kept.pdf")])
        .unwrap();

    let stored = share
        .index
        .store_scanned(SHARE, &share.generation, &[listed_file(SHARE, "late.pdf")])
        .unwrap();
    share
        .index
        .finish_scan(SHARE, &share.generation, &ScanOutcome::Complete)
        .unwrap();

    assert_eq!(stored, 0);
    assert_eq!(share.found_names("pdf"), ["kept.pdf"]);
    assert_eq!(root_state(&share.index, SHARE).status, RootStatus::Indexing);
}

/// A complete scan prunes what it did not see and records when it
/// finished; the root is "Ready".
///
/// parity: SRCH-024
#[test]
fn a_complete_scan_prunes_what_it_did_not_see() {
    let share = ScannedShare::new();
    share.store(&[listed_file(SHARE, "gone.pdf"), listed_file(SHARE, "kept.pdf")]);
    let rescan = share.index.begin_scan(SHARE).unwrap();

    share
        .index
        .store_scanned(SHARE, &rescan, &[listed_file(SHARE, "kept.pdf")])
        .unwrap();
    share
        .index
        .finish_scan(SHARE, &rescan, &ScanOutcome::Complete)
        .unwrap();

    assert_eq!(share.found_names("pdf"), ["kept.pdf"]);
    let root = root_state(&share.index, SHARE);
    assert_eq!(root.status, RootStatus::Ready);
    assert!(root.updated.is_some());
    assert_eq!(root.entry_count, 1);
}

/// Safety rule "symlinks and virtual items never enter the index": a scan
/// batch and a live update both leave them out, whatever the reader
/// passed.
///
/// parity: SRCH-031, SAFE-019
#[test]
fn links_and_virtual_items_are_never_stored() {
    let share = ScannedShare::new();
    let mut link = listed_folder(SHARE, "link");
    link.is_symlink = true;
    let mut shortcut = listed_folder(SHARE, "shortcut");
    shortcut.is_virtual = true;

    share.store(&[link.clone(), shortcut.clone()]);
    let new_folders = share.replace(SHARE, &[link, shortcut]);

    assert!(new_folders.is_empty());
    assert!(share.found_names("link").is_empty());
    assert!(share.found_names("shortcut").is_empty());
}

/// A cache opened in a directory that is a symlink is refused, so the
/// database never lands where the link points.
///
/// parity: SAFE-009
#[test]
fn a_symlinked_cache_directory_is_refused() {
    let base = tempfile::tempdir().unwrap();
    let target = tempfile::tempdir().unwrap();
    let link = base.path().join("winspace");
    std::os::unix::fs::symlink(target.path(), &link).unwrap();

    let opened = SearchIndex::open(&link);

    assert!(opened.is_err());
    assert!(fs::read_dir(target.path()).unwrap().next().is_none());
}

/// A listing is tied to the state before its I/O began. The comparison
/// belongs inside the write transaction, including for changes made by a
/// second process that cannot cancel this process's worker immediately.
#[test]
fn a_stale_live_listing_cannot_write_after_the_root_changes() {
    for change in ["clear", "rescan", "hidden", "disable", "replace"] {
        let share = ScannedShare::new();
        share.store(&[listed_file(SHARE, "keep.pdf")]);
        let before = root_state(&share.index, SHARE);
        match change {
            "clear" => share.index.clear(SHARE).unwrap(),
            "rescan" => {
                share.index.begin_scan(SHARE).unwrap();
            }
            "hidden" => share
                .index
                .configure(SHARE, Caching::Enabled, "", HiddenItems::Include)
                .unwrap(),
            "disable" => share
                .index
                .configure(SHARE, Caching::Disabled, "", HiddenItems::Skip)
                .unwrap(),
            "replace" => {
                share.index.remove(SHARE).unwrap();
                share
                    .index
                    .configure(SHARE, Caching::Enabled, "", HiddenItems::Skip)
                    .unwrap();
            }
            _ => unreachable!(),
        }
        let expected = share.index.status().unwrap();

        let added = share
            .index
            .replace_folder_contents(
                &before,
                SHARE,
                &[listed_file(SHARE, "stale.pdf")],
                &gio::Cancellable::new(),
            )
            .unwrap();

        assert!(added.is_empty(), "{change}");
        assert!(share.found_names("stale").is_empty(), "{change}");
        assert_eq!(share.index.status().unwrap(), expected, "{change}");
    }
}
