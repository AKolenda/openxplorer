// SPDX-License-Identifier: AGPL-3.0-only
//! Watching a folder and every folder below it for changes, with one
//! inotify reader thread for all of them.
//!
//! New in the native app, for Gallery (HOME-021), which lists the photos of
//! a whole tree and must follow a photo saved into any of its folders. The
//! window's own folder watch (`ox-app`'s `folder_view::watch`) starts a
//! thread per folder, which a tree of hundreds of folders cannot afford;
//! this reuses the search cache's [`LocalWatch`] instead. As there, a
//! symbolic link is never followed, only local folders are watched, a
//! folder on another filesystem is not entered, and at most the limit of
//! folders is watched. No file is opened or read.

use std::collections::VecDeque;
use std::fs;
use std::os::unix::fs::MetadataExt;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, PoisonError};

use super::text::local_path;
use super::watch::{LocalWatch, WatchEvent};
use crate::location::file_uri;

/// The name every watch of a [`TreeWatch`] is recorded under.
const ROOT: &str = "tree";

/// Deepest level watched below a watched folder.
const MAX_DEPTH: usize = 128;

/// inotify watches over folder trees. Cloning shares the watches.
#[derive(Debug, Clone)]
pub struct TreeWatch {
    watch: Arc<Mutex<LocalWatch>>,
    limit: usize,
}

impl TreeWatch {
    /// Starts a watch of at most `limit` folders in all.
    ///
    /// # Errors
    ///
    /// When inotify cannot be started.
    pub fn start(limit: usize) -> std::io::Result<Self> {
        Ok(Self {
            watch: Arc::new(Mutex::new(LocalWatch::start(limit)?)),
            limit,
        })
    }

    /// Watches the local folder `uri` and the folders below it on the same
    /// filesystem, as far as the limit allows; folders already watched
    /// stay watched. Blocks on the file system: call it off the main
    /// thread. Returns how many folders are watched now.
    pub fn watch_tree(&self, uri: &str, cancellable: &gio::Cancellable) -> usize {
        let Some(root) = local_path(uri) else {
            return self.count();
        };
        let Ok(metadata) = fs::symlink_metadata(&root) else {
            return self.count();
        };
        if !metadata.is_dir() {
            return self.count();
        }
        let device = metadata.dev();
        let mut pending: VecDeque<(PathBuf, usize)> = VecDeque::from([(root, 0)]);
        while let Some((folder, depth)) = pending.pop_front() {
            if gio::prelude::CancellableExt::is_cancelled(cancellable) || self.count() >= self.limit {
                break;
            }
            if self.lock().add(ROOT, &file_uri(&folder)).is_err() {
                continue;
            }
            if depth >= MAX_DEPTH {
                continue;
            }
            let Ok(children) = fs::read_dir(&folder) else {
                continue;
            };
            for child in children.flatten() {
                let Ok(metadata) = fs::symlink_metadata(child.path()) else {
                    continue;
                };
                if metadata.is_dir() && metadata.dev() == device {
                    pending.push_back((child.path(), depth + 1));
                }
            }
        }
        self.count()
    }

    /// How many folders are watched.
    pub fn count(&self) -> usize {
        self.lock().count(ROOT)
    }

    /// Whether anything in a watched folder changed since the last call,
    /// or the kernel dropped events, after which anything may have.
    pub fn take_changes(&self) -> bool {
        let events = self.lock().take_events();
        events
            .iter()
            .any(|event| matches!(event, WatchEvent::Changed { .. } | WatchEvent::Overflowed))
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, LocalWatch> {
        self.watch.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use super::*;

    fn wait_for_changes(watch: &TreeWatch) -> bool {
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            if watch.take_changes() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        false
    }

    /// A file saved into a folder deep in the tree is seen; a link to a
    /// folder outside it is not followed, and the limit holds.
    ///
    /// parity: HOME-021
    #[test]
    fn a_change_anywhere_in_the_tree_is_seen_and_links_are_not_followed() {
        let root = tempfile::tempdir().expect("a folder");
        let outside = tempfile::tempdir().expect("another folder");
        let deep = root.path().join("Trips/2025/Coast");
        fs::create_dir_all(&deep).expect("folders");
        std::os::unix::fs::symlink(outside.path(), root.path().join("Elsewhere")).expect("a link");
        let watch = TreeWatch::start(64).expect("inotify");

        let watched = watch.watch_tree(&file_uri(root.path()), &gio::Cancellable::new());

        assert_eq!(watched, 4, "the root, Trips, 2025 and Coast");
        assert!(!watch.take_changes());
        fs::write(deep.join("dune.jpg"), "x").expect("a file");
        assert!(wait_for_changes(&watch));
        fs::write(outside.path().join("other.jpg"), "x").expect("a file");
        std::thread::sleep(Duration::from_millis(300));
        assert!(!watch.take_changes(), "the linked folder is not watched");

        let small = TreeWatch::start(2).expect("inotify");
        assert_eq!(
            small.watch_tree(&file_uri(root.path()), &gio::Cancellable::new()),
            2
        );
    }
}
