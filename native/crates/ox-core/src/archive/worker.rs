// SPDX-License-Identifier: AGPL-3.0-only
//! Running blocking archive work off the main thread. Ports the
//! `start_worker` calls of the archive branches of `dispatch` in
//! `desktop/winspace.py`, which run every listing, preview, check and
//! extraction on a worker thread.

/// Runs `job` on a GIO worker thread and resolves with its result, so the
/// main loop stays responsive while an archive is read or written. The job
/// stops early when the [`Cancellation`](crate::transfer::Cancellation) it
/// captured is cancelled.
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
