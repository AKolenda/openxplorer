// SPDX-License-Identifier: AGPL-3.0-only
//! Whether dragged items are on the drive of the folder they are dropped
//! on, which decides what a plain drag does (DND-017): Windows Explorer
//! moves items dragged within a drive and copies them to another drive.
//!
//! A drive is a local filesystem, told apart by its device number, which
//! is also what decides whether a rename can move an item. Only local
//! `file:` items count: a network folder, a mount of the session's GIO
//! daemons and a document portal path share one device number for many
//! shares or files, so a drop there, or from there, stays a copy.
//!
//! Reading a file's device number can block for long on a kernel mount of
//! a share that stopped answering, so it is never read on the GTK thread:
//! the drop reads it once its items are known, on a worker thread and for
//! a limited time ([`answer_within`]).

use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::time::Duration;

use gtk::prelude::*;
use gtk::{gio, glib};

/// What `check` answers on a worker thread, or false when it does not
/// answer within `timeout`; the GTK thread goes on meanwhile.
pub(super) async fn answer_within(timeout: Duration, check: impl FnOnce() -> bool + Send + 'static) -> bool {
    let worker = gio::spawn_blocking(check);
    glib::future_with_timeout(timeout, worker)
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
            .all(|uri| drive_of(uri, &runtime, false) == Some(drive))
}

/// The device number of the local file `uri`, or `None` when it is not a
/// plain local file. A dragged symbolic link is on the drive of the folder
/// holding it (`follow` false); a destination folder is where its link
/// points (`follow` true).
fn drive_of(uri: &str, runtime: &Path, follow: bool) -> Option<u64> {
    let path = local_file_path(uri, runtime)?;
    let metadata = if follow {
        std::fs::metadata(&path)
    } else {
        std::fs::symlink_metadata(&path)
    };
    metadata.ok().map(|metadata| metadata.dev())
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
        let context = glib::MainContext::default();
        let ticks = std::rc::Rc::new(std::cell::Cell::new(0));
        let counter = ticks.clone();
        let tick = glib::timeout_add_local(Duration::from_millis(20), move || {
            counter.set(counter.get() + 1);
            glib::ControlFlow::Continue
        });
        let main_thread = std::thread::current().id();

        let slow = context.block_on(answer_within(Duration::from_millis(300), || {
            std::thread::sleep(Duration::from_secs(2));
            true
        }));
        let checked_on = context.block_on(answer_within(Duration::from_secs(5), move || {
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
}
