// SPDX-License-Identifier: AGPL-3.0-only
//! Shared by the `sizes_*` integration tests: a temporary folder to scan,
//! the scan the app runs on it, and simulated item metadata.
#![allow(
    dead_code,
    reason = "each test crate that includes this module uses a different part of it"
)]

use std::fs;
use std::ops::ControlFlow;
use std::path::{Path, PathBuf};

use ox_core::location::file_uri;
use ox_core::sizes::{
    scan_folder_size, FolderSize, FolderSizeScan, PartialReason, ScanStatus, SizeEntry, SizeEntryKind,
    SizeError, SizeProvider,
};
use ox_core::transfer::Cancellation;
use tempfile::TempDir;

/// The partial status of a scan that left links, mounts, snapshot
/// collections or unreadable items out.
pub const EXCLUDED: ScanStatus = ScanStatus::Partial(PartialReason::EntriesExcluded);

/// A temporary folder to scan, removed when dropped.
pub struct Folder {
    temporary: TempDir,
}

impl Folder {
    pub fn new() -> Self {
        Self {
            temporary: tempfile::tempdir().expect("a temporary folder"),
        }
    }

    pub fn path(&self) -> &Path {
        self.temporary.path()
    }

    pub fn uri(&self) -> String {
        file_uri(self.path())
    }

    /// Writes `contents` to `name` below the folder, creating the folders
    /// on the way, and returns its path.
    pub fn write(&self, name: &str, contents: &[u8]) -> PathBuf {
        let path = self.path().join(name);
        let parent = path.parent().expect("a file below the folder has a parent");
        fs::create_dir_all(parent).expect("create the parent folders");
        fs::write(&path, contents).expect("write the file");
        path
    }

    /// Scans the folder as the app does, with the provider its location
    /// needs.
    pub fn scan(&self) -> FolderSize {
        scan(&self.uri()).expect("the scan starts")
    }

    /// Scans the folder through `provider`.
    pub fn scan_through(&self, provider: &dyn SizeProvider) -> Result<FolderSize, SizeError> {
        FolderSizeScan::new(provider).run(&self.uri(), &Cancellation::new(), |_| {})
    }
}

/// Scans `uri` as the app does, ignoring progress.
pub fn scan(uri: &str) -> Result<FolderSize, SizeError> {
    scan_folder_size(uri, &Cancellation::new(), |_| {})
}

/// Simulated metadata of the item at `uri`: its name is the last path
/// component, and nothing else is known. Tests fill in the rest with
/// struct update syntax.
pub fn simulated_entry(uri: &str, kind: SizeEntryKind) -> SizeEntry {
    let name = uri.rsplit('/').next().expect("split always yields a component");
    SizeEntry {
        uri: uri.into(),
        name: name.into(),
        kind,
        size: None,
        filesystem: None,
        identity: None,
        is_mount_point: false,
    }
}

/// Passes each of `entries` to `visit`, as a provider's listing does,
/// until `visit` breaks.
pub fn visit_each(entries: &[SizeEntry], visit: &mut dyn FnMut(SizeEntry) -> ControlFlow<()>) {
    for entry in entries {
        if visit(entry.clone()).is_break() {
            return;
        }
    }
}
