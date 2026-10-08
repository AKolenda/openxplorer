// SPDX-License-Identifier: AGPL-3.0-only
//! The local paths of the locations a dialog shows, found without making
//! the window wait on `GVfs`.
//!
//! The portal answers with local paths, so a share or device mounted by
//! `GVfs` is chosen through its FUSE path. GIO finds that path by asking
//! the `GVfs` daemon over D-Bus, which waits as long as the daemon is
//! busy or the share stopped answering. A dialog asks that on every
//! selection change, so the question goes to a worker thread, at most
//! once per location, and its answer is kept for the dialog's lifetime.
//! Until it comes, the location has no local path, and the dialog updates
//! itself when it does. An item inside a folder whose path is known has
//! that folder's path with its name added, so the items of a share are
//! never asked about one by one.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use gtk::gio;
use gtk::prelude::*;

use super::probe::PROBE_TIMEOUT;
use crate::locations::Page;

/// What is known about the local path of a location.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Known {
    /// It has this local path.
    Path(PathBuf),
    /// It has none: a landing page, or a location GIO gives no path.
    NoPath,
    /// Not known yet: `GVfs` has to be asked.
    Unknown,
}

impl Known {
    /// GIO's answer `path`.
    fn answer(path: Option<PathBuf>) -> Self {
        path.map_or(Self::NoPath, Self::Path)
    }

    /// The path, if it is known to have one.
    pub(super) fn path(self) -> Option<PathBuf> {
        match self {
            Self::Path(path) => Some(path),
            Self::NoPath | Self::Unknown => None,
        }
    }
}

/// What is known about the local path of `uri` without asking `GVfs`: a
/// local file's path, or no path for a landing page.
pub(super) fn path_at_once(uri: &str) -> Known {
    if Page::from_uri(uri).is_some() {
        return Known::NoPath;
    }
    let file = gio::File::for_uri(uri);
    // A local file's path is in its URI; nothing is asked.
    if file.has_uri_scheme("file") {
        Known::answer(file.path())
    } else {
        Known::Unknown
    }
}

/// Asks GIO for the local path of `uri` on a worker thread, waiting at
/// most [`PROBE_TIMEOUT`]. [`Known::Unknown`] when it did not answer in
/// time; the worker then finishes on its own.
pub(super) async fn ask_for_path(uri: &str) -> Known {
    #[cfg(test)]
    if uri.starts_with(NEVER_ANSWERS) {
        glib::timeout_future(PROBE_TIMEOUT).await;
        return Known::Unknown;
    }
    let uri = uri.to_owned();
    let asking = gio::spawn_blocking(move || gio::File::for_uri(&uri).path());
    match glib::future_with_timeout(PROBE_TIMEOUT, asking).await {
        Ok(Ok(path)) => Known::answer(path),
        Ok(Err(_)) | Err(_) => Known::Unknown,
    }
}

/// Locations under this prefix never answer, for tests of a share that
/// stopped answering.
#[cfg(test)]
pub(super) const NEVER_ANSWERS: &str = "sftp://never-answers.invalid/";

/// `uri` as GIO writes it, so a share root with or without its final
/// slash is one location.
fn key(uri: &str) -> String {
    gio::File::for_uri(uri).uri().to_string()
}

/// The local paths found so far in one dialog.
#[derive(Debug, Default)]
pub(super) struct LocalPaths {
    /// The answers so far: a path, or none for a location without one.
    known: RefCell<HashMap<String, Known>>,
    /// The locations being asked about now.
    asking: RefCell<HashSet<String>>,
}

impl LocalPaths {
    /// The local path of `uri` when it is known: at once, from an earlier
    /// answer, or from the known path of its folder.
    pub(super) fn known(&self, uri: &str) -> Known {
        let at_once = path_at_once(uri);
        if at_once != Known::Unknown {
            return at_once;
        }
        if let Some(known) = self.known.borrow().get(&key(uri)) {
            return known.clone();
        }
        let file = gio::File::for_uri(uri);
        let Some(folder) = file.parent() else {
            return Known::Unknown;
        };
        let folder_known = self.known.borrow().get(&key(&folder.uri())).cloned();
        match (folder_known, file.basename()) {
            (Some(Known::Path(folder_path)), Some(name)) => Known::Path(folder_path.join(name)),
            (Some(Known::NoPath), _) => Known::NoPath,
            _ => Known::Unknown,
        }
    }

    /// Keeps `known`, the answer for `uri`; an unknown answer may be
    /// asked again.
    pub(super) fn remember(&self, uri: &str, known: Known) {
        self.asking.borrow_mut().remove(&key(uri));
        if known != Known::Unknown {
            self.known.borrow_mut().insert(key(uri), known);
        }
    }

    /// Notes that `uri` is being asked about; false when it already is.
    pub(super) fn start_asking(&self, uri: &str) -> bool {
        self.asking.borrow_mut().insert(key(uri))
    }

    /// Notes that `uri` is no longer being asked about, for tests.
    #[cfg(test)]
    pub(super) fn stop_asking(&self, uri: &str) {
        self.asking.borrow_mut().remove(&key(uri));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// parity: INT-032
    #[test]
    fn local_files_and_pages_need_no_question_but_shares_do() {
        assert_eq!(
            path_at_once("file:///home/demo/Documents"),
            Known::Path(PathBuf::from("/home/demo/Documents"))
        );
        assert_eq!(path_at_once(Page::ThisPc.uri()), Known::NoPath);
        assert_eq!(path_at_once("smb://server.invalid/share"), Known::Unknown);
    }

    /// An item in a folder whose path is known has that folder's path.
    ///
    /// parity: INT-032
    #[test]
    fn an_item_takes_the_known_path_of_its_folder() {
        let paths = LocalPaths::default();
        let folder = "smb://server.invalid/share/Reports";
        assert_eq!(paths.known(folder), Known::Unknown, "not asked yet");
        assert!(paths.start_asking(folder));
        assert!(!paths.start_asking(folder), "asked once");

        paths.remember(
            folder,
            Known::Path(PathBuf::from("/run/user/1000/gvfs/share/Reports")),
        );

        assert_eq!(
            paths.known("smb://server.invalid/share/Reports/q3.ods"),
            Known::Path(PathBuf::from("/run/user/1000/gvfs/share/Reports/q3.ods"))
        );
        paths.remember("smb://server.invalid/other", Known::NoPath);
        assert_eq!(paths.known("smb://server.invalid/other/a.txt"), Known::NoPath);
    }
}
