// SPDX-License-Identifier: AGPL-3.0-only
//! Cooperative cancellation of a running operation. Ports `Cancellation` in
//! `v2.0.0:desktop/operations.py`.
//!
//! The token wraps the [`gio::Cancellable`] that in-flight GIO calls receive,
//! so cancelling it both stops the engine between steps and aborts a copy
//! between blocks (OPS-022).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use gio::prelude::*;

use super::error::TransferError;

/// How often a paused operation looks whether it may go on.
const PAUSE_POLL: Duration = Duration::from_millis(50);

/// Cooperative cancellation shared with in-flight GIO calls, and the
/// pause of the operation it belongs to (OPS-021).
#[derive(Debug, Clone, Default)]
pub struct Cancellation {
    cancellable: gio::Cancellable,
    paused: Arc<AtomicBool>,
}

impl Cancellation {
    /// A fresh, not yet cancelled token.
    pub fn new() -> Self {
        Self::default()
    }

    /// Requests cancellation. In-flight GIO calls using this token abort.
    pub fn cancel(&self) {
        self.cancellable.cancel();
    }

    /// True once [`Cancellation::cancel`] was called.
    pub fn is_cancelled(&self) -> bool {
        self.cancellable.is_cancelled()
    }

    /// Stops an operation between two steps once the user cancelled.
    ///
    /// # Errors
    ///
    /// [`TransferError::Cancelled`] once [`Cancellation::cancel`] was called.
    pub fn check(&self) -> Result<(), TransferError> {
        if self.is_cancelled() {
            Err(TransferError::Cancelled)
        } else {
            Ok(())
        }
    }

    /// Pauses the operation: its worker waits at the next block or item
    /// until [`Cancellation::resume`] or [`Cancellation::cancel`].
    pub fn pause(&self) {
        self.paused.store(true, Ordering::SeqCst);
    }

    /// Lets a paused operation go on.
    pub fn resume(&self) {
        self.paused.store(false, Ordering::SeqCst);
    }

    /// True while the operation is paused.
    pub fn is_paused(&self) -> bool {
        self.paused.load(Ordering::SeqCst)
    }

    /// Blocks the calling worker thread while the operation is paused and
    /// not cancelled. Never call it on the main thread.
    pub fn wait_while_paused(&self) {
        while self.is_paused() && !self.is_cancelled() {
            thread::sleep(PAUSE_POLL);
        }
    }

    /// The underlying cancellable for GIO calls.
    pub fn cancellable(&self) -> &gio::Cancellable {
        &self.cancellable
    }
}

/// Stops before the next step when the user cancelled. Without a
/// cancellation (cleanup and the uninterruptible steps of a replacement)
/// there is nothing to check.
///
/// # Errors
///
/// [`TransferError::Cancelled`] once `cancel` was cancelled.
pub(crate) fn check_cancelled(cancel: Option<&Cancellation>) -> Result<(), TransferError> {
    match cancel {
        Some(cancel) => cancel.check(),
        None => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// parity: OPS-022
    #[test]
    fn cancelling_stops_the_next_step_and_the_gio_cancellable() {
        let cancel = Cancellation::new();
        assert_eq!(cancel.check(), Ok(()));
        assert_eq!(check_cancelled(Some(&cancel)), Ok(()));

        cancel.cancel();

        assert!(cancel.is_cancelled());
        assert!(cancel.cancellable().is_cancelled());
        assert_eq!(cancel.check(), Err(TransferError::Cancelled));
        assert_eq!(check_cancelled(Some(&cancel)), Err(TransferError::Cancelled));
        assert_eq!(check_cancelled(None), Ok(()));
    }
}
