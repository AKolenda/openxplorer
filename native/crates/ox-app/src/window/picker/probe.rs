// SPDX-License-Identifier: AGPL-3.0-only
//! What an Open or Save dialog needs to know about a path, asked without
//! blocking the window.
//!
//! The dialog checks the caller's folder, the names typed in File name and
//! whether a saved file would replace another. On a network share that
//! stopped answering, a `stat` waits in the kernel for minutes, and on
//! the main thread it froze every window of the app. GIO answers these
//! questions on its own worker threads, and the dialog stops waiting
//! after [`PROBE_TIMEOUT`]: a share that has not answered by then is
//! reported as not answering.

use std::path::{Path, PathBuf};
use std::time::Duration;

use gtk::prelude::*;
use gtk::{gio, glib};

/// How long the dialog waits for a path's answer.
#[cfg(not(test))]
pub(super) const PROBE_TIMEOUT: Duration = Duration::from_secs(3);
/// Short in tests, which make paths that never answer.
#[cfg(test)]
pub(super) const PROBE_TIMEOUT: Duration = Duration::from_millis(300);

/// What is at a path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Probe {
    /// A folder.
    Folder,
    /// A file, or anything else that is not a folder.
    File,
    /// Nothing, or something that cannot be read.
    Missing,
    /// No answer within [`PROBE_TIMEOUT`]: a share that stopped answering.
    NoAnswer,
}

impl Probe {
    /// Whether something is at the path.
    pub(super) fn exists(self) -> bool {
        matches!(self, Self::Folder | Self::File)
    }
}

/// What is at `path`, asked on GIO's worker threads.
pub(super) async fn probe(path: &Path) -> Probe {
    match query(path, gio::FILE_ATTRIBUTE_STANDARD_TYPE).await {
        Answer::Info(info) if info.file_type() == gio::FileType::Directory => Probe::Folder,
        Answer::Info(_) => Probe::File,
        Answer::Failed => Probe::Missing,
        Answer::TimedOut => Probe::NoAnswer,
    }
}

/// Whether the user may write every one of `locations`, as GIO reports
/// it (`access::can-write`). Read-only when that cannot be read or does
/// not answer in time, so a sandboxed caller never gets more access than
/// it may have.
pub(super) async fn all_writable(locations: &[PathBuf]) -> bool {
    if locations.is_empty() {
        return false;
    }
    for location in locations {
        let writable = match query(location, gio::FILE_ATTRIBUTE_ACCESS_CAN_WRITE).await {
            Answer::Info(info) => info.boolean(gio::FILE_ATTRIBUTE_ACCESS_CAN_WRITE),
            Answer::Failed | Answer::TimedOut => false,
        };
        if !writable {
            return false;
        }
    }
    true
}

/// GIO's answer about one path.
enum Answer {
    Info(gio::FileInfo),
    Failed,
    TimedOut,
}

/// Asks GIO for `attributes` of `path`, waiting at most [`PROBE_TIMEOUT`].
/// Giving up cancels the query; a worker stuck in the kernel finishes on
/// its own.
async fn query(path: &Path, attributes: &str) -> Answer {
    #[cfg(test)]
    if never_answers(path) {
        glib::timeout_future(PROBE_TIMEOUT).await;
        return Answer::TimedOut;
    }
    let file = gio::File::for_path(path);
    let asking = file.query_info_future(attributes, gio::FileQueryInfoFlags::NONE, glib::Priority::DEFAULT);
    match glib::future_with_timeout(PROBE_TIMEOUT, asking).await {
        Ok(Ok(info)) => Answer::Info(info),
        Ok(Err(_)) => Answer::Failed,
        Err(_) => Answer::TimedOut,
    }
}

#[cfg(test)]
thread_local! {
    /// Folders whose paths, and everything below them, never answer.
    static NOT_ANSWERING: std::cell::RefCell<Vec<PathBuf>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// Makes `folder` and everything in it behave like a share that stopped
/// answering, for this test's thread.
#[cfg(test)]
pub(super) fn stop_answering(folder: &Path) {
    NOT_ANSWERING.with(|folders| folders.borrow_mut().push(folder.to_path_buf()));
}

#[cfg(test)]
fn never_answers(path: &Path) -> bool {
    NOT_ANSWERING.with(|folders| folders.borrow().iter().any(|folder| path.starts_with(folder)))
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    fn run<T>(future: impl std::future::Future<Output = T>) -> T {
        glib::MainContext::default().block_on(future)
    }

    /// parity: INT-032
    #[test]
    fn a_path_is_a_folder_a_file_missing_or_not_answering() {
        let folder = tempfile::tempdir().expect("temporary folder");
        let file = folder.path().join("notes.txt");
        fs::write(&file, "x").expect("a file");
        let dead = folder.path().join("share");
        fs::create_dir(&dead).expect("a folder");
        stop_answering(&dead);

        assert_eq!(run(probe(folder.path())), Probe::Folder);
        assert_eq!(run(probe(&file)), Probe::File);
        assert_eq!(run(probe(&folder.path().join("missing"))), Probe::Missing);
        assert_eq!(run(probe(&dead.join("report.odt"))), Probe::NoAnswer);
    }

    /// An Open reply says whether the user may write what was chosen;
    /// anything unknown, or on a share that does not answer, is read-only.
    ///
    /// parity: INT-032
    #[test]
    fn writable_means_every_choice_answered_that_it_may_be_written() {
        use std::os::unix::fs::PermissionsExt;

        let folder = tempfile::tempdir().expect("temporary folder");
        let writable = folder.path().join("notes.txt");
        let read_only = folder.path().join("signed.pdf");
        fs::write(&writable, "x").expect("a file");
        fs::write(&read_only, "x").expect("a file");
        fs::set_permissions(&read_only, fs::Permissions::from_mode(0o444)).expect("read-only");
        let dead = folder.path().join("share");
        fs::create_dir(&dead).expect("a folder");
        stop_answering(&dead);
        let is_root = fs::metadata("/proc/self").is_ok_and(|metadata| {
            use std::os::unix::fs::MetadataExt;
            metadata.uid() == 0
        });

        assert!(run(all_writable(std::slice::from_ref(&writable))));
        assert!(run(all_writable(&[folder.path().to_owned()])), "a folder");
        // root may write anything, so the read-only case holds for users only.
        if !is_root {
            assert!(!run(all_writable(&[writable.clone(), read_only])));
        }
        assert!(!run(all_writable(&[folder.path().join("missing")])), "unknown");
        assert!(
            !run(all_writable(&[writable, dead.join("a.txt")])),
            "not answering"
        );
        assert!(!run(all_writable(&[])), "nothing chosen");
    }
}
