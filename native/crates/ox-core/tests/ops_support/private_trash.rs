// SPDX-License-Identifier: AGPL-3.0-only
//! The safety check of every test that moves items to the Trash or reads,
//! restores or empties the Recycle Bin.
//!
//! These tests use the real `trash:///` backend of `GVfs` (gvfsd-trash).
//! `native/tools/check.py` runs every test binary with a private
//! `XDG_DATA_HOME` in a temporary folder, so the home Trash these tests
//! fill and empty is the test run's own; a test that finds any other data
//! folder stops before it changes anything.
//!
//! `trash:///` also lists the Trash of every other mounted volume (its
//! `.Trash-$UID` folder), which no environment variable redirects: items
//! the user trashed on a USB drive appear there too. So the tests restore
//! and delete only items they trashed themselves, and the one test that
//! empties the Recycle Bin first checks that every listed item came from
//! the test run's temporary folders, and afterwards that it deleted
//! exactly those. An item trashed on another volume in the moment between
//! that check and the emptying would still be deleted: that window cannot
//! be closed while the real Empty Recycle Bin is tested.

use std::path::PathBuf;

use gio::prelude::*;

/// Panics unless the Recycle Bin is the private one of this test run and
/// GIO can list it.
pub fn require_private_trash() {
    let data_home = std::env::var_os("XDG_DATA_HOME").map(PathBuf::from);
    let temp = std::env::temp_dir();
    let is_private = data_home.as_ref().is_some_and(|folder| folder.starts_with(&temp));
    assert!(
        is_private,
        "Recycle Bin tests only run with a private XDG_DATA_HOME below {}, as native/tools/check.py \
         sets it, so they never touch the user's own Recycle Bin; found {data_home:?}",
        temp.display()
    );
    assert_eq!(
        Some(glib::user_data_dir()),
        data_home,
        "GLib must use the private data folder"
    );
    let schemes = gio::Vfs::default().supported_uri_schemes();
    assert!(
        schemes.iter().any(|scheme| scheme == "trash"),
        "this check needs GVfs with its Trash backend (gvfs); found {schemes:?}"
    );
}
