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

use std::future::Future;

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
/// once at the end. Neither is called after the [`Listing`] is dropped.
pub(crate) fn list_folder(
    uri: &str,
    on_batch: impl Fn(Vec<Entry>) + 'static,
    on_done: impl FnOnce(Result<(), EntryError>) + 'static,
) -> Listing {
    let uri = uri.to_owned();
    Listing::spawn(async move {
        let result = entry::enumerate_folder(&uri, on_batch).await;
        on_done(result);
    })
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
