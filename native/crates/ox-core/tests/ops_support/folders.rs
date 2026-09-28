// SPDX-License-Identifier: AGPL-3.0-only
//! A source and a destination folder for the `ops_*` tests that copy or
//! move items between two folders.

use std::fs;
use std::path::PathBuf;

/// A source folder `src` and a destination folder `dst` in a temporary
/// folder, which is removed with it.
pub struct Folders {
    temp: tempfile::TempDir,
}

impl Folders {
    /// Both folders, empty.
    pub fn new() -> Self {
        let temp = tempfile::tempdir().expect("a temporary folder");
        fs::create_dir(temp.path().join("src")).expect("the source folder");
        fs::create_dir(temp.path().join("dst")).expect("the destination folder");
        Self { temp }
    }

    /// The source folder.
    pub fn source(&self) -> PathBuf {
        self.temp.path().join("src")
    }

    /// The destination folder.
    pub fn destination(&self) -> PathBuf {
        self.temp.path().join("dst")
    }
}
