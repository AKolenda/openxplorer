// SPDX-License-Identifier: AGPL-3.0-only
//! An item that fails for a reason other than a name conflict (OPS-047):
//! an unreadable source, a denied permission, a full drive, a file in use,
//! a device that is gone, an I/O error. The engine asks the app, which
//! words the question as Windows 11 does by the failure's
//! [`FailureCause`]: "Try Again" runs that item again, "Skip" leaves it
//! out, "Skip" with "Do this for all current items" leaves out every later
//! failing item without asking, and "Cancel" stops the run, keeping what
//! is done. "Continue", which Windows offers when the destination folder
//! needs an administrator, runs the item again into that folder under
//! `admin://` once [`crate::admin_copy`] found it safe; with "Do this for
//! all current items", every later item of the run goes there too.
//! Skipped items are recorded like any failure, as answered
//! (OPS-023), and their staging is removed as for any failed item. Without
//! a question installed the error is recorded and the next item runs, as
//! before.
//!
//! Inside a folder, the question is about the one entry that failed, as
//! in Windows Explorer: "Retry" copies that entry again, "Skip" leaves only
//! it out, and the rest of the folder is still copied and published. The
//! entries left out are reported at the end, and a move keeps them, with
//! their folders, where they were.

use std::collections::HashSet;

use super::error::TransferError;
use super::types::TransferMode;

/// What kind of failure a [`FailedItem`] is, which picks Windows' question
/// about it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailureCause {
    /// The item, or the folder it goes into, may not be read or written:
    /// "File Access Denied", or "Destination Folder Access Denied" when the
    /// destination folder is the one that may not be written.
    AccessDenied,
    /// The destination's drive is full: "There is not enough space on …".
    NoSpace,
    /// Another program holds the item: "File In Use".
    InUse,
    /// The item is gone from its folder: "Could not find this item".
    NotFound,
    /// Anything else; the question shows the system's error.
    Other,
}

impl FailureCause {
    /// The cause of `error`.
    pub fn of(error: &TransferError) -> Self {
        match error {
            TransferError::PermissionDenied(_) => Self::AccessDenied,
            TransferError::NoSpace(_) => Self::NoSpace,
            TransferError::Busy(_) => Self::InUse,
            TransferError::NotFound(_) => Self::NotFound,
            _ => Self::Other,
        }
    }
}

/// A failed item, as the question shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FailedItem {
    /// What the run does: copy, move, Trash or delete.
    pub mode: TransferMode,
    /// The item's name: its path below the top-level item for an entry
    /// inside a folder.
    pub name: String,
    /// Why it failed, in the system's words.
    pub error: String,
    /// Whether other items of the run remain, so "Do this for all current
    /// items" means something.
    pub more_items: bool,
    /// What kind of failure it is.
    pub cause: FailureCause,
    /// The failed item's URI, which the question describes; empty when the
    /// question is about the whole run, as a full drive found before any
    /// item ran.
    pub uri: String,
    /// The folder a copy or move writes into, `None` for the Recycle Bin and
    /// deletes.
    pub destination_folder: Option<String>,
    /// For a full drive, how many bytes the run needs there, when known.
    pub needed: Option<u64>,
}

impl FailedItem {
    /// The item at `uri`, called `name`, that failed with `error` in a run
    /// in `mode` writing into `destination_folder`.
    pub fn new(
        mode: TransferMode,
        name: String,
        uri: &str,
        destination_folder: Option<&str>,
        error: &TransferError,
    ) -> Self {
        Self {
            mode,
            name,
            error: error.to_string(),
            more_items: true,
            cause: FailureCause::of(error),
            uri: uri.to_owned(),
            destination_folder: destination_folder.map(str::to_owned),
            needed: None,
        }
    }
}

/// The user's answer about a [`FailedItem`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailureAnswer {
    /// Run the item again: "Try Again".
    Retry,
    /// Run the item again with administrator rights, writing into the
    /// destination folder under `admin://`: "Continue" (OPS-047).
    Elevate,
    /// As [`Self::Elevate`], and write every later item of the run the
    /// same way: "Continue" with "Do this for all current items".
    ElevateAll,
    /// Leave it out and go on.
    Skip,
    /// Leave it and every later failing item out, without asking: "Skip"
    /// with "Do this for all current items".
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
    /// The top-level item running, for questions about the entries
    /// inside it.
    current: Option<RunningItem>,
    /// The entries of the running item left out, "Folder/inner/name: why",
    /// each with whether the user saw it in the question.
    left_out: Vec<(String, bool)>,
    /// The source URIs of those entries.
    left_out_sources: HashSet<String>,
    /// The answer about an entry inside the running item that asked for
    /// the whole item to run again as administrator.
    elevation: Option<FailureAnswer>,
}

/// The top-level item of a run that is running.
#[derive(Debug, Clone)]
struct RunningItem {
    /// What the run does.
    mode: TransferMode,
    /// The item's name.
    name: String,
    /// The folder the run writes into, for a copy or move.
    destination_folder: Option<String>,
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
    /// `mode` that writes into `destination_folder`: nothing of it is left
    /// out yet.
    pub(crate) fn start_item(
        &mut self,
        mode: TransferMode,
        name: String,
        destination_folder: Option<String>,
    ) {
        self.current = Some(RunningItem {
            mode,
            name,
            destination_folder,
        });
        self.left_out.clear();
        self.left_out_sources.clear();
        self.elevation = None;
    }

    /// Continue or Continue for all, when the user answered it about an
    /// entry inside the running item, which then failed as a whole.
    pub(crate) fn take_elevation(&mut self) -> Option<FailureAnswer> {
        self.elevation.take()
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
        let running = self.current.clone().unwrap_or(RunningItem {
            mode: TransferMode::Copy,
            name: String::new(),
            destination_folder: None,
        });
        let name = if running.name.is_empty() {
            entry.to_owned()
        } else {
            format!("{}/{entry}", running.name)
        };
        let mut answered = false;
        if !self.skip_all {
            // Other entries, or other items, may follow.
            let failed = FailedItem::new(
                running.mode,
                name.clone(),
                source_uri,
                running.destination_folder.as_deref(),
                error,
            );
            match self.answer(&failed) {
                Some(FailureAnswer::Retry) => return EntryDecision::Retry,
                Some(FailureAnswer::Cancel) => return EntryDecision::Cancel,
                // The destination is the whole item's: it runs again as
                // administrator, from the start.
                Some(answer @ (FailureAnswer::Elevate | FailureAnswer::ElevateAll)) => {
                    self.elevation = Some(answer);
                    return EntryDecision::Fail;
                }
                Some(FailureAnswer::Skip | FailureAnswer::SkipAll) => answered = true,
                None => {}
            }
        }
        self.left_out.push((format!("{name}: {error}"), answered));
        self.left_out_sources.insert(source_uri.to_owned());
        EntryDecision::Skip
    }

    /// The entries of the running item left out, for the report, each with
    /// whether the user saw it in the question.
    pub(crate) fn take_left_out(&mut self) -> Vec<(String, bool)> {
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
