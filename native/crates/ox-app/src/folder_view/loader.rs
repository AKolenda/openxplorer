// SPDX-License-Identifier: AGPL-3.0-only
//! Folder listing.
//!
//! Ports `enumerate_folder` in `desktop/gio_backend.py`. Listing uses
//! GIO's asynchronous enumerator, so the blocking I/O runs on GIO's worker
//! threads and rows arrive in batches: the first batch at once, later ones
//! merged for a quarter second so large folders are not re-sorted per row.
//! Hidden items are listed too; the folder model filters them, so "Show
//! hidden files" needs no reload.
//!
//! Dropping a [`Listing`] cancels it (the GIO futures cancel their
//! `GCancellable` when dropped). Failures are classified by ox-core's
//! [`EnumerateError`], so the window can tell a file from a missing folder
//! or an unmounted share.

use std::time::{Duration, Instant};

use gtk::prelude::*;
use gtk::{gio, glib};
use ox_core::entry::{self, Entry, EnumerateError};

/// Rows in the first batch, shown before the rest of the folder is read.
const FIRST_BATCH: i32 = 64;
/// Rows requested per enumerator call after the first.
const LATER_BATCH: i32 = 512;
/// How long later batches are merged before they are shown.
const MERGE_WINDOW: Duration = Duration::from_millis(250);

/// A running listing. Dropping it cancels the listing.
#[derive(Debug)]
pub(crate) struct Listing {
    handle: Option<glib::JoinHandle<()>>,
}

impl Drop for Listing {
    fn drop(&mut self) {
        if let Some(handle) = self.handle.take() {
            handle.abort();
        }
    }
}

/// Lists `uri`, calling `on_batch` with rows as they arrive and `on_done`
/// once at the end. Neither is called after the [`Listing`] is dropped.
pub(crate) fn list_folder(
    uri: &str,
    on_batch: impl Fn(Vec<Entry>) + 'static,
    on_done: impl FnOnce(Result<(), EnumerateError>) + 'static,
) -> Listing {
    let folder = gio::File::for_uri(uri);
    let handle = glib::spawn_future_local(async move {
        let result = enumerate(&folder, &on_batch).await;
        on_done(result.map_err(|error| EnumerateError::from_glib(&error)));
    });
    Listing { handle: Some(handle) }
}

async fn enumerate(folder: &gio::File, on_batch: &impl Fn(Vec<Entry>)) -> Result<(), glib::Error> {
    let enumerator = folder
        .enumerate_children_future(
            entry::ATTRIBUTES,
            gio::FileQueryInfoFlags::NONE,
            glib::Priority::DEFAULT,
        )
        .await?;
    let mut pending: Vec<Entry> = Vec::new();
    let mut last_flush = Instant::now();
    let mut request = FIRST_BATCH;
    loop {
        let infos = enumerator
            .next_files_future(request, glib::Priority::DEFAULT)
            .await?;
        if infos.is_empty() {
            break;
        }
        let is_first_batch = request == FIRST_BATCH;
        request = LATER_BATCH;
        let entries = infos.iter().map(|info| {
            let child = enumerator.child(info);
            entry::entry_from_info(&child, info)
        });
        pending.extend(entries);
        if is_first_batch || last_flush.elapsed() >= MERGE_WINDOW {
            on_batch(std::mem::take(&mut pending));
            last_flush = Instant::now();
        }
    }
    if !pending.is_empty() {
        on_batch(pending);
    }
    // Closing is best effort; the enumerator is dropped either way.
    let _ = enumerator.close_future(glib::Priority::LOW).await;
    Ok(())
}
