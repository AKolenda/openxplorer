// SPDX-License-Identifier: AGPL-3.0-only
//! An item that fails for a reason other than a name conflict (OPS-047):
//! an unreadable source, a denied permission, a device that is gone, an
//! I/O error. The engine asks the app, as KIO's `SkipDialog` asks
//! Dolphin's user and Nautilus asks its own: "Retry" runs that item again,
//! "Skip" leaves it out, "Skip all" leaves out every later failing item
//! without asking, and "Cancel" stops the run, keeping what is done.
//! Skipped items are reported at the end like any failure (OPS-023), and
//! their staging is removed as for any failed item. Without a question
//! installed the error is recorded and the next item runs, as before.

use super::error::TransferError;
use super::types::TransferMode;

/// A failed item, as the question shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FailedItem {
    /// What the run does: copy, move, Trash or delete.
    pub mode: TransferMode,
    /// The item's name.
    pub name: String,
    /// Why it failed.
    pub error: String,
    /// Whether other items of the run remain, so "Skip all" means
    /// something.
    pub more_items: bool,
}

/// The user's answer about a [`FailedItem`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailureAnswer {
    /// Run the item again.
    Retry,
    /// Leave it out and go on.
    Skip,
    /// Leave it and every later failing item out, without asking.
    SkipAll,
    /// Stop the run; what is done stays done.
    Cancel,
}

/// Asks the user about a failed item; runs on the engine's worker thread.
pub type FailureQuestion = dyn FnMut(&FailedItem) -> FailureAnswer + Send;

/// The question about failed items for one engine, and whether "Skip all"
/// was answered: it covers the whole operation, every run of it included.
#[derive(Default)]
pub(crate) struct ItemFailures {
    question: Option<Box<FailureQuestion>>,
    skip_all: bool,
}

impl ItemFailures {
    /// Failures asked about with `question`.
    pub(crate) fn with_question(question: Box<FailureQuestion>) -> Self {
        Self {
            question: Some(question),
            skip_all: false,
        }
    }

    /// What to do about `item`, which failed with `error`: `None` records
    /// the error and goes on, as without a question. A name conflict, a
    /// cancellation and a problem that needs recovery by hand are never
    /// asked about.
    pub(crate) fn answer(&mut self, item: &FailedItem, error: &TransferError) -> Option<FailureAnswer> {
        let askable = !matches!(
            error,
            TransferError::Cancelled | TransferError::Exists(_) | TransferError::RecoveryRequired(_)
        );
        if !askable || self.skip_all {
            return None;
        }
        let answer = (self.question.as_mut()?)(item);
        self.skip_all = answer == FailureAnswer::SkipAll;
        Some(answer)
    }
}
