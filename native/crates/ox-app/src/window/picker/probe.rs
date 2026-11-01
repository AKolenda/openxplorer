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
    probe_within(path, PROBE_TIMEOUT).await
}

/// [`probe`], waiting at most `timeout`.
pub(super) async fn probe_within(path: &Path, timeout: Duration) -> Probe {
    match query_within(path, gio::FILE_ATTRIBUTE_STANDARD_TYPE, timeout).await {
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

/// Whether a Save dialog's caller can write its file, found out as
/// Windows' dialog does before it closes: by creating the file, then
/// removing it again.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum WriteTest {
    /// The file can be created there.
    Writable,
    /// A file of that name appeared while the test ran, created by
    /// another program: the dialog must look again and ask before
    /// replacing it, never take it for room to save.
    Appeared,
    /// The folder refuses new files: no permission, or a read-only share
    /// or drive.
    Denied,
    /// The file to replace is read-only.
    ReadOnlyFile,
    /// The drive or share is full.
    NoSpace,
    /// The folder cannot be reached through its local path: a `GVfs`
    /// FUSE export that went away, or a share that disconnected.
    Unavailable,
    /// No answer within [`PROBE_TIMEOUT`].
    NoAnswer,
}

/// Whether `name` can be saved in `folder`, through the local path the
/// caller gets, as Windows' Save dialog tests it unless the caller asks it
/// not to (`OFN_NOTESTFILECREATE`): it creates the file and removes it.
/// A file that already exists (`replacing`) is never opened for writing,
/// which on some FUSE filesystems would change it: it is checked with
/// [`test_replace`], and a hidden file of the dialog's own is created
/// beside it instead ([`test_folder`]).
///
/// It runs on a worker thread and waits at most [`PROBE_TIMEOUT`]. The
/// permission bits alone do not tell: `GVfs`'s FUSE export reports every
/// share writable, also one the server shares read-only.
pub(super) async fn test_write(folder: &Path, name: &std::ffi::OsStr, replacing: bool) -> WriteTest {
    let target = folder.join(name);
    if replacing {
        match test_replace(&target).await {
            WriteTest::Writable => test_folder(folder).await,
            refused => refused,
        }
    } else {
        on_a_worker(folder, move || create_and_remove(&target)).await
    }
}

/// Whether `folder` takes new files: a hidden file of the dialog's own is
/// created there and removed again.
pub(super) async fn test_folder(folder: &Path) -> WriteTest {
    let inside = folder.to_path_buf();
    on_a_worker(folder, move || {
        // A name that is taken already is another program's file: try another.
        for _ in 0..3 {
            let unique = glib::uuid_string_random();
            match create_and_remove(&inside.join(format!(".openxplorer-save-test-{unique}"))) {
                WriteTest::Appeared => {}
                test => return test,
            }
        }
        WriteTest::Unavailable
    })
    .await
}

/// Whether the existing file `target` may be replaced: it is not
/// read-only, by its permission bits or as the system answers for this
/// user (`access::can-write`, which also follows ACLs).
pub(super) async fn test_replace(target: &Path) -> WriteTest {
    let file = target.to_path_buf();
    let folder = target.parent().unwrap_or(target);
    on_a_worker(folder, move || {
        let read_only = std::fs::metadata(&file).is_ok_and(|metadata| metadata.permissions().readonly());
        let can_write = gio::File::for_path(&file)
            .query_info(
                gio::FILE_ATTRIBUTE_ACCESS_CAN_WRITE,
                gio::FileQueryInfoFlags::NONE,
                gio::Cancellable::NONE,
            )
            .map_or(true, |info| info.boolean(gio::FILE_ATTRIBUTE_ACCESS_CAN_WRITE));
        if read_only || !can_write {
            WriteTest::ReadOnlyFile
        } else {
            WriteTest::Writable
        }
    })
    .await
}

/// Runs `test` on a worker thread, waiting at most [`PROBE_TIMEOUT`]; a
/// test of a path in `folder` that never answers, in tests, does not run.
async fn on_a_worker(folder: &Path, test: impl FnOnce() -> WriteTest + Send + 'static) -> WriteTest {
    #[cfg(test)]
    if never_answers(folder) {
        glib::timeout_future(PROBE_TIMEOUT).await;
        return WriteTest::NoAnswer;
    }
    #[cfg(not(test))]
    let _ = folder;
    match glib::future_with_timeout(PROBE_TIMEOUT, gio::spawn_blocking(test)).await {
        Ok(Ok(test)) => test,
        Ok(Err(_)) => WriteTest::Unavailable,
        Err(_) => WriteTest::NoAnswer,
    }
}

/// Creates the new file `path` and removes it again. A file that is
/// there when it is created is another program's and is left alone
/// ([`WriteTest::Appeared`]). Where the file cannot be removed again, the
/// caller could not replace or rename its own file there either, so that
/// is a refusal, not room to save; the removal is tried twice first, so a
/// passing hiccup does not leave the file behind.
fn create_and_remove(path: &Path) -> WriteTest {
    use std::io::ErrorKind;
    #[cfg(test)]
    if is_listed(&APPEARING, path) {
        // Another program, such as a sync client, creates it first.
        let _ = std::fs::write(path, "written by another program");
    }
    let refusal = |error: &std::io::Error| match error.kind() {
        ErrorKind::PermissionDenied | ErrorKind::ReadOnlyFilesystem => WriteTest::Denied,
        ErrorKind::StorageFull | ErrorKind::QuotaExceeded => WriteTest::NoSpace,
        _ => WriteTest::Unavailable,
    };
    match std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
    {
        Ok(file) => {
            drop(file);
            // The file is empty and the dialog's own; the caller creates it
            // again when it saves.
            match remove(path).or_else(|_| remove(path)) {
                Ok(()) => WriteTest::Writable,
                Err(error) => refusal(&error),
            }
        }
        Err(error) if error.kind() == ErrorKind::AlreadyExists => WriteTest::Appeared,
        Err(error) => refusal(&error),
    }
}

/// Removes the write test's file `path`.
fn remove(path: &Path) -> std::io::Result<()> {
    #[cfg(test)]
    if is_listed(&UNREMOVABLE, path) {
        // A share that lets files be created but not deleted.
        return Err(std::io::ErrorKind::PermissionDenied.into());
    }
    std::fs::remove_file(path)
}

/// Paths another program creates just before the write test does, for
/// tests. Shared by every thread, as the test runs on GIO's workers.
#[cfg(test)]
static APPEARING: std::sync::Mutex<Vec<PathBuf>> = std::sync::Mutex::new(Vec::new());

/// Paths whose removal fails, for tests.
#[cfg(test)]
static UNREMOVABLE: std::sync::Mutex<Vec<PathBuf>> = std::sync::Mutex::new(Vec::new());

#[cfg(test)]
fn is_listed(list: &std::sync::Mutex<Vec<PathBuf>>, path: &Path) -> bool {
    list.lock()
        .is_ok_and(|paths| paths.iter().any(|listed| path.starts_with(listed)))
}

/// Makes another program create `path` just before the write test does.
#[cfg(test)]
pub(super) fn appear_when_tested(path: &Path) {
    if let Ok(mut paths) = APPEARING.lock() {
        paths.push(path.to_path_buf());
    }
}

/// Makes removing anything in `folder` fail, as on a share that lets
/// files be created but not deleted.
#[cfg(test)]
pub(super) fn fail_removal_in(folder: &Path) {
    if let Ok(mut paths) = UNREMOVABLE.lock() {
        paths.push(folder.to_path_buf());
    }
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
    query_within(path, attributes, PROBE_TIMEOUT).await
}

/// [`query`], waiting at most `timeout`.
async fn query_within(path: &Path, attributes: &str, timeout: Duration) -> Answer {
    #[cfg(test)]
    if never_answers(path) {
        glib::timeout_future(timeout).await;
        return Answer::TimedOut;
    }
    let file = gio::File::for_path(path);
    let asking = file.query_info_future(attributes, gio::FileQueryInfoFlags::NONE, glib::Priority::DEFAULT);
    match glib::future_with_timeout(timeout, asking).await {
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

    /// A file another program creates while the Save dialog tests the
    /// folder is not taken for room to save: it is left as it is, and the
    /// dialog is told it appeared, so it asks before replacing it.
    ///
    /// parity: INT-035
    #[gtk::test]
    fn a_file_that_appears_during_the_write_test_is_not_called_writable() {
        let folder = tempfile::tempdir().expect("temporary folder");
        let target = folder.path().join("report.txt");
        appear_when_tested(&target);

        let test = run(test_write(
            folder.path(),
            std::ffi::OsStr::new("report.txt"),
            false,
        ));

        assert_ne!(test, WriteTest::Writable);
        assert_eq!(
            fs::read_to_string(&target).expect("the other program's file"),
            "written by another program"
        );
    }

    /// Where the test file can be created but not removed again, as on a
    /// share that lets files be created but not deleted, the caller could
    /// not replace or rename its file either: the test says so rather than
    /// calling the folder writable.
    ///
    /// parity: INT-035
    #[gtk::test]
    fn a_test_file_that_cannot_be_removed_is_a_refusal() {
        let folder = tempfile::tempdir().expect("temporary folder");
        fs::write(folder.path().join("old.txt"), "x").expect("a file");
        fail_removal_in(folder.path());

        let new_file = run(test_write(
            folder.path(),
            std::ffi::OsStr::new("report.txt"),
            false,
        ));
        let replaced = run(test_write(folder.path(), std::ffi::OsStr::new("old.txt"), true));

        assert_eq!(new_file, WriteTest::Denied);
        assert_eq!(replaced, WriteTest::Denied);
    }

    /// parity: INT-032
    #[gtk::test]
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
    #[gtk::test]
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
