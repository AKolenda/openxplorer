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
    on_done: impl FnOnce(Result<(), EnumerateError>) + 'static,
) -> Listing {
    let folder = gio::File::for_uri(uri);
    let task = glib::spawn_future_local(async move {
        let result = enumerate(&folder, &on_batch).await;
        on_done(result.map_err(|error| EnumerateError::from_glib(&error)));
    });
    Listing { task }
}

/// Reads `folder` to the end, handing its rows to `on_batch` in merged
/// batches. Rows read before a failure that no batch showed yet are
/// dropped with the failure.
async fn enumerate(folder: &gio::File, on_batch: &impl Fn(Vec<Entry>)) -> Result<(), glib::Error> {
    let enumerator = folder
        .enumerate_children_future(
            entry::ATTRIBUTES,
            gio::FileQueryInfoFlags::NONE,
            glib::Priority::DEFAULT,
        )
        .await?;
    let mut merger = BatchMerger::new(on_batch, MERGE_WINDOW);
    let mut request = FIRST_BATCH;
    loop {
        let infos = enumerator
            .next_files_future(request, glib::Priority::DEFAULT)
            .await?;
        if infos.is_empty() {
            break;
        }
        let entries = infos.iter().map(|info| {
            let child = enumerator.child(info);
            entry::entry_from_info(&child, info)
        });
        merger.add(entries);
        request = LATER_BATCH;
    }
    merger.finish();
    // Closing is best effort; the enumerator is dropped either way.
    let _ = enumerator.close_future(glib::Priority::LOW).await;
    Ok(())
}

/// Merges listed rows into batches: the first rows are shown at once and
/// later ones at most once per merge window, so a large folder is not
/// sorted again for every enumerator call.
struct BatchMerger<'a, F> {
    on_batch: &'a F,
    merge_window: Duration,
    /// Rows read but not shown yet.
    pending: Vec<Entry>,
    /// When a batch was last shown; `None` before the first.
    last_shown: Option<Instant>,
}

impl<'a, F: Fn(Vec<Entry>)> BatchMerger<'a, F> {
    fn new(on_batch: &'a F, merge_window: Duration) -> Self {
        Self {
            on_batch,
            merge_window,
            pending: Vec::new(),
            last_shown: None,
        }
    }

    /// Adds rows just read, and shows everything pending if it is the
    /// first batch or the merge window has passed.
    fn add(&mut self, entries: impl IntoIterator<Item = Entry>) {
        self.pending.extend(entries);
        let is_due = self
            .last_shown
            .is_none_or(|shown| shown.elapsed() >= self.merge_window);
        if is_due {
            self.show_pending();
        }
    }

    /// Shows the rows no batch has shown yet, once the listing is complete.
    fn finish(mut self) {
        if !self.pending.is_empty() {
            self.show_pending();
        }
    }

    /// Hands the pending rows on as one batch.
    fn show_pending(&mut self) {
        (self.on_batch)(std::mem::take(&mut self.pending));
        self.last_shown = Some(Instant::now());
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use super::*;
    use crate::test_support::file_entry;

    /// The names in each batch shown when `reads` arrive in order and the
    /// listing then finishes.
    fn batches_shown(merge_window: Duration, reads: Vec<Vec<Entry>>) -> Vec<Vec<String>> {
        let shown = RefCell::new(Vec::new());
        let record = |batch: Vec<Entry>| {
            let names: Vec<String> = batch.into_iter().map(|entry| entry.name).collect();
            shown.borrow_mut().push(names);
        };
        let mut merger = BatchMerger::new(&record, merge_window);
        for read in reads {
            merger.add(read);
        }
        merger.finish();
        shown.into_inner()
    }

    /// parity: PERF-002
    #[test]
    fn the_first_rows_show_at_once_and_later_ones_are_merged() {
        let reads = vec![
            vec![file_entry("a.txt")],
            vec![file_entry("b.txt")],
            vec![file_entry("c.txt")],
        ];
        let batches = batches_shown(Duration::from_secs(3600), reads);
        assert_eq!(batches, [vec!["a.txt"], vec!["b.txt", "c.txt"]]);
    }

    #[test]
    fn rows_are_shown_as_they_arrive_once_the_window_has_passed() {
        let reads = vec![
            vec![file_entry("a.txt")],
            vec![file_entry("b.txt"), file_entry("c.txt")],
        ];
        let batches = batches_shown(Duration::ZERO, reads);
        assert_eq!(batches, [vec!["a.txt"], vec!["b.txt", "c.txt"]]);
    }
}
