// SPDX-License-Identifier: AGPL-3.0-only
//! Shared by the `sizes_*` integration tests: a temporary folder to scan,
//! the scan the app runs on it, simulated item metadata, and a
//! [`TestProvider`] that stands in for the `LocalSizeProvider` subclasses
//! of `v2.0.0:desktop/tests/test_v06.py`.
#![allow(
    dead_code,
    reason = "each test crate that includes this module uses a different part of it"
)]

use std::fs;
use std::ops::ControlFlow;
use std::path::{Path, PathBuf};

use ox_core::entry::EntryError;
use ox_core::location::file_uri;
use ox_core::sizes::{
    scan_folder_size, FolderSize, FolderSizeScan, LocalSizeProvider, PartialReason, ScanStatus, SizeEntry,
    SizeEntryKind, SizeError, SizeProvider,
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
    /// A new, empty temporary folder.
    pub fn new() -> Self {
        Self {
            temporary: tempfile::tempdir().expect("a temporary folder"),
        }
    }

    /// The folder's path.
    pub fn path(&self) -> &Path {
        self.temporary.path()
    }

    /// The folder's `file://` URI.
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

/// How a [`TestProvider`] lists folders. Each variant stands in for one of
/// the `LocalSizeProvider` subclasses of the Python tests.
#[derive(Debug)]
pub enum Listing {
    /// The real listing.
    Real,
    /// The real listing, cancelling the scan once this many items were
    /// passed on.
    CancelAfter(usize),
    /// These items, whatever the folder.
    Only(Vec<SizeEntry>),
    /// The real listing, except that the folder whose URI ends with
    /// `suffix` fails with `error`.
    FailFolder { suffix: &'static str, error: EntryError },
    /// Every listing fails with this error.
    FailAlways(EntryError),
}

/// Reads through a real [`LocalSizeProvider`], except where `listing` or
/// `inspect_error` say otherwise.
#[derive(Debug)]
pub struct TestProvider {
    local: LocalSizeProvider,
    listing: Listing,
    /// Returned instead of the scanned folder's metadata.
    inspect_error: Option<EntryError>,
}

impl TestProvider {
    /// A provider that lists folders as `listing` says.
    pub fn listing(listing: Listing) -> Self {
        Self {
            local: LocalSizeProvider::new().expect("the mount table is readable"),
            listing,
            inspect_error: None,
        }
    }

    /// A provider that fails with `error` when the scanned folder itself
    /// is inspected.
    pub fn failing_inspect(error: EntryError) -> Self {
        Self {
            inspect_error: Some(error),
            ..Self::listing(Listing::Real)
        }
    }

    /// The real listing of `folder_uri`, cancelling the scan once `limit`
    /// items were passed on.
    fn visit_then_cancel(
        &self,
        folder_uri: &str,
        cancel: &Cancellation,
        visit: &mut dyn FnMut(SizeEntry) -> ControlFlow<()>,
        limit: usize,
    ) -> Result<(), EntryError> {
        let mut passed_on = 0;
        self.local.visit_children(folder_uri, cancel, &mut |entry| {
            let flow = visit(entry);
            passed_on += 1;
            if passed_on == limit {
                cancel.cancel();
            }
            flow
        })
    }
}

impl SizeProvider for TestProvider {
    fn inspect(&self, uri: &str, cancel: &Cancellation) -> Result<SizeEntry, EntryError> {
        match &self.inspect_error {
            Some(error) => Err(error.clone()),
            None => self.local.inspect(uri, cancel),
        }
    }

    fn visit_children(
        &self,
        folder_uri: &str,
        cancel: &Cancellation,
        visit: &mut dyn FnMut(SizeEntry) -> ControlFlow<()>,
    ) -> Result<(), EntryError> {
        match &self.listing {
            Listing::Real => self.local.visit_children(folder_uri, cancel, visit),
            Listing::CancelAfter(limit) => self.visit_then_cancel(folder_uri, cancel, visit, *limit),
            Listing::Only(entries) => {
                visit_each(entries, visit);
                Ok(())
            }
            Listing::FailFolder { suffix, error } => {
                if folder_uri.ends_with(suffix) {
                    return Err(error.clone());
                }
                self.local.visit_children(folder_uri, cancel, visit)
            }
            Listing::FailAlways(error) => Err(error.clone()),
        }
    }
}
