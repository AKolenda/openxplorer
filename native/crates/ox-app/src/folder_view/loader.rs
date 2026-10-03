// SPDX-License-Identifier: AGPL-3.0-only
//! Folder listing on the GTK main loop.
//!
//! Ports `enumerate_folder` in `v2.0.0:desktop/gio_backend.py` by running
//! ox-core's [`entry::enumerate_folder`] as a main-loop task. That reads
//! with GIO's asynchronous enumerator, so the blocking I/O runs on GIO's
//! worker threads, and delivers rows in batches: the first batch at once,
//! later ones merged for a quarter second so large folders are not
//! re-sorted per row. Hidden items are listed too; the folder model
//! filters them, so "Show hidden files" needs no reload.
//!
//! Dropping a [`Listing`] cancels it (the GIO futures cancel their
//! `GCancellable` when dropped). Failures are classified by ox-core's
//! [`EntryError`], so the window can tell a file from a missing folder or
//! an unmounted share.

use std::future::Future;
use std::sync::Arc;

use gtk::glib;
use ox_core::archive::{
    default_preview_root, ArchiveBrowser, ArchiveEntryKind, ArchiveError, ArchiveListing, GioArchiveOpener,
};
use ox_core::entry::{self, Entry, EntryError};
use ox_core::location::ArchiveLocation;
use ox_core::location::RECENT_LOCATIONS_URI;
use ox_core::transfer::Cancellation;

use super::recent_locations::list_recent_locations;

/// A running listing. Dropping it cancels the listing.
#[derive(Debug)]
pub(crate) struct Listing {
    /// The listing task, aborted when the listing is dropped.
    task: glib::JoinHandle<()>,
}

impl Drop for Listing {
    fn drop(&mut self) {
        // Dropping a JoinHandle only detaches the task; aborting it is what
        // stops the listing and cancels its GIO futures.
        self.task.abort();
    }
}

impl Listing {
    /// A listing that runs `work` on the main loop, such as mounting a
    /// share before it is listed again. Dropping it cancels `work`.
    pub(crate) fn spawn(work: impl Future<Output = ()> + 'static) -> Self {
        Self {
            task: glib::spawn_future_local(work),
        }
    }
}

/// Lists `uri`, calling `on_batch` with rows as they arrive and `on_done`
/// once at the end. Recent locations are listed from the desktop's
/// recently used list (SIDE-026). Neither is called after the [`Listing`] is dropped.
pub(crate) fn list_folder(
    uri: &str,
    on_batch: impl Fn(Vec<Entry>) + 'static,
    on_done: impl FnOnce(Result<(), EntryError>) + 'static,
) -> Listing {
    let uri = uri.to_owned();
    Listing::spawn(async move {
        let result = if uri == RECENT_LOCATIONS_URI {
            list_recent_locations(on_batch).await
        } else {
            entry::enumerate_folder(&uri, on_batch).await
        };
        on_done(result);
    })
}

/// Cancels the archive reader's work when the listing that owns it is
/// dropped (the reader runs on a worker thread, which aborting the task
/// alone would not stop).
struct CancelOnDrop(Cancellation);

impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        self.0.cancel();
    }
}

/// Lists the folder `location` inside a ZIP (ARC-026), as one batch of
/// read-only rows, then calls `on_done`. The archive reader applies its
/// usual rules: unsafe names and links are left out, and at most 5,000
/// rows are listed.
pub(crate) fn list_archive_folder(
    location: &ArchiveLocation,
    on_batch: impl Fn(Vec<Entry>) + 'static,
    on_done: impl FnOnce(Result<(), EntryError>) + 'static,
) -> Listing {
    let location = location.clone();
    Listing::spawn(async move {
        let browser = ArchiveBrowser::new(Arc::new(GioArchiveOpener), default_preview_root());
        let guard = CancelOnDrop(Cancellation::new());
        match archive_listing(&browser, &location, &guard.0).await {
            Ok(listing) => {
                let rows = listing
                    .entries
                    .into_iter()
                    .map(|member| {
                        let (is_dir, size) = match member.kind {
                            ArchiveEntryKind::Folder => (true, None),
                            ArchiveEntryKind::File { size, .. } => (false, Some(size)),
                        };
                        let uri = location.member(&member.member).uri();
                        Entry::archive_member(uri, member.name, is_dir, size, member.modified)
                    })
                    .collect();
                on_batch(rows);
                on_done(Ok(()));
            }
            Err(error) => on_done(Err(error)),
        }
        drop(guard);
    })
}

/// The listing of the folder `location`, or [`EntryError::NotDirectory`]
/// when it names a file: a file's own location, or a path typed through
/// the ZIP (which always ends in `/`) to a file. A folder typed without
/// its `/` is listed as the folder.
async fn archive_listing(
    browser: &ArchiveBrowser,
    location: &ArchiveLocation,
    cancel: &Cancellation,
) -> Result<ArchiveListing, EntryError> {
    let archive = location.archive_uri.clone();
    let list = |prefix: String| browser.list_in_background(archive.clone(), prefix, cancel.clone());
    let failure = |error: ArchiveError| match error {
        ArchiveError::Cancelled => EntryError::Cancelled,
        error => EntryError::Failed(error.to_string()),
    };
    let Some(parent) = location.parent() else {
        return list(String::new()).await.map_err(failure);
    };
    let name = location
        .segments()
        .last()
        .map(|name| (*name).to_owned())
        .unwrap_or_default();
    if location.is_folder() {
        let listing = list(location.member.clone()).await.map_err(failure)?;
        if !listing.entries.is_empty() {
            return Ok(listing);
        }
        let siblings = list(parent.member).await.map_err(failure)?;
        let is_file = siblings
            .entries
            .iter()
            .any(|entry| entry.name == name && matches!(entry.kind, ArchiveEntryKind::File { .. }));
        return if is_file {
            Err(EntryError::NotDirectory(name))
        } else {
            Ok(listing)
        };
    }
    let siblings = list(parent.member).await.map_err(failure)?;
    match siblings
        .entries
        .iter()
        .find(|entry| entry.name == name)
        .map(|entry| entry.kind)
    {
        Some(ArchiveEntryKind::Folder) => list(format!("{}/", location.member)).await.map_err(failure),
        Some(ArchiveEntryKind::File { .. }) => Err(EntryError::NotDirectory(name)),
        None => Err(EntryError::NotFound(name)),
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::rc::Rc;

    use super::*;
    use crate::test_support::harness::{wait_until, Fixture};

    /// What a listing delivered.
    #[derive(Debug, Default)]
    struct Delivered {
        batches: Cell<u32>,
        done: Cell<bool>,
    }

    /// Lists `uri`, counting into `delivered`.
    fn counted_listing(uri: &str, delivered: &Rc<Delivered>) -> Listing {
        let batches = Rc::clone(delivered);
        let done = Rc::clone(delivered);
        list_folder(
            uri,
            move |_| batches.batches.set(batches.batches.get() + 1),
            move |_| done.done.set(true),
        )
    }

    /// A superseded listing is dropped, and nothing of it arrives, not even
    /// while a listing of the same folder runs to its end.
    ///
    /// parity: NAV-016
    #[gtk::test]
    fn a_dropped_listing_delivers_no_rows_and_no_end() {
        let fixture = Fixture::with_files(300);
        let dropped = Rc::new(Delivered::default());
        let kept = Rc::new(Delivered::default());

        drop(counted_listing(&fixture.uri(), &dropped));
        let _listing = counted_listing(&fixture.uri(), &kept);
        wait_until("the kept listing to end", || kept.done.get());

        assert!(kept.batches.get() > 0);
        assert_eq!(dropped.batches.get(), 0);
        assert!(!dropped.done.get());
    }
}
