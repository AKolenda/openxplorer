// SPDX-License-Identifier: AGPL-3.0-only
//! What a copy or move that never finished left behind (OPS-038).
//!
//! The Python app cancelled every job when its web interface crashed and
//! asked the user to inspect any `.winspace-transfer-*.part` folders
//! before removing them. The native app has no separate interface process;
//! what can still happen is that the app itself stops mid-copy (a crash, a
//! kill, a power cut). While a copy or move runs, a mark in the cache
//! directory names its destination folders, and the run holds a lock on
//! it. A mark that can be locked on the next start belongs to a run that
//! never finished; locks, unlike process ids, also tell that inside the
//! Flatpak's own process namespace and after a reboot. Its folders are
//! searched for the engine's private staging and backup items, and the
//! user is told where they are. Nothing is deleted, because the staging
//! item of an interrupted move may hold the only copy.

use std::fs;
use std::io::{self, Read, Write};
use std::path::PathBuf;

use gio::prelude::*;

use super::context::on_worker;
use super::error::OpsError;
use crate::random::random_hex;
use crate::transfer::{is_own_backup_name, is_own_staging_name};

/// The random bytes that keep two marks apart.
const MARK_BYTES: usize = 16;

/// The marks of running copies and moves, in one folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnfinishedMarks {
    folder: PathBuf,
}

/// The mark of one running copy or move, locked while it lives; dropping
/// it, when the run ends in any way but a crash, removes the mark.
#[derive(Debug)]
pub struct UnfinishedMark {
    path: PathBuf,
    /// The open mark, whose lock says that its run is still going.
    _lock: fs::File,
}

impl Drop for UnfinishedMark {
    fn drop(&mut self) {
        // A mark that cannot be removed is reported as a false alarm on
        // the next start, which lists nothing when no staging is left.
        let _ = fs::remove_file(&self.path);
    }
}

impl UnfinishedMarks {
    /// Marks kept in `folder`.
    pub fn new(folder: impl Into<PathBuf>) -> Self {
        Self {
            folder: folder.into(),
        }
    }

    /// Marks kept in the user's cache directory, beside the app's other
    /// `winspace` data. A cleared cache only loses a report.
    pub fn in_cache_directory() -> Self {
        Self::new(
            glib::user_cache_dir()
                .join("winspace")
                .join("unfinished-operations"),
        )
    }

    /// Marks a copy or move into `destinations` as running.
    ///
    /// # Errors
    ///
    /// When the mark cannot be written. The operation may still run; it
    /// only cannot be reported after a crash.
    pub fn mark(&self, destinations: &[String]) -> io::Result<UnfinishedMark> {
        fs::create_dir_all(&self.folder)?;
        let path = self.folder.join(random_hex(MARK_BYTES)?);
        let mut file = fs::File::create_new(&path)?;
        file.lock()?;
        file.write_all(destinations.join("\n").as_bytes())?;
        Ok(UnfinishedMark { path, _lock: file })
    }

    /// The staging and backup items that runs which never finished left in
    /// their destination folders, as URIs; their marks are removed.
    ///
    /// # Errors
    ///
    /// Only when the worker stops; unreadable marks and folders are
    /// skipped.
    pub async fn collect_leftovers(&self) -> Result<Vec<String>, OpsError> {
        let marks = self.clone();
        on_worker(move || Ok(marks.collect_leftovers_blocking())).await
    }

    /// [`Self::collect_leftovers`] on the calling thread.
    fn collect_leftovers_blocking(&self) -> Vec<String> {
        let Ok(entries) = fs::read_dir(&self.folder) else {
            return Vec::new();
        };
        let mut leftovers: Vec<String> = Vec::new();
        for entry in entries.flatten() {
            let path = entry.path();
            // A mark whose lock is held belongs to a run still going, in
            // this or another window's process.
            let Ok(mut mark) = fs::File::open(&path) else {
                continue;
            };
            if mark.try_lock().is_err() {
                continue;
            }
            let mut destinations = String::new();
            let _ = mark.read_to_string(&mut destinations);
            for folder in destinations.lines() {
                for item in own_leftovers_in(folder) {
                    if !leftovers.contains(&item) {
                        leftovers.push(item);
                    }
                }
            }
            let _ = fs::remove_file(&path);
        }
        leftovers
    }
}

/// The title of the dialog that reports leftovers.
pub const UNFINISHED_TITLE: &str = "File operations did not finish";

/// The most leftovers the dialog lists by name.
const LISTED_LEFTOVERS: usize = 10;

/// What the dialog says about `leftovers`, URIs from
/// [`UnfinishedMarks::collect_leftovers`]: local items by path.
pub fn leftovers_message(leftovers: &[String]) -> String {
    let mut lines: Vec<String> = leftovers
        .iter()
        .take(LISTED_LEFTOVERS)
        .map(|uri| {
            let file = gio::File::for_uri(uri);
            file.path()
                .map_or_else(|| uri.clone(), |path| path.display().to_string())
        })
        .collect();
    if leftovers.len() > LISTED_LEFTOVERS {
        lines.push(format!("…and {} more", leftovers.len() - LISTED_LEFTOVERS));
    }
    format!(
        "OpenXplorer stopped before a copy or move finished. It left these private items:\n\n{}\n\n\
         Inspect them before removing them: an interrupted move may have left the only copy of an \
         item there.",
        lines.join("\n")
    )
}

/// The staging and backup items directly in `folder`, without following
/// links.
fn own_leftovers_in(folder: &str) -> Vec<String> {
    let directory = gio::File::for_uri(folder);
    let Ok(listing) = directory.enumerate_children(
        gio::FILE_ATTRIBUTE_STANDARD_NAME,
        gio::FileQueryInfoFlags::NOFOLLOW_SYMLINKS,
        gio::Cancellable::NONE,
    ) else {
        return Vec::new();
    };
    let mut found = Vec::new();
    while let Ok(Some(info)) = listing.next_file(gio::Cancellable::NONE) {
        let name = info.name();
        let is_own = name
            .to_str()
            .is_some_and(|name| is_own_staging_name(name) || is_own_backup_name(name));
        if is_own {
            found.push(directory.child(&name).uri().to_string());
        }
    }
    let _ = listing.close(gio::Cancellable::NONE);
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    /// parity: OPS-038
    #[test]
    fn a_run_that_never_finished_reports_its_staging_and_a_finished_one_nothing() {
        let temp = tempfile::tempdir().expect("a temporary folder");
        let marks = UnfinishedMarks::new(temp.path().join("marks"));
        let target = temp.path().join("target");
        fs::create_dir(&target).expect("the target");
        let staging = format!(".winspace-transfer-{}.part", "a".repeat(32));
        fs::create_dir(target.join(&staging)).expect("the staging folder");
        fs::write(target.join("kept.txt"), b"kept").expect("an ordinary file");
        let target_uri = gio::File::for_path(&target).uri().to_string();

        drop(marks.mark(std::slice::from_ref(&target_uri)).expect("a mark"));
        let after_finished_run = marks.collect_leftovers_blocking();
        let running = marks.mark(std::slice::from_ref(&target_uri)).expect("a mark");
        let while_it_runs = marks.collect_leftovers_blocking();
        // A crash: the mark stays, and nothing holds its lock any more.
        let crashed = marks.folder.join("0123456789abcdef");
        fs::write(&crashed, &target_uri).expect("a crashed run's mark");
        let after_crash = marks.collect_leftovers_blocking();
        let once_reported = marks.collect_leftovers_blocking();

        assert!(after_finished_run.is_empty());
        assert!(while_it_runs.is_empty(), "a running copy keeps its mark");
        assert!(running.path.is_file(), "and it is not removed");
        assert!(!crashed.exists(), "a reported mark is removed");
        drop(running);
        assert_eq!(after_crash, [format!("{target_uri}/{staging}")]);
        assert!(once_reported.is_empty(), "the mark is removed once reported");
        assert!(target.join(&staging).is_dir(), "nothing is deleted");
        let message = leftovers_message(&after_crash);
        assert!(message.contains(&target.join(&staging).display().to_string()));
        assert!(message.ends_with("may have left the only copy of an item there."));
    }
}
