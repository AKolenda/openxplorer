// SPDX-License-Identifier: AGPL-3.0-only
//! Folder listing with GIO's asynchronous enumerator.
//!
//! Ports `enumerate_folder` in `v2.0.0:desktop/gio_backend.py`. GIO owns backend
//! enumeration (no per-row stat) and does the blocking reads on its own
//! worker threads, so the listing can be awaited on the GTK main loop. Rows
//! arrive in batches: the first one at once, so the top of a large folder
//! shows quickly, and later ones merged for a quarter of a second, so the
//! view is not re-sorted for every read.
//!
//! Unlike the Python backend, hidden items are listed too, with
//! [`Entry::is_hidden`] set: the view filters them, so toggling Show hidden
//! files does not read the folder again.

use std::time::{Duration, Instant};

use gio::prelude::*;

use super::{entry_from_info, Entry, EntryError, ATTRIBUTES};

/// Rows in the first batch, shown before the rest of the folder is read.
const FIRST_BATCH_SIZE: i32 = 64;
/// Rows requested per enumerator call after the first.
const LATER_BATCH_SIZE: i32 = 512;
/// How long later batches are merged before they are delivered.
const MERGE_WINDOW: Duration = Duration::from_millis(250);

/// Lists the folder at `uri`, passing its rows to `on_batch` as they are
/// read.
///
/// Dropping the future cancels the listing: GIO's pending read is
/// cancelled and `on_batch` is not called again. If reading fails part-way,
/// the rows already delivered stay delivered and rows still being merged
/// are dropped.
///
/// The URI is passed to GIO as given, so virtual folders such as
/// `trash:///` can be listed. Validate addresses typed or pasted by the
/// user with the `location` module first, as the Python app does before
/// every listing.
///
/// # Errors
///
/// The GIO failure, sorted by [`EntryError`]. When it is
/// [`EntryError::NotMounted`], mount the location and list it again.
pub async fn enumerate_folder(uri: &str, on_batch: impl FnMut(Vec<Entry>)) -> Result<(), EntryError> {
    let folder = gio::File::for_uri(uri);
    let enumerator = folder
        .enumerate_children_future(ATTRIBUTES, gio::FileQueryInfoFlags::NONE, glib::Priority::DEFAULT)
        .await?;
    let outcome = read_rows(&enumerator, BatchCoalescer::new(on_batch)).await;
    // Closing only releases the enumerator; it cannot change what was read.
    let _ = enumerator.close_future(glib::Priority::LOW).await;
    outcome
}

/// Reads every row of an open enumerator into `batches`.
async fn read_rows(
    enumerator: &gio::FileEnumerator,
    mut batches: BatchCoalescer<impl FnMut(Vec<Entry>)>,
) -> Result<(), EntryError> {
    let mut request_size = FIRST_BATCH_SIZE;
    loop {
        let infos = enumerator
            .next_files_future(request_size, glib::Priority::DEFAULT)
            .await?;
        if infos.is_empty() {
            batches.finish();
            return Ok(());
        }
        let rows = infos
            .iter()
            .map(|info| entry_from_info(&enumerator.child(info), info))
            .collect();
        batches.push(rows, Instant::now());
        request_size = LATER_BATCH_SIZE;
    }
}

/// Delivers the first rows at once and merges later ones for
/// [`MERGE_WINDOW`], so a large folder shows quickly but is not re-sorted
/// for every read.
struct BatchCoalescer<F> {
    deliver: F,
    pending: Vec<Entry>,
    last_delivery: Option<Instant>,
}

impl<F: FnMut(Vec<Entry>)> BatchCoalescer<F> {
    fn new(deliver: F) -> Self {
        Self {
            deliver,
            pending: Vec::new(),
            last_delivery: None,
        }
    }

    /// Adds rows read at `now`, delivering everything pending when this is
    /// the first batch or the merge window has passed.
    fn push(&mut self, rows: Vec<Entry>, now: Instant) {
        self.pending.extend(rows);
        let window_has_passed = self
            .last_delivery
            .is_none_or(|last| now.duration_since(last) >= MERGE_WINDOW);
        if window_has_passed {
            (self.deliver)(std::mem::take(&mut self.pending));
            self.last_delivery = Some(now);
        }
    }

    /// Delivers what is still pending at the end of the folder.
    fn finish(mut self) {
        if !self.pending.is_empty() {
            (self.deliver)(self.pending);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::fs;
    use std::rc::Rc;

    use super::*;
    use crate::entry::test_support::entry_for_uri;

    /// [`FIRST_BATCH_SIZE`] as a row count.
    fn first_batch_len() -> usize {
        usize::try_from(FIRST_BATCH_SIZE).expect("the first batch size is a small positive number")
    }

    /// A temporary folder holding `count` small files.
    fn folder_with_files(count: usize) -> tempfile::TempDir {
        let folder = tempfile::tempdir().expect("temporary folder");
        for index in 0..count {
            fs::write(folder.path().join(format!("file {index}.txt")), b"x").expect("fixture file");
        }
        folder
    }

    fn folder_uri(folder: &tempfile::TempDir) -> String {
        gio::File::for_path(folder.path()).uri().into()
    }

    /// `count` rows without metadata; the coalescer only counts them.
    fn placeholder_rows(count: usize) -> Vec<Entry> {
        let info = gio::FileInfo::new();
        (0..count)
            .map(|index| entry_for_uri(&format!("file:///tmp/{index}"), &info))
            .collect()
    }

    /// parity: PERF-002
    #[test]
    fn the_top_of_a_large_folder_arrives_first() {
        let folder = folder_with_files(first_batch_len() + 10);
        let uri = folder_uri(&folder);
        let mut batch_sizes = Vec::new();
        let listing = enumerate_folder(&uri, |batch| batch_sizes.push(batch.len()));
        glib::MainContext::new()
            .block_on(listing)
            .expect("the folder lists");
        assert_eq!(batch_sizes.first(), Some(&first_batch_len()));
        assert_eq!(batch_sizes.iter().sum::<usize>(), first_batch_len() + 10);
    }

    /// Lists `uri` on `context` until the first rows arrive, then drops the
    /// listing and keeps the loop running long enough for a listing that
    /// kept going to read and deliver the remaining rows.
    fn drop_listing_after_first_batch(context: &glib::MainContext, uri: String, delivered: &Rc<Cell<usize>>) {
        let counter = Rc::clone(delivered);
        let count_rows = move |batch: Vec<Entry>| counter.set(counter.get() + batch.len());
        let listing = context.spawn_local(async move { enumerate_folder(&uri, count_rows).await });
        while delivered.get() == 0 {
            context.iteration(true);
        }
        listing.abort();
        context.block_on(glib::timeout_future(Duration::from_millis(200)));
    }

    /// parity: NAV-016
    #[test]
    fn dropping_the_listing_stops_delivery() {
        let folder = folder_with_files(first_batch_len() + 10);
        let uri = folder_uri(&folder);
        let delivered = Rc::new(Cell::new(0));
        let context = glib::MainContext::new();
        context
            .with_thread_default(|| drop_listing_after_first_batch(&context, uri, &delivered))
            .expect("the test owns its main context");
        assert_eq!(delivered.get(), first_batch_len());
    }

    /// parity: PERF-002
    #[test]
    fn later_batches_are_merged_until_the_window_has_passed() {
        let start = Instant::now();
        let mut delivered = Vec::new();
        let mut batches = BatchCoalescer::new(|batch: Vec<Entry>| delivered.push(batch.len()));
        batches.push(placeholder_rows(3), start);
        batches.push(placeholder_rows(2), start + MERGE_WINDOW / 2);
        batches.push(placeholder_rows(4), start + MERGE_WINDOW);
        batches.push(placeholder_rows(1), start + MERGE_WINDOW);
        batches.finish();
        assert_eq!(delivered, [3, 6, 1]);
    }

    #[test]
    fn nothing_is_delivered_for_an_empty_folder() {
        let mut deliveries = 0;
        BatchCoalescer::new(|_: Vec<Entry>| deliveries += 1).finish();
        assert_eq!(deliveries, 0);
    }
}
