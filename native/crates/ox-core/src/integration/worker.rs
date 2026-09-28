// SPDX-License-Identifier: AGPL-3.0-only
//! Running blocking integration work off the main thread. Ports the
//! `start_worker` calls of the integration branches of `dispatch` in
//! `desktop/winspace.py`, which run every file-association, Brave,
//! opening and terminal request on a worker thread.

/// Runs `job` on a GIO worker thread and resolves with its result, so the
/// main loop stays responsive while `xdg-mime` runs, a folder is queried
/// or Brave's preferences are written.
///
/// # Panics
///
/// Resumes the job's panic on the awaiting thread: a panicking job is a
/// bug, and it must not look like a finished one.
pub(super) async fn on_worker<T, F>(job: F) -> T
where
    T: Send + 'static,
    F: FnOnce() -> T + Send + 'static,
{
    match gio::spawn_blocking(job).await {
        Ok(value) => value,
        Err(panic) => std::panic::resume_unwind(panic),
    }
}
