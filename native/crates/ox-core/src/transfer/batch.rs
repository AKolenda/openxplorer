// SPDX-License-Identifier: AGPL-3.0-only
//! The settings shared by every item of one run, and what they imply for
//! each item: whether it is copied, moved, trashed or deleted, its start
//! progress, and whether the write guard checks its source. Ports the batch
//! arguments of `TransferEngine._run_items` in `desktop/operations.py`.

use super::cancellation::Cancellation;
use super::conflicts::Placement;
use super::guard::SourceChange;
use super::types::{progress_fraction, TransferMode};

/// The settings shared by every item of one run.
pub(crate) struct Batch<'a> {
    /// What the run does with each item.
    pub(crate) action: ItemAction<'a>,
    /// The user's cancellation, checked before each step of every item.
    /// It is the run's only token: the item's steps receive it from here.
    pub(crate) cancel: &'a Cancellation,
    /// The number of distinct items.
    pub(crate) total: usize,
}

/// What a run does with each item: its [`Operation`] once the destination
/// folder of a copy or move has been resolved and checked.
///
/// [`Operation`]: super::Operation
pub(crate) enum ItemAction<'a> {
    /// Copy or move into the destination folder, under the name the
    /// conflict policy chooses.
    Transfer(Placement<'a>),
    /// Trash or permanent delete.
    Remove(Removal),
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
    /// The mode of the run, which names it on the progress panel.
    pub(crate) fn mode(&self) -> TransferMode {
        match &self.action {
            ItemAction::Transfer(placement) => placement.mode,
            ItemAction::Remove(Removal::Trash) => TransferMode::Trash,
            ItemAction::Remove(Removal::PermanentDelete) => TransferMode::Delete,
        }
    }

    /// The progress shown when item `index` starts. Trash and delete have
    /// no byte progress, so the batch position is the only honest fraction
    /// to show for them.
    pub(crate) fn start_fraction(&self, index: usize) -> f64 {
        match self.action {
            ItemAction::Remove(_) => progress_fraction(index as u64, self.total as u64),
            ItemAction::Transfer(_) => 0.0,
        }
    }

    /// XFER-020: move, Trash and delete change their sources, so the write
    /// guard checks them; a copy keeps them.
    pub(crate) fn source_change(&self) -> SourceChange {
        match self.mode() {
            TransferMode::Move | TransferMode::Trash | TransferMode::Delete => SourceChange::Changed,
            TransferMode::Copy => SourceChange::Kept,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gio_node::GioNode;
    use crate::transfer::ConflictPolicy;

    /// What a batch with one action implies for its items.
    struct ActionCase<'a> {
        action: ItemAction<'a>,
        mode: TransferMode,
        source_change: SourceChange,
        /// The progress when the second of four items starts.
        second_item_fraction: f64,
    }

    /// parity: XFER-014, XFER-020
    #[test]
    fn each_action_decides_its_mode_source_checks_and_start_progress() {
        let cancel = Cancellation::new();
        let folder = GioNode::new("file:///tmp/destination");
        let placement = |mode: TransferMode| Placement {
            mode,
            policy: ConflictPolicy::Skip,
            destination_folder: &folder,
        };
        let cases = [
            ActionCase {
                action: ItemAction::Transfer(placement(TransferMode::Copy)),
                mode: TransferMode::Copy,
                source_change: SourceChange::Kept,
                second_item_fraction: 0.0,
            },
            ActionCase {
                action: ItemAction::Transfer(placement(TransferMode::Move)),
                mode: TransferMode::Move,
                source_change: SourceChange::Changed,
                second_item_fraction: 0.0,
            },
            ActionCase {
                action: ItemAction::Remove(Removal::Trash),
                mode: TransferMode::Trash,
                source_change: SourceChange::Changed,
                second_item_fraction: 0.25,
            },
            ActionCase {
                action: ItemAction::Remove(Removal::PermanentDelete),
                mode: TransferMode::Delete,
                source_change: SourceChange::Changed,
                second_item_fraction: 0.25,
            },
        ];
        for case in cases {
            let batch = Batch {
                action: case.action,
                cancel: &cancel,
                total: 4,
            };

            let fraction = batch.start_fraction(1);

            assert_eq!(batch.mode(), case.mode);
            assert_eq!(batch.source_change(), case.source_change, "{:?}", case.mode);
            let difference = (fraction - case.second_item_fraction).abs();
            assert!(difference < f64::EPSILON, "{:?}: {fraction}", case.mode);
        }
    }
}
