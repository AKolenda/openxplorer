// SPDX-License-Identifier: AGPL-3.0-only
//! Folder sizes of network shares through GIO metadata (PROP-030), and the
//! background scan the window awaits (PROP-026, PROP-029).
//!
//! `v2.0.0:desktop/tests` has no test of `GioSizeProvider`: shares are simulated
//! in memory here, and the GIO provider is checked on a real local folder
//! against the `lstat` provider. Every file is inside a temporary
//! directory.

mod sizes_support;

use std::collections::HashMap;
use std::ops::ControlFlow;
use std::os::unix::fs::symlink;
use std::sync::mpsc;

use ox_core::entry::EntryError;
use ox_core::sizes::{
    scan_folder_size_in_background, FolderSize, FolderSizeScan, GioSizeProvider, ScanStatus, SizeEntry,
    SizeEntryKind, SizeError, SizeProvider,
};
use ox_core::transfer::Cancellation;
use sizes_support::{simulated_entry, visit_each, Folder, EXCLUDED};

/// The filesystem id of the simulated share.
const SHARE_FILESYSTEM: &str = "smb-share:server=nas,share=work";

/// A share simulated in memory: the items of each folder, by folder URI.
#[derive(Debug, Default)]
struct SimulatedShare {
    folders: HashMap<String, Vec<SizeEntry>>,
}

impl SimulatedShare {
    /// Adds the folder `uri` on `filesystem` to the listing of `parent`.
    fn add_folder(&mut self, parent: &str, uri: &str, filesystem: &str) {
        let folder = SizeEntry {
            filesystem: Some(filesystem.into()),
            ..simulated_entry(uri, SizeEntryKind::Folder)
        };
        self.folders.entry(parent.into()).or_default().push(folder);
        self.folders.entry(uri.into()).or_default();
    }

    /// Adds a file of `size` bytes at `uri` to the listing of `parent`.
    fn add_file(&mut self, parent: &str, uri: &str, size: u64) {
        let file = SizeEntry {
            size: Some(size),
            filesystem: Some(SHARE_FILESYSTEM.into()),
            ..simulated_entry(uri, SizeEntryKind::File)
        };
        self.folders.entry(parent.into()).or_default().push(file);
    }
}

impl SizeProvider for SimulatedShare {
    fn inspect(&self, uri: &str, _: &Cancellation) -> Result<SizeEntry, EntryError> {
        Ok(SizeEntry {
            filesystem: Some(SHARE_FILESYSTEM.into()),
            ..simulated_entry(uri, SizeEntryKind::Folder)
        })
    }

    fn visit_children(
        &self,
        folder_uri: &str,
        _: &Cancellation,
        visit: &mut dyn FnMut(SizeEntry) -> ControlFlow<()>,
    ) -> Result<(), EntryError> {
        let Some(entries) = self.folders.get(folder_uri) else {
            return Err(EntryError::NotFound(folder_uri.into()));
        };
        visit_each(entries, visit);
        Ok(())
    }
}

/// parity: PROP-030, PROP-028
#[test]
fn a_subfolder_on_another_filesystem_of_a_share_is_skipped() {
    let mut share = SimulatedShare::default();
    share.add_file("smb://nas/work", "smb://nas/work/a.txt", 3);
    share.add_folder("smb://nas/work", "smb://nas/work/sub", SHARE_FILESYSTEM);
    share.add_file("smb://nas/work/sub", "smb://nas/work/sub/b.txt", 4);
    share.add_folder("smb://nas/work", "smb://nas/work/mounted", "another share");
    share.add_file("smb://nas/work/mounted", "smb://nas/work/mounted/c.txt", 100);

    let size = FolderSizeScan::new(&share)
        .run("smb://nas/work", &Cancellation::new(), |_| {})
        .unwrap();

    assert_eq!((size.bytes, size.files, size.folders), (7, 2, 1));
    assert_eq!((size.skipped, size.status), (1, EXCLUDED));
}

/// GIO reads the same totals from a local folder as `lstat` does, without
/// following links. GIO reports no inode, so the tree has no hard link.
///
/// parity: PROP-030, PROP-028
#[test]
fn gio_metadata_gives_the_same_totals_without_following_links() {
    let folder = Folder::new();
    let actual = folder.write("a.txt", b"abc");
    folder.write("sub/b.txt", b"12345");
    symlink(&actual, folder.path().join("link")).unwrap();
    symlink(folder.path(), folder.path().join("sub/loop")).unwrap();

    let through_gio = folder.scan_through(&GioSizeProvider).unwrap();
    let through_lstat = folder.scan();

    let totals = |size: &FolderSize| (size.bytes, size.files, size.folders, size.skipped, size.status);
    assert_eq!(totals(&through_gio), (8, 2, 1, 2, EXCLUDED));
    assert_eq!(totals(&through_gio), totals(&through_lstat));
}

/// parity: PROP-026, PROP-029
#[test]
fn a_background_scan_publishes_progress_and_returns_the_totals() {
    let folder = Folder::new();
    folder.write("a", b"abc");
    folder.write("sub/b", b"12345");
    let (sender, published) = mpsc::channel();

    let scan = scan_folder_size_in_background(folder.uri(), Cancellation::new(), move |size| {
        // The receiver outlives the scan, so the send cannot fail.
        sender.send(size.status).expect("the test is still listening");
    });
    let size = glib::MainContext::new().block_on(scan).unwrap();

    let statuses: Vec<ScanStatus> = published.iter().collect();
    assert_eq!(statuses.first(), Some(&ScanStatus::Scanning));
    assert_eq!(statuses.last(), Some(&ScanStatus::Complete));
    assert_eq!((size.bytes, size.files, size.folders), (8, 2, 1));
}

/// Cancelled before the scanned folder was read, a scan returns the
/// cancellation instead of a total, as `cancel.check()` in Python's
/// `LocalSizeProvider.inspect` does.
///
/// parity: PROP-029
#[test]
fn a_background_scan_cancelled_before_it_starts_returns_the_cancellation() {
    let folder = Folder::new();
    folder.write("a", b"abc");
    let cancel = Cancellation::new();
    cancel.cancel();

    let scan = scan_folder_size_in_background(folder.uri(), cancel, |_| {});
    let outcome = glib::MainContext::new().block_on(scan);

    assert_eq!(outcome, Err(SizeError::Read(EntryError::Cancelled)));
}
