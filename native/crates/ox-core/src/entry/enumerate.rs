// SPDX-License-Identifier: AGPL-3.0-only
//! Folder listing on a reader thread.
//!
//! Ports `enumerate_folder` in `v2.0.0:desktop/gio_backend.py`. GIO owns backend
//! enumeration (no per-row stat). A reader thread of its own reads the
//! folder with GIO's blocking enumerator and turns each `GFileInfo` into an
//! [`Entry`] there, so the GTK main loop only receives finished rows: with
//! 100,000 files, building the rows on the main loop took about half a
//! second of the interface thread's time. Rows arrive in batches: the first
//! one at once, so the top of a large folder shows quickly, and later ones
//! merged for a quarter of a second, so the view is not re-sorted for every
//! read.
//!
//! Unlike the Python backend, hidden items are listed too, with
//! [`Entry::is_hidden`] set: the view filters them, so toggling Show hidden
//! files does not read the folder again.

use std::collections::VecDeque;
use std::future::poll_fn;
use std::sync::{Arc, Mutex, PoisonError};
use std::task::{Poll, Waker};
use std::time::{Duration, Instant};

use gio::prelude::*;

use super::{entry_from_info, Entry, EntryError, ATTRIBUTES};

/// Rows in the first batch, shown before the rest of the folder is read.
const FIRST_BATCH_SIZE: usize = 64;
/// Rows the reader sends at a time after the first.
const LATER_BATCH_SIZE: usize = 512;
/// How long later batches are merged before they are delivered.
const MERGE_WINDOW: Duration = Duration::from_millis(250);

/// Lists the folder at `uri`, passing its rows to `on_batch` as they are
/// read.
///
/// Dropping the future cancels the listing: the reader's pending read is
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
    let reader = Reader::start(uri)?;
    let mut batches = BatchCoalescer::new(on_batch);
    loop {
        match reader.next().await {
            Read::Rows(rows) => {
                if batches.push(rows, Instant::now()) {
                    // Let the main loop draw the rows and handle input, or
                    // drop this listing, before the next batch.
                    yield_once().await;
                }
            }
            Read::End => {
                batches.finish();
                // The listing ends after the views have taken in the last
                // rows, at low priority as closing GIO's enumerator here
                // used to: what runs at the end (restoring the scroll
                // position and the selection) needs the rows laid out.
                glib::timeout_future_with_priority(glib::Priority::LOW, Duration::ZERO).await;
                return Ok(());
            }
            Read::Failed(error) => return Err(error),
        }
    }
}

/// What the reader thread sends.
#[derive(Debug)]
enum Read {
    /// The next rows of the folder.
    Rows(Vec<Entry>),
    /// The whole folder is read.
    End,
    /// Reading failed; nothing follows.
    Failed(EntryError),
}

/// What the reader thread and the listing share.
#[derive(Debug, Default)]
struct Mailbox {
    /// Reads not yet taken, oldest first.
    reads: VecDeque<Read>,
    /// Woken when a read arrives.
    waker: Option<Waker>,
    /// The listing was dropped; the reader stops.
    abandoned: bool,
}

/// A reader thread listing one folder; dropping it stops the reader.
struct Reader {
    mailbox: Arc<Mutex<Mailbox>>,
    cancellable: gio::Cancellable,
}

impl Reader {
    /// Starts reading the folder at `uri` on a thread of its own.
    fn start(uri: &str) -> Result<Self, EntryError> {
        let mailbox = Arc::new(Mutex::new(Mailbox::default()));
        let cancellable = gio::Cancellable::new();
        let (shared, cancel, uri) = (Arc::clone(&mailbox), cancellable.clone(), uri.to_owned());
        std::thread::Builder::new()
            .name("ox-list-folder".into())
            .spawn(move || read_folder(&uri, &cancel, &shared))
            .map_err(|error| EntryError::Failed(error.to_string()))?;
        Ok(Self { mailbox, cancellable })
    }

    /// The next read, once the reader has sent it.
    async fn next(&self) -> Read {
        poll_fn(|context| {
            let mut mailbox = lock(&self.mailbox);
            match mailbox.reads.pop_front() {
                Some(read) => Poll::Ready(read),
                None => {
                    mailbox.waker = Some(context.waker().clone());
                    Poll::Pending
                }
            }
        })
        .await
    }
}

impl Drop for Reader {
    fn drop(&mut self) {
        lock(&self.mailbox).abandoned = true;
        self.cancellable.cancel();
    }
}

/// Returns to the main loop once, so what is waiting there runs first.
async fn yield_once() {
    let mut yielded = false;
    poll_fn(|context| {
        if yielded {
            Poll::Ready(())
        } else {
            yielded = true;
            context.waker().wake_by_ref();
            Poll::Pending
        }
    })
    .await;
}

/// The mailbox, also after a reader panicked while holding it.
fn lock(mailbox: &Mutex<Mailbox>) -> std::sync::MutexGuard<'_, Mailbox> {
    mailbox.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Sends `read` to the listing; false once the listing was dropped.
fn send(mailbox: &Mutex<Mailbox>, read: Read) -> bool {
    let mut mailbox = lock(mailbox);
    if mailbox.abandoned {
        return false;
    }
    mailbox.reads.push_back(read);
    if let Some(waker) = mailbox.waker.take() {
        waker.wake();
    }
    true
}

/// Reads the folder at `uri` into `mailbox`, the first
/// [`FIRST_BATCH_SIZE`] rows on their own, then [`LATER_BATCH_SIZE`] at a
/// time, until the end, a failure or the listing is dropped.
fn read_folder(uri: &str, cancellable: &gio::Cancellable, mailbox: &Mutex<Mailbox>) {
    let folder = gio::File::for_uri(uri);
    let enumerator =
        match folder.enumerate_children(ATTRIBUTES, gio::FileQueryInfoFlags::NONE, Some(cancellable)) {
            Ok(enumerator) => enumerator,
            Err(error) => {
                send(mailbox, Read::Failed(error.into()));
                return;
            }
        };
    let mut batch_size = FIRST_BATCH_SIZE;
    let mut rows = Vec::with_capacity(batch_size);
    let outcome = loop {
        match enumerator.next_file(Some(cancellable)) {
            Ok(Some(info)) => {
                rows.push(entry_from_info(&enumerator.child(&info), &info));
                if rows.len() == batch_size {
                    batch_size = LATER_BATCH_SIZE;
                    let full = std::mem::replace(&mut rows, Vec::with_capacity(batch_size));
                    if !send(mailbox, Read::Rows(full)) {
                        break None;
                    }
                }
            }
            Ok(None) => break Some(Read::End),
            Err(error) => break Some(Read::Failed(error.into())),
        }
    };
    // Closing only releases the enumerator; it cannot change what was read.
    let _ = enumerator.close(gio::Cancellable::NONE);
    if let Some(last) = outcome {
        if matches!(last, Read::End) && !rows.is_empty() && !send(mailbox, Read::Rows(rows)) {
            return;
        }
        send(mailbox, last);
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
    /// the first batch or the merge window has passed. Returns whether it
    /// delivered.
    fn push(&mut self, rows: Vec<Entry>, now: Instant) -> bool {
        self.pending.extend(rows);
        let window_has_passed = self
            .last_delivery
            .is_none_or(|last| now.duration_since(last) >= MERGE_WINDOW);
        if window_has_passed {
            (self.deliver)(std::mem::take(&mut self.pending));
            self.last_delivery = Some(now);
        }
        window_has_passed
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
        FIRST_BATCH_SIZE
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

    /// The rows are built on the reader thread, so a large folder costs
    /// the main loop only the rows it receives.
    ///
    /// parity: PERF-003
    #[test]
    fn rows_are_built_off_the_listing_thread() {
        let folder = folder_with_files(1500);
        let uri = folder_uri(&folder);
        let built_here = || super::super::info::ROWS_BUILT_HERE.with(std::cell::Cell::get);
        let before = built_here();
        let mut listed = 0;
        let listing = enumerate_folder(&uri, |batch| listed += batch.len());
        glib::MainContext::new()
            .block_on(listing)
            .expect("the folder lists");
        assert_eq!(listed, 1500);
        assert_eq!(built_here(), before, "no row was built on this thread");
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
