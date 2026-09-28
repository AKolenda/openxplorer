// SPDX-License-Identifier: AGPL-3.0-only
//! What every file operation runs with, where its blocking work runs, and
//! how its yes-or-no questions to GIO honour the cancellation.
//!
//! The Python bridge gives each write a `GioCancellation` and the
//! `assert_writable` guard of `desktop/previous_versions.py`, and runs it
//! on a worker thread (`start_worker` in `desktop/winspace.py`). Here an
//! [`OperationContext`] carries the first two, and [`on_worker`] runs the
//! blocking part on GIO's pool of blocking-I/O threads, so the GTK main
//! loop keeps running while the caller awaits the result.

use std::fmt;
use std::sync::Arc;

use super::error::OpsError;
use crate::transfer::{
    check_write_tree, Cancellation, Node, SourceChange, TransferEngine, TransferError, WriteGuard,
};

/// Locations that must never change, such as previous versions
/// (snapshots). The app installs the check of its previous-versions
/// settings; every operation asks it before it changes anything.
#[derive(Clone, Default)]
pub struct WriteProtection {
    guard: Option<Arc<WriteGuard>>,
}

impl WriteProtection {
    /// No location is protected. Tests and locations without previous
    /// versions use this.
    pub fn unrestricted() -> Self {
        Self::default()
    }

    /// Protects every location `guard` refuses. The guard returns the
    /// refusal the user sees, like the previous-versions refusal of
    /// `PreviousVersions.assert_writable` in `desktop/previous_versions.py`.
    pub fn new(guard: impl Fn(&str) -> Result<(), TransferError> + Send + Sync + 'static) -> Self {
        Self {
            guard: Some(Arc::new(guard)),
        }
    }

    /// Refuses a protected `uri`.
    ///
    /// # Errors
    ///
    /// The guard's refusal.
    pub(crate) fn check(&self, uri: &str) -> Result<(), OpsError> {
        match &self.guard {
            Some(guard) => guard(uri).map_err(OpsError::from),
            None => Ok(()),
        }
    }

    /// XFER-020: asks the protection about `destination` and every location
    /// below it, paired with `source`'s tree, before anything changes; about
    /// `source`'s tree too when `source_change` is [`SourceChange::Changed`].
    /// Nothing is followed through links. This is the transfer engine's own
    /// preflight, so the rule is enforced by one walk everywhere.
    ///
    /// # Errors
    ///
    /// The refusal for the first protected location, the nesting limit,
    /// cancellation, or a failure to inspect or list the tree.
    pub(crate) fn check_tree(
        &self,
        source: &dyn Node,
        destination: &dyn Node,
        cancel: &Cancellation,
        source_change: SourceChange,
    ) -> Result<(), OpsError> {
        check_write_tree(
            self.guard.as_deref(),
            source,
            Some(destination),
            cancel,
            source_change,
        )?;
        Ok(())
    }

    /// True when some location is protected.
    pub(crate) fn is_restricted(&self) -> bool {
        self.guard.is_some()
    }

    /// `engine` with this protection as its write guard, which it checks
    /// for every item of every tree before the item changes (XFER-020).
    pub(crate) fn install(&self, engine: TransferEngine) -> TransferEngine {
        match &self.guard {
            Some(guard) => {
                let guard = Arc::clone(guard);
                engine.with_write_guard(move |uri: &str| guard(uri))
            }
            None => engine,
        }
    }
}

impl fmt::Debug for WriteProtection {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("WriteProtection")
            .field("is_restricted", &self.is_restricted())
            .finish()
    }
}

/// The user's cancellation and the app's write protection for one
/// operation.
#[derive(Debug, Clone, Default)]
pub struct OperationContext {
    /// Stops the operation between steps and aborts in-flight GIO calls.
    /// Keep a clone to cancel from the interface.
    pub cancel: Cancellation,
    /// Locations the operation must not change.
    pub protection: WriteProtection,
}

impl OperationContext {
    /// A context with a fresh cancellation and the given protection.
    pub fn new(protection: WriteProtection) -> Self {
        Self {
            cancel: Cancellation::new(),
            protection,
        }
    }

    /// The GIO cancellable behind [`OperationContext::cancel`], for GIO
    /// calls.
    pub(crate) fn cancellable(&self) -> &gio::Cancellable {
        self.cancel.cancellable()
    }
}

/// Shown when blocking work ended without a result: a bug, never a user
/// error, reported instead of taking the window down.
const WORKER_STOPPED: &str = "The file operation stopped unexpectedly. Check the folder before trying again.";

/// Runs `work` on GIO's pool of blocking-I/O threads and waits for it
/// without blocking the caller's main loop.
///
/// # Errors
///
/// The error `work` returns, or a failure when `work` panicked.
pub(crate) async fn on_worker<T, F>(work: F) -> Result<T, OpsError>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, OpsError> + Send + 'static,
{
    match gio::spawn_blocking(work).await {
        Ok(outcome) => outcome,
        Err(_panic) => Err(OpsError::failed(WORKER_STOPPED)),
    }
}

/// The answer of `query`, a yes-or-no question to GIO, unless the user
/// cancelled while it ran.
///
/// Queries such as `Node::exists` and `Node::can_trash` answer "no" when
/// GIO fails, and GIO fails a query the user's cancellation abandoned. Read
/// as an answer, that "no" would make a taken name look free or a folder
/// look as if it had no Trash, and could start the very step the user just
/// stopped. So every such query is asked through this function, which
/// reports the cancellation instead.
///
/// # Errors
///
/// [`OpsError::Cancelled`] when `cancel` is cancelled once `query` has
/// returned, whatever it answered.
pub(crate) fn unless_cancelled<T>(cancel: &Cancellation, query: impl FnOnce() -> T) -> Result<T, OpsError> {
    let answer = query();
    cancel.check()?;
    Ok(answer)
}

#[cfg(test)]
mod tests {
    use super::*;

    const READ_ONLY: &str = "Previous-version locations are read-only in OpenXplorer.";

    /// A protection refusing everything below `/snapshots`.
    fn snapshot_protection() -> WriteProtection {
        WriteProtection::new(|uri: &str| {
            if uri.starts_with("file:///snapshots") {
                Err(TransferError::failed(READ_ONLY))
            } else {
                Ok(())
            }
        })
    }

    #[test]
    fn protection_refuses_only_what_its_guard_refuses() {
        let protection = snapshot_protection();

        let protected = protection.check("file:///snapshots/old");
        let writable = protection.check("file:///home/user");

        assert_eq!(protected, Err(OpsError::Failed(READ_ONLY.into())));
        assert_eq!(writable, Ok(()));
        assert!(protection.is_restricted());
        assert_eq!(WriteProtection::unrestricted().check("file:///snapshots"), Ok(()));
    }

    #[test]
    fn worker_results_and_panics_reach_the_caller() {
        let context = glib::MainContext::new();

        let value = context.block_on(on_worker(|| Ok(7)));
        let refused = context.block_on(on_worker::<(), _>(|| Err(OpsError::Cancelled)));
        let panicked = context.block_on(on_worker::<(), _>(|| {
            panic!("deliberate test panic: a worker bug must not take the window down")
        }));

        assert_eq!(value, Ok(7));
        assert_eq!(refused, Err(OpsError::Cancelled));
        assert_eq!(panicked, Err(OpsError::failed(WORKER_STOPPED)));
    }

    #[test]
    fn a_query_answered_after_a_cancellation_reports_the_cancellation() {
        let live = Cancellation::new();
        let cancelled = Cancellation::new();

        let answered = unless_cancelled(&live, || false);
        let abandoned = unless_cancelled(&cancelled, || {
            cancelled.cancel();
            false
        });

        assert_eq!(answered, Ok(false));
        assert_eq!(abandoned, Err(OpsError::Cancelled));
    }
}
