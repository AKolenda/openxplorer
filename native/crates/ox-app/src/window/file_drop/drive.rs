// SPDX-License-Identifier: AGPL-3.0-only
//! Whether dragged items are on the drive of the folder they are dropped
//! on, which decides what a plain drag does (DND-017): Windows Explorer
//! moves items dragged within a drive and copies them to another drive.
//!
//! A drive is one mount of a local filesystem
//! ([`ox_core::drive::Drive`]), which is also what decides whether a
//! rename can move an item. Only local `file:` items count: a network
//! folder, a mount of the session's GIO daemons and a document portal path
//! share one device number for many shares or files, and a kernel mount of
//! a share is a network place too, so a drop there, or from there, stays a
//! copy.
//!
//! Reading a file's device number can block for long on a kernel mount of
//! a share that stopped answering, so it is never read on the GTK thread:
//! the drop reads it once its items are known, on a thread of its own and
//! for a limited time, with only a few such reads at once
//! ([`answer_within`]). A path the mount table already places on a share
//! is not read at all.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use gtk::prelude::*;
use gtk::{gio, glib};
use ox_core::drive::Drive;

/// How many drive reads may run at once. Each runs on a thread of its
/// own, never on GIO's shared threads: a read stuck on a share that
/// stopped answering cannot be stopped, so it must not hold a thread the
/// rest of the app needs, and only a few may linger.
pub(super) struct ProbeLimit {
    /// How many run now, stuck ones included.
    running: AtomicUsize,
    /// The most that may run.
    max: usize,
}

impl ProbeLimit {
    /// At most `max` reads at once.
    pub(super) const fn new(max: usize) -> Self {
        Self {
            running: AtomicUsize::new(0),
            max,
        }
    }

    /// A place for one more read, or `None` when `max` already run.
    fn take(&'static self) -> Option<ProbeSlot> {
        self.running
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |running| {
                (running < self.max).then_some(running + 1)
            })
            .ok()
            .map(|_| ProbeSlot(self))
    }
}

/// One running read's place, given back when it ends.
struct ProbeSlot(&'static ProbeLimit);

impl Drop for ProbeSlot {
    fn drop(&mut self) {
        self.0.running.fetch_sub(1, Ordering::SeqCst);
    }
}

/// The drive reads of drops: a few, as each normally takes microseconds.
pub(super) static DRIVE_PROBES: ProbeLimit = ProbeLimit::new(4);

/// What `check` answers on a thread of its own, or false when it does not
/// answer within `timeout` or `limit` reads already run; the GTK thread
/// goes on meanwhile.
pub(super) async fn answer_within(
    limit: &'static ProbeLimit,
    timeout: Duration,
    check: impl FnOnce() -> bool + Send + 'static,
) -> bool {
    let Some(slot) = limit.take() else {
        return false;
    };
    let (sender, receiver) = async_channel::bounded(1);
    let started = std::thread::Builder::new()
        .name("drive-read".to_owned())
        .spawn(move || {
            let answer = check();
            drop(slot);
            let _ = sender.try_send(answer);
        });
    if started.is_err() {
        return false;
    }
    glib::future_with_timeout(timeout, receiver.recv())
        .await
        .ok()
        .and_then(Result::ok)
        .unwrap_or(false)
}

/// True when every item of `uris` is on the local drive of `folder`. It
/// reads file metadata, so call it off the GTK thread.
/// An empty list, an item that cannot be read, or any item or folder that
/// is not a local file is false, so the drop copies.
pub(super) fn on_same_drive(uris: &[String], folder: &str) -> bool {
    let runtime = glib::user_runtime_dir();
    let Some(drive) = drive_of(folder, &runtime, true) else {
        return false;
    };
    !uris.is_empty()
        && uris
            .iter()
            .all(|uri| drive_of(uri, &runtime, false).is_some_and(|item| item.is_same_local_drive(&drive)))
}

/// The drive of the local file `uri`, or `None` when it is not a plain
/// local file. A dragged symbolic link is on the drive of the folder
/// holding it (`follow` false); a destination folder is where its link
/// points (`follow` true).
fn drive_of(uri: &str, runtime: &Path, follow: bool) -> Option<Drive> {
    let path = local_file_path(uri, runtime)?;
    ox_core::drive::drive_of(&path, follow)
}

/// The path of `uri` when it is a `file:` address outside the session's
/// runtime folder, which holds the GIO daemons' mounts and the document
/// portal.
fn local_file_path(uri: &str, runtime: &Path) -> Option<PathBuf> {
    let scheme = uri.split(':').next()?;
    if !scheme.eq_ignore_ascii_case("file") {
        return None;
    }
    let path = gio::File::for_uri(uri).path()?;
    (!path.starts_with(runtime)).then_some(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The `file:` URI of `path`.
    fn uri_of(path: &Path) -> String {
        gio::File::for_path(path).uri().to_string()
    }

    /// parity: DND-017
    #[test]
    fn items_in_one_folder_are_on_its_drive_and_an_empty_drop_is_not() {
        let folder = tempfile::tempdir().expect("a temporary folder");
        let item = folder.path().join("Notes.txt");
        std::fs::write(&item, "notes").expect("the item is written");
        let destination = folder.path().join("Documents");
        std::fs::create_dir(&destination).expect("the folder is made");

        assert!(on_same_drive(&[uri_of(&item)], &uri_of(&destination)));
        assert!(!on_same_drive(&[], &uri_of(&destination)));
    }

    /// parity: DND-017
    #[test]
    fn network_missing_and_runtime_items_are_never_on_the_drive() {
        let folder = tempfile::tempdir().expect("a temporary folder");
        let destination = uri_of(folder.path());
        let missing = uri_of(&folder.path().join("Gone.txt"));
        let runtime = glib::user_runtime_dir();
        let portal = uri_of(&runtime.join("doc/1234/Notes.txt"));

        assert!(!on_same_drive(
            &["smb://nas/share/Notes.txt".to_owned()],
            &destination
        ));
        assert!(!on_same_drive(&[missing], &destination));
        assert!(!on_same_drive(&[uri_of(folder.path())], "smb://nas/share/"));
        assert_eq!(local_file_path(&portal, &runtime), None);
    }

    /// A drive that does not answer in time counts as another, and the
    /// GTK thread goes on while it is read.
    ///
    /// parity: DND-017
    #[gtk::test]
    fn a_drive_that_does_not_answer_in_time_is_another() {
        static LIMIT: ProbeLimit = ProbeLimit::new(4);
        let context = glib::MainContext::default();
        let ticks = std::rc::Rc::new(std::cell::Cell::new(0));
        let counter = ticks.clone();
        let tick = glib::timeout_add_local(Duration::from_millis(20), move || {
            counter.set(counter.get() + 1);
            glib::ControlFlow::Continue
        });
        let main_thread = std::thread::current().id();

        let slow = context.block_on(answer_within(&LIMIT, Duration::from_millis(300), || {
            std::thread::sleep(Duration::from_secs(2));
            true
        }));
        let checked_on = context.block_on(answer_within(&LIMIT, Duration::from_secs(5), move || {
            std::thread::current().id() != main_thread
        }));
        tick.remove();

        assert!(!slow, "too late counts as another drive");
        assert!(ticks.get() >= 5, "the GTK thread went on: {} ticks", ticks.get());
        assert!(checked_on, "the drive is read on a worker thread");
    }

    /// parity: DND-017
    #[test]
    fn a_dragged_link_is_on_the_drive_of_the_folder_holding_it() {
        let folder = tempfile::tempdir().expect("a temporary folder");
        let link = folder.path().join("Link");
        std::os::unix::fs::symlink("/proc/self", &link).expect("the link is made");

        assert!(on_same_drive(&[uri_of(&link)], &uri_of(folder.path())));
    }

    /// A drive that never answers, such as a share that stopped answering,
    /// ties up only the drive reads' own threads, never GIO's shared ones,
    /// and at most a few: past that, a drop copies without reading, until
    /// the stuck reads end.
    ///
    /// parity: DND-017
    #[gtk::test]
    fn a_drive_that_never_answers_ties_up_at_most_a_few_threads_of_its_own() {
        static LIMIT: ProbeLimit = ProbeLimit::new(2);
        let context = glib::MainContext::default();
        let gate = std::sync::Arc::new(std::sync::Mutex::new(()));
        let closed = gate.lock().expect("the gate");
        let names = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        for _ in 0..2 {
            let (gate, names) = (gate.clone(), names.clone());
            let answered = context.block_on(answer_within(&LIMIT, Duration::from_millis(100), move || {
                let name = std::thread::current().name().map(str::to_owned);
                names.lock().expect("the names").push(name);
                let _stuck = gate.lock();
                true
            }));
            assert!(!answered, "a stuck read counts as another drive");
        }
        let ran = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let third_ran = ran.clone();
        let third = context.block_on(answer_within(&LIMIT, Duration::from_secs(1), move || {
            third_ran.store(true, std::sync::atomic::Ordering::SeqCst);
            true
        }));

        assert!(!third, "past the limit a drop copies");
        assert!(
            !ran.load(std::sync::atomic::Ordering::SeqCst),
            "and reads nothing"
        );
        let names = names.lock().expect("the names").clone();
        assert_eq!(
            names,
            vec![Some("drive-read".to_owned()); 2],
            "the reads' own threads"
        );

        drop(closed);
        let freed = context.block_on(answer_within(&LIMIT, Duration::from_secs(5), || true));
        let mut tries = 0;
        let mut answered = freed;
        while !answered && tries < 50 {
            std::thread::sleep(Duration::from_millis(100));
            answered = context.block_on(answer_within(&LIMIT, Duration::from_secs(5), || true));
            tries += 1;
        }
        assert!(answered, "once the stuck reads end, drives are read again");
    }
}
