// SPDX-License-Identifier: AGPL-3.0-only
//! Live updates of local folders through the index service's public API.
//!
//! Ports `desktop/tests/test_v05.py::LiveTests` on real local folders with
//! real inotify events, as the app uses the service.
//! `test_watch_limit_fallback_is_reported` and `test_root_exclusions` need
//! the crate's internals and are ported in `src/search`.

mod search_support;

use std::fs;
use std::os::unix::fs::symlink;
use std::time::Duration;

use ox_core::search::{
    AutoIndex, GioFolderReader, HiddenItems, IndexService, IndexSettings, RootStatus, SearchIndex,
    SearchQuery, UpdateMode,
};
use search_support::{tick_for, tick_until, IndexedFolder};

/// Ported from `desktop/tests/test_v05.py::LiveTests::test_live_create_without_full_scan`
///
/// parity: SRCH-028
#[test]
fn a_new_file_is_indexed_without_a_full_scan() {
    let folder = IndexedFolder::new();
    let generation = folder.state().generation;

    fs::write(folder.tree.join("live-bank.pdf"), "test").unwrap();

    folder.tick_until("the new file is found", || {
        folder.found_names("live-bank").len() == 1
    });
    assert_eq!(folder.state().generation, generation);
}

/// Ported from `desktop/tests/test_v05.py::LiveTests::test_new_nested_folder_watched`
///
/// parity: SRCH-028
#[test]
fn a_new_nested_folder_is_watched() {
    let folder = IndexedFolder::new();
    let new = folder.tree.join("new");
    fs::create_dir(&new).unwrap();
    fs::write(new.join("first.txt"), "one").unwrap();
    folder.tick_until("first.txt is found", || {
        folder.found_names("first.txt").len() == 1
    });

    fs::write(new.join("second.txt"), "two").unwrap();

    folder.tick_until("second.txt is found", || {
        folder.found_names("second.txt").len() == 1
    });
    folder.tick_until("both folders are watched", || folder.state().watch_count >= 2);
}

/// Ported from `desktop/tests/test_v05.py::LiveTests::test_live_rename_and_delete`
///
/// parity: SRCH-028
#[test]
fn renames_and_deletions_update_the_cache() {
    let folder = IndexedFolder::new();
    let before = folder.tree.join("before.pdf");
    let after = folder.tree.join("after.pdf");
    fs::write(&before, "test").unwrap();
    folder.tick_until("before.pdf is found", || {
        folder.found_names("before.pdf").len() == 1
    });

    fs::rename(&before, &after).unwrap();
    folder.tick_until("the rename is seen", || {
        folder.found_names("after.pdf").len() == 1 && folder.found_names("before.pdf").is_empty()
    });
    fs::remove_file(&after).unwrap();

    folder.tick_until("the deletion is seen", || {
        folder.found_names("after.pdf").is_empty()
    });
}

/// Ported from `desktop/tests/test_v05.py::LiveTests::test_hidden_and_symlinks_not_traversed`
///
/// parity: SRCH-031, SAFE-019
#[test]
fn hidden_items_and_symlinks_are_not_indexed() {
    let folder = IndexedFolder::new();
    fs::write(folder.tree.join(".hidden"), "secret").unwrap();
    symlink(folder.base.path(), folder.tree.join("link")).unwrap();

    tick_for(
        &folder.service,
        &IndexSettings::default(),
        Duration::from_millis(600),
    );

    assert!(folder.found_names("hidden").is_empty());
    assert!(folder.found_names("link").is_empty());
}

/// Ported from `desktop/tests/test_v05.py::LiveTests::test_two_window_index_leader`
///
/// The second window does not scan; the owner runs the scan it asked for,
/// which gives the root a new generation.
///
/// parity: SRCH-027
#[test]
fn a_second_window_passes_its_refresh_to_the_owner() {
    let folder = IndexedFolder::new();
    let index = SearchIndex::open(&folder.base.path().join("index")).unwrap();
    let other = IndexService::start(index, GioFolderReader, || {}).unwrap();
    let generation = folder.state().generation;

    assert!(!other.is_owner());
    other.refresh(&folder.root).unwrap();

    folder.tick_until("the owner rescans", || {
        let root = folder.state();
        root.status == RootStatus::Ready && root.generation != generation
    });
}

/// Ported from `desktop/tests/test_v05.py::LiveTests::test_snapshot_folders_not_indexed`
///
/// parity: SRCH-031
#[test]
fn snapshot_folders_are_not_indexed() {
    let folder = IndexedFolder::new();
    let snapshot = folder.tree.join(".snapshot");
    fs::create_dir(&snapshot).unwrap();
    fs::write(snapshot.join("secret.pdf"), "old").unwrap();

    tick_for(
        &folder.service,
        &IndexSettings::default(),
        Duration::from_millis(600),
    );

    assert!(folder.found_names("secret.pdf").is_empty());
}

/// `.snapshot` is hidden, so the ported test passes on the hidden rule
/// alone; with hidden items included, the snapshot rule still keeps
/// snapshot history out.
///
/// parity: SRCH-031
#[test]
fn snapshot_folders_are_not_indexed_even_with_hidden_items() {
    let folder = IndexedFolder::with_hidden_items(HiddenItems::Include);
    fs::write(folder.tree.join(".visible-when-hidden-shown"), "x").unwrap();
    let snapshots = folder.tree.join(".snapshots");
    fs::create_dir(&snapshots).unwrap();
    fs::write(snapshots.join("secret.pdf"), "old").unwrap();

    folder.tick_until("the hidden file is found", || {
        let query = SearchQuery {
            hidden_items: HiddenItems::Include,
            ..SearchQuery::new("visible-when")
        };
        let results = folder.service.index().search(&query, None).unwrap();
        results.hits.len() == 1
    });
    tick_for(
        &folder.service,
        &IndexSettings::default(),
        Duration::from_millis(600),
    );

    assert!(folder.found_names("secret.pdf").is_empty());
}

/// Pausing Auto-index removes the watches and ignores changes; switching
/// it back on rescans every root.
///
/// parity: SRCH-026
#[test]
fn a_paused_auto_index_catches_up_when_switched_back_on() {
    let folder = IndexedFolder::new();
    let paused = IndexSettings {
        auto_index: AutoIndex::Paused,
        ..IndexSettings::default()
    };
    tick_until(&folder.service, &paused, "the root reports the pause", || {
        folder.state().update_mode == UpdateMode::Paused
    });
    assert_eq!(folder.state().watch_count, 0);

    fs::write(folder.tree.join("while-paused.pdf"), "x").unwrap();
    tick_for(&folder.service, &paused, Duration::from_millis(600));
    assert!(folder.found_names("while-paused").is_empty());

    folder.tick_until("the rescan finds the file", || {
        folder.found_names("while-paused").len() == 1
    });
    folder.tick_until("live events are back", || {
        folder.state().update_mode == UpdateMode::LiveLocalEvents
    });
}

/// parity: SRCH-023
#[test]
fn refresh_all_rescans_every_enabled_root() {
    let folder = IndexedFolder::new();
    let generation = folder.state().generation;

    folder.service.refresh_all().unwrap();

    folder.tick_until("the root is rescanned", || {
        let root = folder.state();
        root.status == RootStatus::Ready && root.generation != generation
    });
}
