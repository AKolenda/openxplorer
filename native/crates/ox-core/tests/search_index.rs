// SPDX-License-Identifier: AGPL-3.0-only
//! The search cache's private storage through its public API.
//!
//! Ports the search database tests of
//! `v2.0.0:desktop/tests/test_terminal_security.py::PrivateStorageTests`: a
//! database or sidecar replaced by a symlink is refused, and the file the
//! link points to is never changed.

use std::fs;
use std::os::unix::fs::symlink;
use std::path::Path;

use ox_core::search::{SearchEngine, SearchError, SearchIndex};

/// A file outside the cache that a planted symlink points to.
fn link_target(directory: &Path) -> std::path::PathBuf {
    let target = directory.join("target");
    fs::write(&target, b"unchanged").expect("the link target is written");
    target
}

/// Ported from `v2.0.0:desktop/tests/test_terminal_security.py::PrivateStorageTests::test_database_symlink_refused`
///
/// parity: SAFE-009
#[test]
fn a_symlinked_database_is_refused() {
    let root = tempfile::tempdir().unwrap();
    let directory = root.path().join("cache");
    fs::create_dir(&directory).unwrap();
    let target = link_target(root.path());
    symlink(&target, directory.join("search.sqlite3")).unwrap();

    let opened = SearchIndex::open(&directory);

    assert!(matches!(opened, Err(SearchError::Io { .. })));
    assert_eq!(fs::read(&target).unwrap(), b"unchanged");
}

/// Ported from `v2.0.0:desktop/tests/test_terminal_security.py::PrivateStorageTests::test_database_sidecar_symlink_refused`
///
/// parity: SAFE-009
#[test]
fn a_symlinked_database_sidecar_is_refused() {
    let root = tempfile::tempdir().unwrap();
    let directory = root.path().join("cache");
    fs::create_dir(&directory).unwrap();
    let target = link_target(root.path());
    symlink(&target, directory.join("search.sqlite3-wal")).unwrap();

    let opened = SearchIndex::open(&directory);

    assert!(matches!(opened, Err(SearchError::Io { .. })));
    assert_eq!(fs::read(&target).unwrap(), b"unchanged");
}

/// Every use of the cache checks its files again, so a symlink planted
/// after the cache was opened is refused before SQLite follows it.
///
/// parity: SAFE-009, SAFE-019
#[test]
fn a_symlink_planted_after_opening_is_refused() {
    let root = tempfile::tempdir().unwrap();
    let directory = root.path().join("cache");
    let index = SearchIndex::open(&directory).unwrap();
    let target = link_target(root.path());
    let journal = directory.join("search.sqlite3-journal");
    symlink(&target, &journal).unwrap();

    let status = index.status();

    assert!(matches!(status, Err(SearchError::Io { .. })));
    assert_eq!(fs::read(&target).unwrap(), b"unchanged");
}

/// A new cache lists no roots and searches with the trigram index; it
/// holds names and metadata only, in `search.sqlite3`.
///
/// parity: SRCH-022
#[test]
fn a_new_cache_is_empty() {
    let root = tempfile::tempdir().unwrap();

    let index = SearchIndex::open(root.path()).unwrap();
    let status = index.status().unwrap();

    assert!(status.roots.is_empty());
    assert_eq!(status.entry_count, 0);
    assert_eq!(status.engine, SearchEngine::Trigram);
    assert_eq!(status.database, root.path().join("search.sqlite3"));
}
