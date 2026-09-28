// SPDX-License-Identifier: AGPL-3.0-only
//! Folder listing on the GTK main loop.
//!
//! Ports `enumerate_folder` in `desktop/gio_backend.py` by running
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

use gtk::glib;
use ox_core::entry::{self, Entry, EntryError};

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

/// Lists `uri`, calling `on_batch` with rows as they arrive and `on_done`
/// once at the end. Neither is called after the [`Listing`] is dropped.
pub(crate) fn list_folder(
    uri: &str,
    on_batch: impl Fn(Vec<Entry>) + 'static,
    on_done: impl FnOnce(Result<(), EntryError>) + 'static,
) -> Listing {
    let uri = uri.to_owned();
    let task = glib::spawn_future_local(async move {
        let result = entry::enumerate_folder(&uri, on_batch).await;
        on_done(result);
    });
    Listing { task }
}
