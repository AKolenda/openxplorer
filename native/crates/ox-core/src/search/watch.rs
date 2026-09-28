// SPDX-License-Identifier: AGPL-3.0-only
//! Live local change events: one inotify watch per indexed directory.
//!
//! Ports `LocalWatch` in `desktop/local_watch.py` (SRCH-028, SRCH-029).
//! No file contents are read here. Watch exhaustion and queue overflow are
//! reported to the service, never passed off as complete live coverage.
//!
//! A reader thread waits for events and sends them over a channel; the
//! service takes them on its next tick. Each event keeps the instant it
//! arrived, so the service's debounce still counts from the change itself.

use std::collections::{HashMap, HashSet};
use std::ffi::{CStr, OsStr};
use std::mem::MaybeUninit;
use std::os::fd::OwnedFd;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread::{self, JoinHandle};
use std::time::Instant;

use rustix::event::{poll, PollFd, PollFlags, Timespec};
use rustix::fs::inotify::{self, CreateFlags, ReadFlags, WatchFlags};
use rustix::io::Errno;

use super::text::{is_at_or_below, local_path, parent_uri};
use crate::location::file_uri;

/// Most directories watched at once (`max_watches` in Python).
pub(crate) const WATCH_LIMIT: usize = 8192;

/// How long the reader waits before looking whether it should stop.
const STOP_CHECK_INTERVAL: Timespec = Timespec {
    tv_sec: 0,
    tv_nsec: 200_000_000,
};

/// Bytes read from inotify at once.
const READ_BUFFER_BYTES: usize = 256 * 1024;

/// Why a directory could not be watched. Stored as the root's watch error.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub(crate) enum WatchError {
    /// [`WATCH_LIMIT`] directories are watched already.
    #[error("Live watch limit reached (8192 directories). Unwatched directories use timed checks.")]
    LimitReached,
    /// The location is not a local directory.
    #[error("Only local folders can be watched.")]
    NotLocal,
    /// The kernel refused the watch, for example because the user's inotify
    /// limit is used up.
    #[error("Could not watch a directory: {0}")]
    Refused(Errno),
    /// inotify could not be started at all, so nothing is watched.
    #[error("inotify unavailable; using incremental checks.")]
    Unavailable,
}

/// A change the service must act on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum WatchEvent {
    /// Something in `folder`, below `root`, changed at `changed_at`.
    Changed {
        /// The root the watch belongs to.
        root: String,
        /// The folder to re-read.
        folder: String,
        /// When the event arrived.
        changed_at: Instant,
    },
    /// The kernel dropped events: only a full rescan is reliable now.
    Overflowed,
}

/// Which root watches a directory, and under which URI.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct WatchReference {
    root: String,
    uri: String,
}

/// The watches: the roots referencing each watch descriptor, and the
/// descriptor of each watched path.
#[derive(Debug, Default)]
struct WatchTable {
    by_descriptor: HashMap<i32, HashSet<WatchReference>>,
    by_path: HashMap<PathBuf, i32>,
}

/// inotify watches for the indexed local directories.
#[derive(Debug)]
pub(crate) struct LocalWatch {
    inotify: Arc<OwnedFd>,
    table: Arc<Mutex<WatchTable>>,
    limit: usize,
    events: Receiver<WatchEvent>,
    stop: Arc<AtomicBool>,
    reader: Option<JoinHandle<()>>,
}

impl LocalWatch {
    /// Starts watching with at most `limit` directories.
    ///
    /// # Errors
    ///
    /// The error of `inotify_init1` or of starting the reader thread, for
    /// example when inotify is unavailable.
    pub(crate) fn start(limit: usize) -> std::io::Result<Self> {
        let inotify = Arc::new(inotify::init(CreateFlags::NONBLOCK | CreateFlags::CLOEXEC)?);
        let table = Arc::new(Mutex::new(WatchTable::default()));
        let stop = Arc::new(AtomicBool::new(false));
        let (sender, events) = mpsc::channel();
        let reader = EventReader {
            inotify: Arc::clone(&inotify),
            table: Arc::clone(&table),
            stop: Arc::clone(&stop),
            sender,
        };
        let reader = thread::Builder::new()
            .name("openxplorer-inotify".to_owned())
            .spawn(move || reader.run())?;
        Ok(Self {
            inotify,
            table,
            limit,
            events,
            stop,
            reader: Some(reader),
        })
    }

    /// Watches the directory `uri` for `root`.
    ///
    /// Safety rule "never follow a link" (`add` in `local_watch.py`):
    /// `IN_DONT_FOLLOW` and `IN_ONLYDIR` watch only a real directory, so a
    /// symlink cannot extend the watch outside the root.
    ///
    /// # Errors
    ///
    /// [`WatchError::LimitReached`] beyond the limit, [`WatchError::NotLocal`]
    /// for a non-local URI, and [`WatchError::Refused`] when the kernel
    /// refuses.
    pub(crate) fn add(&self, root: &str, uri: &str) -> Result<(), WatchError> {
        let path = local_path(uri).ok_or(WatchError::NotLocal)?;
        let reference = WatchReference {
            root: root.to_owned(),
            uri: uri.to_owned(),
        };
        let mut table = lock(&self.table);
        if let Some(descriptor) = table.by_path.get(&path).copied() {
            table.reference(descriptor, reference);
            return Ok(());
        }
        if table.by_descriptor.len() >= self.limit {
            return Err(WatchError::LimitReached);
        }
        let flags = WatchFlags::MODIFY
            | WatchFlags::ATTRIB
            | WatchFlags::CLOSE_WRITE
            | WatchFlags::MOVED_FROM
            | WatchFlags::MOVED_TO
            | WatchFlags::CREATE
            | WatchFlags::DELETE
            | WatchFlags::DELETE_SELF
            | WatchFlags::MOVE_SELF
            | WatchFlags::ONLYDIR
            | WatchFlags::DONT_FOLLOW;
        let descriptor = inotify::add_watch(&*self.inotify, &path, flags).map_err(WatchError::Refused)?;
        table.by_path.insert(path, descriptor);
        // inotify returns the same descriptor for a bind mount or other alias
        // of a watched directory.
        table.reference(descriptor, reference);
        Ok(())
    }

    /// How many directories `root` watches.
    pub(crate) fn count(&self, root: &str) -> usize {
        let table = lock(&self.table);
        table
            .by_descriptor
            .values()
            .filter(|references| references.iter().any(|reference| reference.root == root))
            .count()
    }

    /// Stops every watch of `root`.
    pub(crate) fn remove_root(&self, root: &str) {
        lock(&self.table).remove(&self.inotify, root, None);
    }

    /// The events that arrived since the last call.
    pub(crate) fn take_events(&self) -> Vec<WatchEvent> {
        self.events.try_iter().collect()
    }
}

impl Drop for LocalWatch {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(reader) = self.reader.take() {
            // The reader looks at `stop` at least every 200 ms. A reader
            // that panicked has nothing left to clean up.
            let _ = reader.join();
        }
    }
}

impl WatchTable {
    /// Records that `reference` watches the directory of `descriptor`.
    fn reference(&mut self, descriptor: i32, reference: WatchReference) {
        self.by_descriptor
            .entry(descriptor)
            .or_default()
            .insert(reference);
    }

    /// Drops the references of `root`, only those at or below `subtree`
    /// when given, and stops the watches nothing references any more.
    fn remove(&mut self, inotify: &OwnedFd, root: &str, subtree: Option<&str>) {
        let mut unreferenced = Vec::new();
        for (descriptor, references) in &mut self.by_descriptor {
            references.retain(|reference| {
                let is_in_subtree = subtree.is_none_or(|subtree| is_at_or_below(&reference.uri, subtree));
                reference.root != root || !is_in_subtree
            });
            if references.is_empty() {
                unreferenced.push(*descriptor);
            }
        }
        for descriptor in unreferenced {
            self.forget(descriptor);
            // The directory may be gone already, which ended its watch.
            let _ = inotify::remove_watch(inotify, descriptor);
        }
    }

    /// Forgets a watch the kernel ended or that was removed.
    fn forget(&mut self, descriptor: i32) {
        self.by_descriptor.remove(&descriptor);
        self.by_path.retain(|_, watched| *watched != descriptor);
    }
}

/// The reader thread's view of the watch.
struct EventReader {
    inotify: Arc<OwnedFd>,
    table: Arc<Mutex<WatchTable>>,
    stop: Arc<AtomicBool>,
    sender: Sender<WatchEvent>,
}

impl EventReader {
    /// Reads events until the watch is dropped.
    fn run(self) {
        let mut buffer = vec![MaybeUninit::<u8>::uninit(); READ_BUFFER_BYTES];
        while !self.stop.load(Ordering::Relaxed) {
            if !self.wait_for_events() {
                continue;
            }
            let mut events = inotify::Reader::new(&*self.inotify, &mut buffer);
            // WOULDBLOCK once the queue is empty; any other error is retried
            // on the next wake-up, as Python's loop does.
            while let Ok(event) = events.next() {
                self.handle(event.wd(), event.events(), event.file_name());
            }
        }
    }

    /// Waits up to [`STOP_CHECK_INTERVAL`]; true when events are waiting.
    fn wait_for_events(&self) -> bool {
        let mut descriptors = [PollFd::new(&*self.inotify, PollFlags::IN)];
        matches!(poll(&mut descriptors, Some(&STOP_CHECK_INTERVAL)), Ok(ready) if ready > 0)
    }

    /// Turns one inotify event into service events (`_loop` in Python).
    fn handle(&self, descriptor: i32, flags: ReadFlags, name: Option<&CStr>) {
        if flags.contains(ReadFlags::QUEUE_OVERFLOW) {
            self.send(WatchEvent::Overflowed);
            return;
        }
        let changed_at = Instant::now();
        let removed_name = removed_subfolder(flags, name);
        for reference in self.references(descriptor, flags) {
            if let Some(child) = removed_name.and_then(|name| child_uri(&reference.uri, name)) {
                // The watches below a deleted or moved-away folder end with it.
                lock(&self.table).remove(&self.inotify, &reference.root, Some(&child));
            }
            let Some(folder) = changed_folder(flags, reference.uri) else {
                continue;
            };
            self.send(WatchEvent::Changed {
                root: reference.root,
                folder,
                changed_at,
            });
        }
    }

    /// The references of `descriptor`; a watch the kernel ended
    /// (`IN_IGNORED`) is forgotten after reading them.
    fn references(&self, descriptor: i32, flags: ReadFlags) -> Vec<WatchReference> {
        let mut table = lock(&self.table);
        let Some(references) = table.by_descriptor.get(&descriptor) else {
            return Vec::new();
        };
        let references = references.iter().cloned().collect();
        if flags.contains(ReadFlags::IGNORED) {
            table.forget(descriptor);
        }
        references
    }

    /// Sends an event; the service may have closed, which ends the watch.
    fn send(&self, event: WatchEvent) {
        let _ = self.sender.send(event);
    }
}

/// The name of the subfolder an event says was deleted or moved away.
fn removed_subfolder(flags: ReadFlags, name: Option<&CStr>) -> Option<&Path> {
    let is_removal = flags.intersects(ReadFlags::DELETE | ReadFlags::MOVED_FROM);
    if !is_removal || !flags.contains(ReadFlags::ISDIR) {
        return None;
    }
    let name = name?.to_bytes();
    Some(Path::new(OsStr::from_bytes(name)))
}

/// The URI of `name` in the watched directory `uri`, spelled as the
/// watches below it are.
fn child_uri(uri: &str, name: &Path) -> Option<String> {
    let directory = local_path(uri)?;
    Some(file_uri(&directory.join(name)))
}

/// The folder to re-read after an event on the watched directory `uri`:
/// its parent when the directory itself was deleted or moved, none when
/// only the watch ended.
fn changed_folder(flags: ReadFlags, uri: String) -> Option<String> {
    if flags.intersects(ReadFlags::DELETE_SELF | ReadFlags::MOVE_SELF) {
        return Some(parent_uri(&uri));
    }
    if flags.contains(ReadFlags::IGNORED) {
        return None;
    }
    Some(uri)
}

/// Locks the watch table.
///
/// A panic while it was held leaves at worst a stale watch, which the next
/// full scan replaces, so a poisoned lock is used as it is.
fn lock(table: &Mutex<WatchTable>) -> MutexGuard<'_, WatchTable> {
    table.lock().unwrap_or_else(PoisonError::into_inner)
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::time::Duration;

    use super::*;

    /// How long the tests wait for the kernel's events.
    const EVENT_TIMEOUT: Duration = Duration::from_secs(5);

    /// How often the tests look for events.
    const POLL_INTERVAL: Duration = Duration::from_millis(50);

    /// Waits until `watch` reports events and returns them; none when
    /// nothing arrived within [`EVENT_TIMEOUT`].
    fn wait_for_events(watch: &LocalWatch) -> Vec<WatchEvent> {
        let deadline = Instant::now() + EVENT_TIMEOUT;
        while Instant::now() < deadline {
            let events = watch.take_events();
            if !events.is_empty() {
                return events;
            }
            thread::sleep(POLL_INTERVAL);
        }
        Vec::new()
    }

    /// Ported from `desktop/tests/test_v05.py::LiveTests::test_watch_limit_fallback_is_reported`
    /// (the watch half; the service half is in `service_tests.rs`).
    ///
    /// parity: SRCH-029
    #[test]
    fn directories_beyond_the_limit_are_refused() {
        let tree = tempfile::tempdir().unwrap();
        fs::create_dir(tree.path().join("extra")).unwrap();
        let root = file_uri(tree.path());
        let watch = LocalWatch::start(1).unwrap();

        watch.add(&root, &root).unwrap();
        let refused = watch.add(&root, &file_uri(&tree.path().join("extra")));

        assert_eq!(refused, Err(WatchError::LimitReached));
        assert_eq!(watch.count(&root), 1);
    }

    /// parity: SRCH-028
    #[test]
    fn a_new_file_reports_its_folder() {
        let tree = tempfile::tempdir().unwrap();
        let root = file_uri(tree.path());
        let watch = LocalWatch::start(WATCH_LIMIT).unwrap();
        watch.add(&root, &root).unwrap();

        fs::write(tree.path().join("new.txt"), "x").unwrap();

        let events = wait_for_events(&watch);
        assert!(matches!(&events[0], WatchEvent::Changed { folder, .. } if *folder == root));
    }

    /// parity: SRCH-028
    #[test]
    fn deleting_a_watched_folder_ends_its_watch() {
        let tree = tempfile::tempdir().unwrap();
        let child = tree.path().join("child");
        fs::create_dir(&child).unwrap();
        let root = file_uri(tree.path());
        let watch = LocalWatch::start(WATCH_LIMIT).unwrap();
        watch.add(&root, &root).unwrap();
        watch.add(&root, &file_uri(&child)).unwrap();

        fs::remove_dir(&child).unwrap();

        let deadline = Instant::now() + EVENT_TIMEOUT;
        while watch.count(&root) != 1 && Instant::now() < deadline {
            thread::sleep(POLL_INTERVAL);
        }
        assert_eq!(watch.count(&root), 1);
        watch.remove_root(&root);
        assert_eq!(watch.count(&root), 0);
    }

    /// A watched folder that is deleted asks for its parent to be re-read,
    /// because only the parent's listing shows it gone.
    #[test]
    fn a_deleted_watched_folder_reports_its_parent() {
        let tree = tempfile::tempdir().unwrap();
        let child = tree.path().join("child");
        fs::create_dir(&child).unwrap();
        let root = file_uri(tree.path());
        let child_uri = file_uri(&child);
        let watch = LocalWatch::start(WATCH_LIMIT).unwrap();
        watch.add(&root, &child_uri).unwrap();

        fs::remove_dir(&child).unwrap();

        let events = wait_for_events(&watch);
        let reported = events
            .iter()
            .any(|event| matches!(event, WatchEvent::Changed { folder, .. } if *folder == root));
        assert!(reported, "the parent is reported: {events:?}");
    }

    #[test]
    fn only_local_folders_are_watched() {
        let watch = LocalWatch::start(WATCH_LIMIT).unwrap();
        assert_eq!(
            watch.add("smb://nas/share", "smb://nas/share"),
            Err(WatchError::NotLocal)
        );
    }

    /// parity: SRCH-031
    #[test]
    fn a_symlink_to_a_folder_is_not_watched() {
        let tree = tempfile::tempdir().unwrap();
        let target = tempfile::tempdir().unwrap();
        let link = tree.path().join("link");
        std::os::unix::fs::symlink(target.path(), &link).unwrap();
        let watch = LocalWatch::start(WATCH_LIMIT).unwrap();

        let refused = watch.add(&file_uri(tree.path()), &file_uri(&link));

        assert!(matches!(refused, Err(WatchError::Refused(_))));
    }
}
