// SPDX-License-Identifier: AGPL-3.0-only
//! Helpers shared by the settings tests: file modes, directory listings
//! and the Quick access order a window shows.

use std::fs;
use std::os::unix::fs::MetadataExt;
use std::path::Path;

/// The Desktop standard folder of the test user.
pub(super) const DESKTOP: &str = "file:///home/test/Desktop";

/// The Downloads standard folder of the test user.
pub(super) const DOWNLOADS: &str = "file:///home/test/Downloads";

/// The Documents standard folder of the test user.
pub(super) const DOCUMENTS: &str = "file:///home/test/Documents";

/// The `quick_order` a window sends when its sidebar shows Desktop,
/// Downloads and Documents, as in `desktop/tests/test_pins.py`.
pub(super) fn shown_quick_order() -> Vec<String> {
    [DESKTOP, DOWNLOADS, DOCUMENTS].map(String::from).to_vec()
}

/// The permission bits of `path`, for example `0o600`.
pub(super) fn mode(path: &Path) -> u32 {
    fs::metadata(path).expect("the path exists").mode() & 0o777
}

/// The sorted names of the files in `directory` that start with `prefix`.
pub(super) fn names_starting_with(directory: &Path, prefix: &str) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(directory)
        .expect("the directory exists")
        .map(|entry| entry.expect("a directory entry").file_name())
        .map(|name| name.to_string_lossy().into_owned())
        .filter(|name| name.starts_with(prefix))
        .collect();
    names.sort();
    names
}
