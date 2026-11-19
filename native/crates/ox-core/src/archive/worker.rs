// SPDX-License-Identifier: AGPL-3.0-only
//! Running blocking archive work off the main thread. Ports the
//! `start_worker` calls of the archive branches of `dispatch` in
//! `v2.0.0:desktop/winspace.py`, which run every listing, preview, check and
//! extraction on a worker thread.

use super::ArchiveError;

/// Runs `job` on a GIO worker thread and resolves with its result, so the
/// main loop stays responsive while an archive is read or written. The job
/// stops early when the [`Cancellation`](crate::transfer::Cancellation) it
/// captured is cancelled.
///
/// A job that panics read an archive no parser foresaw: it ends as
/// [`ArchiveError::DamagedArchive`], and the panic, already written to the
/// log by the panic hook, never reaches the main thread, which an
/// archive must not take down.
pub(super) async fn on_worker<T, F>(job: F) -> Result<T, ArchiveError>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, ArchiveError> + Send + 'static,
{
    gio::spawn_blocking(job)
        .await
        .unwrap_or(Err(ArchiveError::DamagedArchive))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A job that panics is a damaged archive, and the awaiting thread
    /// goes on.
    #[test]
    fn a_panicking_job_is_a_damaged_archive() {
        let context = glib::MainContext::new();
        let panicked = context.block_on(on_worker(|| -> Result<(), ArchiveError> {
            panic!("a parser met a header it never foresaw")
        }));
        assert!(matches!(panicked, Err(ArchiveError::DamagedArchive)));
        let fine = context.block_on(on_worker(|| Ok::<_, ArchiveError>(7)));
        assert!(matches!(fine, Ok(7)));
    }
}
