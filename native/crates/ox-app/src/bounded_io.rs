// SPDX-License-Identifier: AGPL-3.0-only
//! Looking at a file from the main thread without ever freezing the
//! window on it.
//!
//! A path that `gio::File::path` gives is not always on this computer: an
//! SMB or NFS share mounted by the kernel, sshfs and the FUSE export of `GVfs`
//! all have paths, and a server that stops answering blocks every `stat`,
//! `open` and `read` of them for as long as it is gone, from seconds to
//! minutes. Where the main thread needs a quick fact about a file (is it a
//! link, a folder, a launcher), [`wait_for`] asks a worker thread and waits
//! at most a fraction of a second: a local file answers in microseconds,
//! and a file that does not answer in time is taken as unknown, so the
//! window carries on without that fact. The worker is left to finish, or
//! to wait on the share, on its own.

use std::sync::mpsc;
use std::time::Duration;

/// How long the main thread waits for a fact about a file: long enough
/// for any disk, short enough that nobody sees a freeze.
pub(crate) const MAIN_THREAD_WAIT: Duration = Duration::from_millis(400);

/// What `job`, which looks at the file `key` names (a path or a URI),
/// returns, if it returns within `wait`; `None` when it did not (the file
/// is on a share that stopped answering) or its thread could not start.
/// `job` runs on a thread of its own, never the caller's.
pub(crate) fn wait_for<T: Send + 'static>(
    key: &str,
    wait: Duration,
    job: impl FnOnce() -> T + Send + 'static,
) -> Option<T> {
    let (sender, receiver) = mpsc::sync_channel(1);
    #[cfg(test)]
    let stalled = stall::is_stalled(key);
    #[cfg(not(test))]
    let _ = key;
    std::thread::Builder::new()
        .name("ox-file-look".to_owned())
        .spawn(move || {
            #[cfg(test)]
            if stalled {
                // A share that stopped answering, as a test sets it up.
                std::thread::sleep(stall::STALL);
            }
            let _ = sender.send(job());
        })
        .ok()?;
    receiver.recv_timeout(wait).ok()
}

/// Tests' stand-in for a mount whose server stopped answering: a test
/// names a folder, and every look at a file under it through
/// [`wait_for`] waits [`stall::STALL`] before it starts, as `stat` waits on
/// a dead sshfs or NAS mount. (A real stalled FUSE or network mount needs
/// privileges a test does not have.)
#[cfg(test)]
pub(crate) mod stall {
    use std::sync::Mutex;
    use std::time::Duration;

    /// How long a look at a stalled file waits.
    pub(crate) const STALL: Duration = Duration::from_secs(30);

    static STALLED: Mutex<Vec<String>> = Mutex::new(Vec::new());

    /// Makes every file under `folder` (a path, and its `file://` URI)
    /// stall until the returned guard is dropped.
    pub(crate) fn stall(folder: &std::path::Path) -> Stalled {
        let path = folder.to_string_lossy().into_owned();
        let uri = gtk::glib::filename_to_uri(folder, None).map_or_else(|_| path.clone(), String::from);
        let keys = vec![path, uri];
        STALLED
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .extend(keys.iter().cloned());
        Stalled(keys)
    }

    /// Whether `key` is under a stalled folder.
    pub(super) fn is_stalled(key: &str) -> bool {
        STALLED
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .iter()
            .any(|folder| key.starts_with(folder.as_str()))
    }

    /// Ends the stall when dropped.
    pub(crate) struct Stalled(Vec<String>);

    impl Drop for Stalled {
        fn drop(&mut self) {
            let mut stalled = STALLED.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            stalled.retain(|folder| !self.0.contains(folder));
        }
    }

    /// Duration helper for tests asserting a look gave up in time.
    pub(crate) fn well_within(wait: Duration) -> Duration {
        wait + Duration::from_secs(2)
    }
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use super::*;

    /// A look that answers is returned; one under a stalled folder gives
    /// up after the wait, and the caller goes on.
    #[test]
    fn a_look_that_does_not_answer_gives_up_in_time() {
        let folder = tempfile::tempdir().expect("a temporary folder");
        let path = folder.path().join("Notes.txt");
        std::fs::write(&path, "x").expect("written");
        let key = path.to_string_lossy().into_owned();
        let looked = path.clone();
        assert_eq!(
            wait_for(&key, MAIN_THREAD_WAIT, move || looked.is_file()),
            Some(true)
        );
        let _stalled = stall::stall(folder.path());
        let started = Instant::now();
        let looked = path.clone();
        assert_eq!(wait_for(&key, MAIN_THREAD_WAIT, move || looked.is_file()), None);
        assert!(started.elapsed() < stall::well_within(MAIN_THREAD_WAIT));
    }
}
