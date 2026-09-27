// SPDX-License-Identifier: AGPL-3.0-only
//! Folder listing, on the calling thread or on a worker thread.
//!
//! Ports `enumerate_folder` in `desktop/gio_backend.py`: GIO owns backend
//! enumeration (no per-row stat), hidden items are skipped unless asked
//! for, rows are delivered in batches as they arrive, and cancellation is
//! checked before every row. Rows already delivered stay delivered when a
//! later row fails; the unfinished batch is dropped.

mod task;

use gio::prelude::*;

pub use task::EnumerationTask;

use super::{entry_from_info, Entry, EnumerateError, ATTRIBUTES};

/// Rows per batch in the Python backend.
pub const DEFAULT_BATCH_SIZE: usize = 128;

/// What a finished listing reports.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnumerationSummary {
    /// The listed folder, as GIO names it.
    pub uri: String,
    /// Rows delivered, after hidden items were skipped.
    pub count: usize,
}

/// Lists `uri` on the calling thread with GIO's synchronous API.
///
/// `on_batch` receives rows in batches of at most `batch_size` (at least
/// one) as they are read. Cancelling `cancellable` stops the listing before
/// the next row and returns [`EnumerateError::Cancelled`].
///
/// The URI is passed to GIO as given, so virtual folders such as
/// `trash:///` can be listed. Validate addresses typed or pasted by the
/// user with the `location` module first, as the Python app does before
/// every listing.
pub fn enumerate_blocking(
    uri: &str,
    show_hidden: bool,
    cancellable: Option<&gio::Cancellable>,
    batch_size: usize,
    on_batch: &mut dyn FnMut(Vec<Entry>),
) -> Result<EnumerationSummary, EnumerateError> {
    let batch_size = batch_size.max(1);
    let is_cancelled = || cancellable.is_some_and(|c| c.is_cancelled());
    let folder = gio::File::for_uri(uri);
    let enumerator = folder
        .enumerate_children(ATTRIBUTES, gio::FileQueryInfoFlags::NONE, cancellable)
        .map_err(|error| EnumerateError::from_glib(&error))?;

    let mut batch = Vec::with_capacity(batch_size);
    let mut count = 0;
    let outcome = loop {
        if is_cancelled() {
            break Err(EnumerateError::Cancelled);
        }
        let info = match enumerator.next_file(cancellable) {
            Ok(Some(info)) => info,
            Ok(None) => break Ok(()),
            Err(error) => break Err(EnumerateError::from_glib(&error)),
        };
        let hidden = info.boolean("standard::is-hidden");
        if hidden && !show_hidden {
            continue;
        }
        let child = enumerator.child(&info);
        batch.push(entry_from_info(&child, &info));
        count += 1;
        if batch.len() >= batch_size {
            let full = std::mem::replace(&mut batch, Vec::with_capacity(batch_size));
            on_batch(full);
        }
    };
    // Closing can only fail for an already-failed stream; the listing's
    // own outcome is what matters.
    let _ = enumerator.close(gio::Cancellable::NONE);
    outcome?;
    if !batch.is_empty() {
        on_batch(batch);
    }
    Ok(EnumerationSummary {
        uri: folder.uri().to_string(),
        count,
    })
}

/// Lists `uri` on a new worker thread and returns at once.
///
/// `on_batch` runs on the worker thread; forward the rows to the main loop
/// from there (for example over a channel). The returned task is a
/// [`Future`](std::future::Future) that the GTK main loop can await, or
/// can be waited on with [`EnumerationTask::wait`]. Cancel the listing
/// with `cancellable`. See [`enumerate_blocking`] for how `uri` is used.
pub fn enumerate<F>(
    uri: &str,
    show_hidden: bool,
    cancellable: &gio::Cancellable,
    batch_size: usize,
    mut on_batch: F,
) -> EnumerationTask
where
    F: FnMut(Vec<Entry>) + Send + 'static,
{
    let uri = uri.to_string();
    let cancellable = cancellable.clone();
    EnumerationTask::spawn(move || {
        enumerate_blocking(&uri, show_hidden, Some(&cancellable), batch_size, &mut on_batch)
    })
}
