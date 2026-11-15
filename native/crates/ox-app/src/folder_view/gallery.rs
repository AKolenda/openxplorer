// SPDX-License-Identifier: AGPL-3.0-only
//! Gallery's listing: the photos of the Pictures folder and its subfolders
//! (HOME-019), listed like a folder.
//!
//! Windows 11 File Explorer's Gallery shows "your most recently taken
//! photos" at the top
//! (<https://support.microsoft.com/en-us/topic/september-26-2023-windows-configuration-update-542780c2-594c-46cb-979d-11116fe164ba>).
//! The window sorts and groups the rows by the date each photo was taken
//! ([`crate::folder_view::sort_roles::photo_date`]); this module finds the
//! photos and reads that date.
//!
//! The folders are walked as a search walks them
//! ([`ox_core::search::walk_search`]): never through a link, into another
//! filesystem or a system folder, and below the top level only on this
//! computer's own disks. The walk runs on a thread of its own and hands
//! photos over folder by folder; the main loop takes them in batches, the
//! first at once and later ones merged for a fifth of a second, so a
//! library of thousands of photos never holds the window up. Dropping the
//! [`Listing`] cancels the walk.
//!
//! A photo's date taken is EXIF's `DateTimeOriginal`, read with
//! kamadak-exif (pure Rust) from at most the first [`EXIF_HEAD`] bytes of
//! the file, on the walk's thread, and remembered for the session by
//! location, size and date modified ([`DATES`]), so listing Gallery again
//! reads only new or changed photos. Nothing is written and nothing leaves
//! the computer.

use std::collections::HashMap;
use std::io::Read;
use std::sync::{Mutex, OnceLock, PoisonError};
use std::time::Duration;

use gtk::prelude::*;
use gtk::{gio, glib};
use ox_core::entry::{Entry, EntryError};
use ox_core::search::{
    walk_search, FacetMatcher, HiddenItems, KindFacet, LiveSearch, NamePattern, SearchFacets, SearchIn,
    TreeWatch,
};

use super::loader::Listing;

/// How much of a photo is read for its date: EXIF sits at the start.
const EXIF_HEAD: u64 = 256 * 1024;
/// How long later batches are merged before they reach the window.
const BATCH_MERGE: Duration = Duration::from_millis(200);
/// The most folders Gallery watches for changes.
const WATCH_LIMIT: usize = 4096;
/// How often the watch is asked whether anything changed.
const WATCH_POLL: Duration = Duration::from_millis(400);

/// The content types whose files may carry EXIF; others are not opened.
const EXIF_TYPES: [&str; 8] = [
    "image/jpeg",
    "image/tiff",
    "image/heic",
    "image/heif",
    "image/avif",
    "image/png",
    "image/webp",
    "image/x-adobe-dng",
];

/// What is remembered of a photo's date: the size and date modified it
/// was read at, and the date taken found then.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct KnownDate {
    size: Option<u64>,
    modified: Option<u64>,
    taken: Option<u64>,
}

/// The dates taken read this session, by location.
static DATES: OnceLock<Mutex<HashMap<String, KnownDate>>> = OnceLock::new();

/// Lists the photos of `folders` and their subfolders, hidden ones and
/// those in hidden folders only with `hidden` set to include them, calling
/// `on_batch` with rows as they are found and `on_done` once at the end. A
/// folder that does not exist or cannot be read lists nothing, without an
/// error: Gallery is then empty. Neither is called after the [`Listing`]
/// is dropped.
pub(crate) fn list_gallery(
    folders: Vec<String>,
    hidden: HiddenItems,
    on_batch: impl Fn(Vec<Entry>) + 'static,
    on_done: impl FnOnce(Result<(), EntryError>) + 'static,
) -> Listing {
    Listing::spawn(async move {
        #[cfg(test)]
        crate::test_support::held::wait_while_held(ox_core::location::GALLERY_URI).await;
        let cancellable = gio::Cancellable::new();
        let _cancel_on_drop = CancelOnDrop(cancellable.clone());
        let (sender, receiver) = async_channel::unbounded::<Vec<Entry>>();
        let walking = cancellable.clone();
        let started = std::thread::Builder::new()
            .name("openxplorer-gallery".to_owned())
            .spawn(move || walk_folders(&folders, hidden, &walking, &sender));
        if started.is_err() {
            on_done(Err(EntryError::Failed(ox_core::i18n::gettext(
                "Gallery could not start reading your photos.",
            ))));
            return;
        }
        let mut is_first = true;
        while let Ok(mut batch) = receiver.recv().await {
            if !is_first {
                glib::timeout_future(BATCH_MERGE).await;
            }
            while let Ok(more) = receiver.try_recv() {
                batch.extend(more);
            }
            is_first = false;
            on_batch(batch);
        }
        on_done(Ok(()));
    })
}

/// Cancels the walk when the listing that owns it is dropped.
struct CancelOnDrop(gio::Cancellable);

impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        self.0.cancel();
    }
}

/// Walks every one of `folders` in turn on the calling thread, sending
/// each batch of photos, with their dates taken, to `sender`.
fn walk_folders(
    folders: &[String],
    hidden: HiddenItems,
    cancellable: &gio::Cancellable,
    sender: &async_channel::Sender<Vec<Entry>>,
) {
    let facets = SearchFacets {
        kind: KindFacet::Images,
        ..SearchFacets::default()
    };
    let Ok(now) = glib::DateTime::now_local() else {
        return;
    };
    let filter: FacetMatcher = facets.matcher(&now);
    let mut walked: Vec<&String> = Vec::new();
    for folder in folders {
        if walked.contains(&folder) {
            continue;
        }
        walked.push(folder);
        let search = LiveSearch {
            folder: folder.clone(),
            pattern: NamePattern::new(""),
            hidden_items: hidden,
            limit: usize::MAX,
            search_in: SearchIn::Names,
            filter,
            subfolders: true,
        };
        // A missing or unreadable folder lists nothing; a cancelled walk
        // ends here.
        let _ = walk_search(&search, cancellable, &mut |mut batch| {
            batch.retain(|entry| !entry.is_dir);
            for entry in &mut batch {
                if cancellable.is_cancelled() {
                    return;
                }
                entry.meta.taken = date_taken_of(entry);
            }
            let _ = sender.send_blocking(batch);
        });
        if cancellable.is_cancelled() {
            return;
        }
    }
}

/// The date `entry` was taken, from the session's memory while its size
/// and date modified are unchanged, else read from the file now.
fn date_taken_of(entry: &Entry) -> Option<u64> {
    let dates = DATES.get_or_init(Mutex::default);
    let known = dates
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .get(&entry.uri)
        .copied();
    if let Some(known) = known.filter(|known| known.size == entry.size && known.modified == entry.modified) {
        return known.taken;
    }
    let taken = read_date_taken(entry);
    dates.lock().unwrap_or_else(PoisonError::into_inner).insert(
        entry.uri.clone(),
        KnownDate {
            size: entry.size,
            modified: entry.modified,
            taken,
        },
    );
    taken
}

/// Reads `entry`'s EXIF date taken, for a local file of a type that may
/// carry EXIF.
fn read_date_taken(entry: &Entry) -> Option<u64> {
    let content_type = entry.content_type.as_deref()?;
    if !EXIF_TYPES.contains(&content_type) {
        return None;
    }
    let path = gio::File::for_uri(&entry.uri).path()?;
    let mut head = Vec::new();
    std::fs::File::open(path)
        .ok()?
        .take(EXIF_HEAD)
        .read_to_end(&mut head)
        .ok()?;
    date_taken_in(&head)
}

/// The EXIF date taken in `head`, the start of a picture, as seconds
/// since the Unix epoch, read as the local time it names.
pub(crate) fn date_taken_in(head: &[u8]) -> Option<u64> {
    // The bytes are untrusted: a reader that panics on them leaves the
    // photo without a date rather than ending the walk.
    let text = std::panic::catch_unwind(|| {
        let mut cursor = std::io::Cursor::new(head);
        let exif = exif::Reader::new().read_from_container(&mut cursor).ok()?;
        let field = exif.get_field(exif::Tag::DateTimeOriginal, exif::In::PRIMARY)?;
        match &field.value {
            exif::Value::Ascii(parts) => parts
                .first()
                .map(|bytes| String::from_utf8_lossy(bytes).into_owned()),
            _ => None,
        }
    })
    .ok()??;
    let date = crate::media_details::NaiveDateTime::parse_exif(&text)?;
    let local = glib::DateTime::from_local(
        date.year,
        date.month,
        date.day,
        date.hour,
        date.minute,
        f64::from(date.second),
    )
    .ok()?;
    u64::try_from(local.to_unix()).ok()
}

/// Watches the folders Gallery lists, and every folder below them, while
/// it is shown, calling back when anything in them changed (HOME-021).
/// Dropping it stops watching.
pub(crate) struct GalleryWatch {
    watch: Option<TreeWatch>,
    poll: Option<glib::SourceId>,
    /// Cancels a scan of the folders still running.
    scanning: gio::Cancellable,
}

impl std::fmt::Debug for GalleryWatch {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.debug_struct("GalleryWatch").finish_non_exhaustive()
    }
}

impl GalleryWatch {
    /// Starts watching; `on_change` runs on the main loop after changes.
    /// Without inotify nothing is watched, and F5 lists Gallery again.
    pub(crate) fn start(on_change: impl Fn() + 'static) -> Self {
        let watch = match TreeWatch::start(WATCH_LIMIT) {
            Ok(watch) => Some(watch),
            Err(error) => {
                glib::g_warning!(ox_core::LOG_DOMAIN, "Gallery cannot follow changes: {error}");
                None
            }
        };
        let poll = watch.clone().map(|polled| {
            glib::timeout_add_local(WATCH_POLL, move || {
                if polled.take_changes() {
                    on_change();
                }
                glib::ControlFlow::Continue
            })
        });
        Self {
            watch,
            poll,
            scanning: gio::Cancellable::new(),
        }
    }

    /// Watches `folders` and the folders below them, on a worker thread;
    /// folders already watched stay so. Run again after each listing, as
    /// new folders may have appeared.
    pub(crate) fn watch_folders(&mut self, folders: Vec<String>) {
        let Some(watch) = self.watch.clone() else {
            return;
        };
        self.scanning.cancel();
        self.scanning = gio::Cancellable::new();
        let scanning = self.scanning.clone();
        let _ = std::thread::Builder::new()
            .name("openxplorer-gallery-watch".to_owned())
            .spawn(move || {
                for folder in folders {
                    watch.watch_tree(&folder, &scanning);
                }
            });
    }
}

impl Drop for GalleryWatch {
    fn drop(&mut self) {
        self.scanning.cancel();
        if let Some(poll) = self.poll.take() {
            poll.remove();
        }
        // Stopping inotify's reader waits for it to look up, up to a fifth
        // of a second: never on the main thread.
        if let Some(watch) = self.watch.take() {
            let _ = std::thread::Builder::new()
                .name("openxplorer-gallery-unwatch".to_owned())
                .spawn(move || drop(watch));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A picture's EXIF date taken is read as the local time it names;
    /// a picture without EXIF, or with broken EXIF, has none.
    ///
    /// parity: HOME-020
    #[gtk::test]
    fn the_date_taken_is_read_from_exif_in_local_time() {
        let jpeg_without = [0xff_u8, 0xd8, 0xff, 0xd9];
        assert_eq!(date_taken_in(&jpeg_without), None);
        assert_eq!(
            date_taken_in(b"\xff\xd8\xff\xe1\x00\x10Exif\0\0II*\0garbage"),
            None
        );
        let with = crate::test_support::pictures::jpeg_bytes(0x0033_6699, Some("2025:12:24 10:00:00"));
        let expected = glib::DateTime::from_local(2025, 12, 24, 10, 0, 0.0)
            .expect("a date")
            .to_unix();
        assert_eq!(
            date_taken_in(&with).and_then(|seconds| i64::try_from(seconds).ok()),
            Some(expected)
        );
    }
}
