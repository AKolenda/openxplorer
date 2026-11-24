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
//! entries left out are recorded as answered, and a move keeps them, with
//! their folders, where they were.
//!
//! Nothing is said once the run ends: as in Windows Explorer, which has no
//! summary of an operation, every failure is asked about while it
//! happens. A problem that needs recovery by hand, such as a kept backup,
//! is told in the same place ([`FailureCause::NeedsAttention`]).

use std::collections::HashSet;

use super::error::TransferError;
use super::gone_drive::DestinationDrive;
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
    /// A folder would go into itself or a folder inside it (XFER-016):
    /// refused, so the question offers only Skip and Cancel.
    InsideSource,
    /// Anything else; the question shows the system's error.
    Other,
    /// The destination folder's drive is gone and the user skipped the
    /// item it failed: Windows' "Location is not available", whose OK
    /// ends the run.
    LocationUnavailable,
    /// Something the user must sort out by hand, such as a backup kept
    /// after a replacement or staging left behind: the question says what
    /// and where, and only lets the run go on (OPS-023).
    NeedsAttention,
}

impl FailureCause {
    /// The cause of `error`.
    pub fn of(error: &TransferError) -> Self {
        match error {
            TransferError::PermissionDenied(_) => Self::AccessDenied,
            TransferError::NoSpace(_) => Self::NoSpace,
            TransferError::Busy(_) => Self::InUse,
            TransferError::NotFound(_) => Self::NotFound,
            TransferError::InsideSource(_) => Self::InsideSource,
            TransferError::RecoveryRequired(_) => Self::NeedsAttention,
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
    /// Whether the run can do the item again as administrator, so the
    /// question may offer Continue with the shield; only the transfer
    /// engine can. Operations outside it offer Try Again instead.
    pub administrator: bool,
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
            administrator: true,
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
    /// The drive the run writes to, which may go away while it runs.
    destination: DestinationDrive,
    /// The user skipped an item that failed because that drive went away.
    skipped_gone_drive: bool,
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

    /// After Skip on an item whose drive went away: whether the run ends
    /// now, the user told once that the destination folder `destination`
    /// is not available. True only while `more_items` remain and the
    /// drive is still gone; a run of `mode`.
    pub(crate) fn stops_for_gone_drive(
        &mut self,
        mode: TransferMode,
        destination: Option<&str>,
        more_items: bool,
    ) -> bool {
        if !std::mem::take(&mut self.skipped_gone_drive) || !more_items || !self.destination.is_gone() {
            return false;
        }
        let uri = destination.unwrap_or_default();
        let item = FailedItem {
            more_items: false,
            cause: FailureCause::LocationUnavailable,
            ..FailedItem::new(
                mode,
                String::new(),
                uri,
                destination,
                &super::gone_drive::device_not_ready(),
            )
        };
        self.tell(&item);
        true
    }

    /// Watches `destination`, the drive the run that starts writes to.
    pub(crate) fn watch_destination(&mut self, destination: DestinationDrive) {
        self.destination = destination;
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
                | TransferError::InsideSource(_)
                // The share or device is gone: every later entry would
                // fail too.
                | TransferError::NotMounted(_)
        );
        if !per_entry {
            return EntryDecision::Fail;
        }
        // The drive written to went away: every later entry would fail
        // too, and the item says so once (see `gone_drive`).
        if self.destination.is_gone() {
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
        // "Skip all" answered this entry too.
        let mut answered = self.skip_all;
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

    /// `error`, the failure of an item of the run, in the words of a
    /// drive that went away when the destination's has.
    pub(crate) fn explain(&self, error: TransferError) -> TransferError {
        self.destination.explain(error)
    }

    /// Whether an item that failed with `error` is asked about. A name
    /// conflict, a cancellation, a problem that needs recovery by hand and
    /// something the location cannot do at all (which has questions of its
    /// own, such as "Move by copying?") are not, nor anything after "Skip
    /// all".
    pub(crate) fn asks_about(&self, error: &TransferError) -> bool {
        askable(error) && !self.skip_all && self.question.is_some()
    }

    /// Whether an item that failed with `error` is left out by an earlier
    /// "Skip all", which answered it: it is not reported afterwards.
    pub(crate) fn skips_for_all(&self, error: &TransferError) -> bool {
        askable(error) && self.skip_all
    }

    /// The user's answer about `item`, which [`Self::asks_about`] allowed;
    /// `None` without a question.
    pub(crate) fn answer(&mut self, item: &FailedItem) -> Option<FailureAnswer> {
        let answer = (self.question.as_mut()?)(item);
        self.skip_all = answer == FailureAnswer::SkipAll;
        let skipped = matches!(answer, FailureAnswer::Skip | FailureAnswer::SkipAll);
        self.skipped_gone_drive = skipped && super::gone_drive::is_device_gone_message(&item.error);
        Some(answer)
    }

    /// Tells the user, in the question about failed items, about `item`,
    /// whose cause is [`FailureCause::NeedsAttention`] or a failure the
    /// run cannot ask about otherwise, and waits until they read it; also
    /// after "Skip all" and a cancellation, because the user must act on
    /// it. True when the question showed it.
    pub(crate) fn tell(&mut self, item: &FailedItem) -> bool {
        let Some(question) = self.question.as_mut() else {
            return false;
        };
        question(item);
        true
    }

    /// Tells `item`, staging the run could not remove from the drive it
    /// writes to, as [`Self::tell`] does; unless that drive went away.
    /// Windows 11 asks nothing between Skip on such an item and "Location
    /// is not available", and the drive is not there to look at; the
    /// question about the item said the drive is gone, so `item` counts
    /// as told (true). It stays among the run's errors and recovery notes.
    pub(crate) fn tell_leftover(&mut self, item: &FailedItem) -> bool {
        if self.destination.is_gone() {
            return true;
        }
        self.tell(item)
    }
}

/// Whether an item that failed with `error` may be asked about: not a name
/// conflict, a cancellation, a problem that needs recovery by hand or
/// something the location cannot do at all.
fn askable(error: &TransferError) -> bool {
    !matches!(
        error,
        TransferError::Cancelled
            | TransferError::Exists(_)
            | TransferError::RecoveryRequired(_)
            | TransferError::NotSupported(_)
            | TransferError::ReplaceUnsupported(_)
    )
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use super::*;

    /// A USB stick mounted at /media/demo/STICK.
    const WITH_STICK: &str = "\
22 1 8:2 / / rw,relatime shared:1 - ext4 /dev/sda2 rw
95 22 8:17 / /media/demo/STICK rw,nosuid,nodev shared:50 - vfat /dev/sdb1 rw
";

    /// The stick pulled out.
    const WITHOUT_STICK: &str = "22 1 8:2 / / rw,relatime shared:1 - ext4 /dev/sda2 rw\n";

    /// "No such file or directory".
    const ENOENT: i32 = 2;

    /// Failures asked about with Skip, and the names asked about.
    fn skipping() -> (ItemFailures, Arc<Mutex<Vec<String>>>) {
        let asked = Arc::new(Mutex::new(Vec::new()));
        let heard = Arc::clone(&asked);
        let failures = ItemFailures::with_question(Box::new(move |item: &FailedItem| {
            heard.lock().expect("the names").push(item.name.clone());
            FailureAnswer::Skip
        }));
        (failures, asked)
    }

    /// An entry of a folder copied onto a stick that was pulled out fails
    /// the whole item at once, as for a share that went away, instead of
    /// asking "Item Not Found" about every entry of the folder.
    ///
    /// parity: OPS-047
    #[test]
    fn an_entry_on_a_drive_that_went_away_fails_its_item_without_asking() {
        let stick = "file:///media/demo/STICK";
        let missing = TransferError::NotFound("No such file or directory".into());
        let (mut failures, asked) = skipping();
        failures.start_item(TransferMode::Copy, "Photos".into(), Some(stick.into()));
        failures.watch_destination(
            DestinationDrive::fixed(stick, WITH_STICK, None).now(WITHOUT_STICK, Some(ENOENT)),
        );

        let decision = failures.entry_failed("beach.jpg", "file:///home/demo/Photos/beach.jpg", &missing);

        assert_eq!(decision, EntryDecision::Fail);
        assert!(asked.lock().expect("the names").is_empty(), "nothing asked");

        // With the stick still there, the entry is asked about as before.
        let (mut failures, asked) = skipping();
        failures.start_item(TransferMode::Copy, "Photos".into(), Some(stick.into()));
        failures.watch_destination(DestinationDrive::fixed(stick, WITH_STICK, Some(ENOENT)));
        let decision = failures.entry_failed("beach.jpg", "file:///home/demo/Photos/beach.jpg", &missing);
        assert_eq!(decision, EntryDecision::Skip);
        assert_eq!(*asked.lock().expect("the names"), ["Photos/beach.jpg"]);
    }
}
