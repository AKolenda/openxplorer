// SPDX-License-Identifier: AGPL-3.0-only
//! Cooperative cancellation of a running operation. Ports `Cancellation` in
//! `desktop/operations.py`.
//!
//! The token wraps the [`gio::Cancellable`] that in-flight GIO calls receive,
//! so cancelling it both stops the engine between steps and aborts a copy
//! between blocks (OPS-022).

use gio::prelude::*;

use super::error::TransferError;

/// Cooperative cancellation shared with in-flight GIO calls.
#[derive(Debug, Clone, Default)]
pub struct Cancellation {
    cancellable: gio::Cancellable,
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
