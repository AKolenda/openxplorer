// SPDX-License-Identifier: AGPL-3.0-only
//! The settings shared by every item of one run, and what they imply for
//! each item: its start progress, how it is named, whether the write guard
//! checks its source, and how Trash and delete remove it. Ports the batch
//! arguments of `TransferEngine._run_items` in `desktop/operations.py`.

use super::cancellation::Cancellation;
use super::conflicts::Placement;
use super::guard::SourceChange;
use super::node::Node;
use super::types::{progress_fraction, ConflictPolicy, TransferMode};

/// The settings shared by every item of one run.
pub(crate) struct Batch<'a> {
    pub(crate) mode: TransferMode,
    pub(crate) policy: ConflictPolicy,
    /// The destination folder; `None` for Trash and delete.
    pub(crate) destination_folder: Option<&'a dyn Node>,
    pub(crate) cancel: &'a Cancellation,
    /// The number of distinct items.
    pub(crate) total: usize,
}

/// How Trash and delete remove an item.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Removal {
    /// Move to the Trash.
    Trash,
    /// Delete permanently, after the user confirmed it.
    PermanentDelete,
}

impl Batch<'_> {
    /// The progress shown when item `index` starts. Trash and delete have
    /// no byte progress, so the batch position is the only honest fraction
    /// to show for them.
    pub(crate) fn start_fraction(&self, index: usize) -> f64 {
        if self.mode.is_removal() {
            progress_fraction(index as u64, self.total as u64)
        } else {
            0.0
        }
    }

    /// How a copy or move into `destination_folder` names its items.
    pub(crate) fn placement<'a>(&'a self, destination_folder: &'a dyn Node) -> Placement<'a> {
        Placement {
            mode: self.mode,
            policy: self.policy,
            destination_folder,
            cancel: self.cancel,
        }
    }

    /// XFER-020: move, Trash and delete change their sources, so the write
    /// guard checks them; a copy keeps them.
    pub(crate) fn source_change(&self) -> SourceChange {
        match self.mode {
            TransferMode::Move | TransferMode::Trash | TransferMode::Delete => SourceChange::Changed,
            TransferMode::Copy => SourceChange::Kept,
        }
    }

    /// The removal this run makes; `None` for copies and moves.
    pub(crate) fn removal(&self) -> Option<Removal> {
        match self.mode {
            TransferMode::Trash => Some(Removal::Trash),
            TransferMode::Delete => Some(Removal::PermanentDelete),
            TransferMode::Copy | TransferMode::Move => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// What a batch of one mode implies for its items.
    struct ModeCase {
        mode: TransferMode,
        source_change: SourceChange,
        removal: Option<Removal>,
        /// The progress when the second of four items starts.
        second_item_fraction: f64,
    }

    /// parity: XFER-014, XFER-020
    #[test]
    fn each_mode_decides_source_checks_removal_and_start_progress() {
        let cases = [
            ModeCase {
                mode: TransferMode::Copy,
                source_change: SourceChange::Kept,
                removal: None,
                second_item_fraction: 0.0,
            },
            ModeCase {
                mode: TransferMode::Move,
                source_change: SourceChange::Changed,
                removal: None,
                second_item_fraction: 0.0,
            },
            ModeCase {
                mode: TransferMode::Trash,
                source_change: SourceChange::Changed,
                removal: Some(Removal::Trash),
                second_item_fraction: 0.25,
            },
            ModeCase {
                mode: TransferMode::Delete,
                source_change: SourceChange::Changed,
                removal: Some(Removal::PermanentDelete),
                second_item_fraction: 0.25,
            },
        ];
        let cancel = Cancellation::new();
        for case in cases {
            let batch = Batch {
                mode: case.mode,
                policy: ConflictPolicy::Skip,
                destination_folder: None,
                cancel: &cancel,
                total: 4,
            };

            assert_eq!(batch.source_change(), case.source_change, "{:?}", case.mode);
            assert_eq!(batch.removal(), case.removal, "{:?}", case.mode);
            let fraction = batch.start_fraction(1);
            let difference = (fraction - case.second_item_fraction).abs();
            assert!(difference < f64::EPSILON, "{:?}: {fraction}", case.mode);
        }
    }
}
