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
//!
//! Inside a folder, the question is about the one entry that failed, as
//! in Windows Explorer: "Retry" copies that entry again, "Skip" leaves only
//! it out, and the rest of the folder is still copied and published. The
//! entries left out are reported at the end, and a move keeps them, with
//! their folders, where they were.

use std::collections::HashSet;

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
    /// The run's mode and the name of the top-level item running, for
    /// questions about the entries inside it.
    current: Option<(TransferMode, String)>,
    /// The entries of the running item left out, "Folder/inner/name: why".
    left_out: Vec<String>,
    /// The source URIs of those entries.
    left_out_sources: HashSet<String>,
}

/// What the copy does about an entry inside a folder that failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum EntryDecision {
    /// Copy the entry again.
    Retry,
    /// Leave the entry out and copy the rest.
    Skip,
    /// The whole item fails with the error, as before: a cancellation, a
    /// name conflict, a location that is gone, something that needs
    /// recovery by hand.
    Fail,
    /// The user cancelled the run.
    Cancel,
}

impl ItemFailures {
    /// Failures asked about with `question`.
    pub(crate) fn with_question(question: Box<FailureQuestion>) -> Self {
        Self {
            question: Some(question),
            ..Self::default()
        }
    }

    /// Starts (or starts again) the top-level item `name` of a run in
    /// `mode`: nothing of it is left out yet.
    pub(crate) fn start_item(&mut self, mode: TransferMode, name: String) {
        self.current = Some((mode, name));
        self.left_out.clear();
        self.left_out_sources.clear();
    }

    /// What to do about `entry`, the path of an entry below the running
    /// item, whose source is at `source_uri`, that failed with `error`.
    /// The user is asked as about an item; without a question, or after
    /// "Skip all", it is left out.
    pub(crate) fn entry_failed(
        &mut self,
        entry: &str,
        source_uri: &str,
        error: &TransferError,
    ) -> EntryDecision {
        let per_entry = !matches!(
            error,
            TransferError::Cancelled
                | TransferError::Exists(_)
                | TransferError::RecoveryRequired(_)
                | TransferError::NotSupported(_)
                | TransferError::ReplaceUnsupported(_)
                // The share or device is gone: every later entry would
                // fail too.
                | TransferError::NotMounted(_)
        );
        if !per_entry {
            return EntryDecision::Fail;
        }
        let (mode, top) = self
            .current
            .clone()
            .unwrap_or((TransferMode::Copy, String::new()));
        let name = if top.is_empty() {
            entry.to_owned()
        } else {
            format!("{top}/{entry}")
        };
        if !self.skip_all {
            let failed = FailedItem {
                mode,
                name: name.clone(),
                error: error.to_string(),
                // Other entries, or other items, may follow.
                more_items: true,
            };
            match self.answer(&failed) {
                Some(FailureAnswer::Retry) => return EntryDecision::Retry,
                Some(FailureAnswer::Cancel) => return EntryDecision::Cancel,
                Some(FailureAnswer::Skip | FailureAnswer::SkipAll) | None => {}
            }
        }
        self.left_out.push(format!("{name}: {error}"));
        self.left_out_sources.insert(source_uri.to_owned());
        EntryDecision::Skip
    }

    /// The entries of the running item left out, for the report.
    pub(crate) fn take_left_out(&mut self) -> Vec<String> {
        std::mem::take(&mut self.left_out)
    }

    /// The source URIs of the entries left out, which a move keeps.
    pub(crate) fn left_out_sources(&self) -> &HashSet<String> {
        &self.left_out_sources
    }

    /// Whether an item that failed with `error` is asked about. A name
    /// conflict, a cancellation, a problem that needs recovery by hand and
    /// something the location cannot do at all (which has questions of its
    /// own, such as "Move by copying?") are not, nor anything after "Skip
    /// all".
    pub(crate) fn asks_about(&self, error: &TransferError) -> bool {
        let askable = !matches!(
            error,
            TransferError::Cancelled
                | TransferError::Exists(_)
                | TransferError::RecoveryRequired(_)
                | TransferError::NotSupported(_)
                | TransferError::ReplaceUnsupported(_)
        );
        askable && !self.skip_all && self.question.is_some()
    }

    /// The user's answer about `item`, which [`Self::asks_about`] allowed;
    /// `None` without a question.
    pub(crate) fn answer(&mut self, item: &FailedItem) -> Option<FailureAnswer> {
        let answer = (self.question.as_mut()?)(item);
        self.skip_all = answer == FailureAnswer::SkipAll;
        Some(answer)
    }
}
