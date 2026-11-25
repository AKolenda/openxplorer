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
pub async fn enumerate_folder(uri: &str, mut on_batch: impl FnMut(Vec<Entry>)) -> Result<(), EntryError> {
    let reader = Reader::start(uri)?;
    let folder = uri.to_owned();
    let mut batches = BatchCoalescer::new(move |mut rows: Vec<Entry>| {
        for row in &mut rows {
            keep_folder_spelling(row, &folder);
        }
        on_batch(rows);
    });
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

/// Gives `row`, read in the SMB folder `folder`, the folder's spelling.
///
/// SMB names ignore case, and `GVfs` names everything inside a mounted
/// share as the share was first mounted (`smb://nas/projects/Plans` after
/// `projects` was typed once), whatever spelling the folder was opened
/// with. A row whose location starts with the folder's in another case
/// takes the folder's, so the address bar, the tab and Recent locations
/// keep the spelling the share was opened with, the server's own when it
/// was opened from the server's list.
fn keep_folder_spelling(row: &mut Entry, folder: &str) {
    if !crate::location::is_smb_location(folder) {
        return;
    }
    let folder = folder.trim_end_matches('/');
    let respell = |uri: &mut String| {
        let Some(head) = uri.get(..folder.len()) else {
            return;
        };
        let rest_starts_a_name = uri[folder.len()..].starts_with('/');
        if head != folder && head.eq_ignore_ascii_case(folder) && rest_starts_a_name {
            uri.replace_range(..folder.len(), folder);
        }
    };
    respell(&mut row.uri);
    if let Some(target) = row.target_uri.as_mut() {
        respell(target);
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
            if let Some(read) = mailbox.reads.pop_front() {
                return Poll::Ready(read);
            }
            mailbox.waker = Some(context.waker().clone());
            Poll::Pending
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

/// Gives a shortcut's row ([`Entry::is_shortcut`]) its own size and time
/// of change, as Windows shows a `.lnk`'s, where the listing followed the
/// link to its target's. Read with `lstat` on the listing's worker
/// thread; a link that cannot be read keeps what the listing found. A
/// shortcut to a folder still opens as one.
fn list_shortcuts_own_facts(row: &mut Entry, child: &gio::File) {
    if !row.is_shortcut() {
        return;
    }
    let Some(metadata) = child.path().and_then(|path| std::fs::symlink_metadata(path).ok()) else {
        return;
    };
    row.size = Some(metadata.len());
    // Signed, so a link dated 1970 or earlier keeps its date.
    row.modified = Some(std::os::unix::fs::MetadataExt::mtime(&metadata));
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
                let child = enumerator.child(&info);
                let mut row = entry_from_info(&child, &info);
                list_shortcuts_own_facts(&mut row, &child);
                rows.push(row);
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

/// How many times as long as delivering a batch took the listing waits
/// before the next one: the main loop then spends at most about a quarter
/// of its time taking in rows.
const DELIVERY_SPACING: u32 = 3;

/// Delivers the first rows at once and merges later ones for
/// [`MERGE_WINDOW`], or for [`DELIVERY_SPACING`] times as long as the last
/// delivery took when that is longer, so a large folder shows quickly but
/// is not re-sorted for every read. Taking in a batch costs the views a
/// pass over every item listed so far, so with 100,000 items the spacing
/// keeps the window answering between batches.
struct BatchCoalescer<F> {
    deliver: F,
    pending: Vec<Entry>,
    last_delivery: Option<Instant>,
    window: Duration,
}

impl<F: FnMut(Vec<Entry>)> BatchCoalescer<F> {
    fn new(deliver: F) -> Self {
        Self {
            deliver,
            pending: Vec::new(),
            last_delivery: None,
            window: MERGE_WINDOW,
        }
    }

    /// Adds rows read at `now`, delivering everything pending when this is
    /// the first batch or the merge window has passed. Returns whether it
    /// delivered.
    fn push(&mut self, rows: Vec<Entry>, now: Instant) -> bool {
        self.pending.extend(rows);
        let window_has_passed = self
            .last_delivery
            .is_none_or(|last| now.duration_since(last) >= self.window);
        if window_has_passed {
            let started = Instant::now();
            (self.deliver)(std::mem::take(&mut self.pending));
            self.window = MERGE_WINDOW.max(started.elapsed() * DELIVERY_SPACING);
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

    /// A row `GVfs` names under the share's mounted spelling takes the
    /// spelling the folder was opened with; other rows are left alone.
    ///
    /// parity: NET-041
    #[test]
    fn rows_of_a_share_keep_the_folders_spelling() {
        let info = gio::FileInfo::new();
        info.set_file_type(gio::FileType::Directory);
        info.set_display_name("Plans");
        let mut row = entry_for_uri("smb://harbornas.local/projects/Plans", &info);
        keep_folder_spelling(&mut row, "smb://harbornas.local/Projects/");
        assert_eq!(row.uri, "smb://harbornas.local/Projects/Plans");

        let mut elsewhere = entry_for_uri("smb://harbornas.local/projectsold/Plans", &info);
        keep_folder_spelling(&mut elsewhere, "smb://harbornas.local/Projects");
        assert_eq!(elsewhere.uri, "smb://harbornas.local/projectsold/Plans");

        let mut local = entry_for_uri("file:///tmp/a/B", &info);
        keep_folder_spelling(&mut local, "file:///tmp/A");
        assert_eq!(local.uri, "file:///tmp/a/B");
    }

    /// A shortcut's Size and Date modified are its own, as Windows shows a
    /// `.lnk`'s (checked in a Windows 11 VM): a link to a large, old file
    /// or to a folder lists the link's few bytes and its own time. A link
    /// named for a type keeps its target's facts, as a file does.
    ///
    /// parity: VIEW-074
    #[test]
    fn a_shortcut_lists_its_own_size_and_date() {
        let folder = tempfile::tempdir().expect("temporary folder");
        let video = folder.path().join("Harbor Clip.mp4");
        fs::write(&video, vec![0_u8; 300_000]).expect("fixture file");
        let old = std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_000_000_000);
        fs::File::options()
            .write(true)
            .open(&video)
            .and_then(|file| file.set_modified(old))
            .expect("an old time");
        fs::create_dir(folder.path().join("Projects")).expect("fixture folder");
        std::os::unix::fs::symlink(&video, folder.path().join("Harbor Clip - Shortcut")).expect("link");
        std::os::unix::fs::symlink(
            folder.path().join("Projects"),
            folder.path().join("Projects - Shortcut"),
        )
        .expect("link");
        std::os::unix::fs::symlink(&video, folder.path().join("clip.mp4")).expect("link");

        let rows = std::cell::RefCell::new(Vec::new());
        glib::MainContext::new()
            .block_on(enumerate_folder(&folder_uri(&folder), |batch| {
                rows.borrow_mut().extend(batch);
            }))
            .expect("listed");
        let rows = rows.into_inner();
        let row = |name: &str| rows.iter().find(|row| row.name == name).expect("listed").clone();

        let link_size = |name: &str| {
            fs::symlink_metadata(folder.path().join(name))
                .expect("a link")
                .len()
        };
        let video_link = row("Harbor Clip - Shortcut");
        assert_eq!(video_link.size, Some(link_size("Harbor Clip - Shortcut")));
        assert!(video_link.modified > Some(1_000_000_000), "the link's own time");
        let folder_link = row("Projects - Shortcut");
        assert!(folder_link.is_dir, "it still opens as a folder");
        assert_eq!(folder_link.size, Some(link_size("Projects - Shortcut")));
        assert_eq!(
            row("clip.mp4").size,
            Some(300_000),
            "a link named for a type is a file"
        );
    }

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

    /// A batch the views took long to take in spaces the next one out,
    /// so the main loop is mostly free while a large folder is listed.
    ///
    /// parity: PERF-002
    #[test]
    fn a_slow_delivery_spaces_out_the_next_one() {
        let start = Instant::now();
        let mut delivered = Vec::new();
        let mut batches = BatchCoalescer::new(|batch: Vec<Entry>| {
            if delivered.is_empty() {
                std::thread::sleep(Duration::from_millis(150));
            }
            delivered.push(batch.len());
        });
        batches.push(placeholder_rows(3), start);
        batches.push(
            placeholder_rows(2),
            start + MERGE_WINDOW + Duration::from_millis(10),
        );
        batches.push(placeholder_rows(4), start + Duration::from_millis(460));
        batches.finish();
        assert_eq!(
            delivered,
            [3, 6],
            "the second batch waited 450 ms, three times the first's 150 ms"
        );
    }

    #[test]
    fn nothing_is_delivered_for_an_empty_folder() {
        let mut deliveries = 0;
        BatchCoalescer::new(|_: Vec<Entry>| deliveries += 1).finish();
        assert_eq!(deliveries, 0);
    }
}
