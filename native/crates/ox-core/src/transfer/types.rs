// SPDX-License-Identifier: AGPL-3.0-only
//! Transfer request and progress types: the operation of a run, the
//! protocol names of modes and conflict policies accepted by
//! `TransferEngine.run` in `desktop/operations.py`, its progress events and
//! its `Result`.

use std::str::FromStr;

use super::error::TransferError;

/// The kind of [`Operation`] a run performs, by its protocol name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransferMode {
    /// Copy into the destination folder; sources are never changed.
    Copy,
    /// Native move or rename into the destination folder, never a
    /// copy-then-delete.
    Move,
    /// Move to the Trash, never falling back to a permanent delete.
    Trash,
    /// Permanent delete, only after explicit confirmation.
    Delete,
}

impl TransferMode {
    /// Every mode, so a protocol name is read back through
    /// [`as_str`](Self::as_str) and each name is spelt once.
    pub const ALL: [TransferMode; 4] = [
        TransferMode::Copy,
        TransferMode::Move,
        TransferMode::Trash,
        TransferMode::Delete,
    ];

    /// The protocol name (`copy`, `move`, `trash`, `delete`).
    pub fn as_str(self) -> &'static str {
        match self {
            TransferMode::Copy => "copy",
            TransferMode::Move => "move",
            TransferMode::Trash => "trash",
            TransferMode::Delete => "delete",
        }
    }
}

impl FromStr for TransferMode {
    type Err = TransferError;

    /// Parses the protocol name; anything else is refused with the Python
    /// app's message.
    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::ALL
            .into_iter()
            .find(|mode| mode.as_str() == value)
            .ok_or_else(|| TransferError::failed("Unknown operation."))
    }
}

/// What to do when a destination name already exists.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConflictPolicy {
    /// Leave the existing item untouched and report the source as skipped.
    Skip,
    /// Replace existing files and merge same-name folders (Windows-style).
    Replace,
    /// Use the next free `name (copy N)` name.
    KeepBoth,
}

impl ConflictPolicy {
    /// Every policy, so a protocol name is read back through
    /// [`as_str`](Self::as_str) and each name is spelt once.
    pub const ALL: [ConflictPolicy; 3] = [
        ConflictPolicy::Skip,
        ConflictPolicy::Replace,
        ConflictPolicy::KeepBoth,
    ];

    /// The protocol name (`skip`, `replace`, `keep-both`).
    pub fn as_str(self) -> &'static str {
        match self {
            ConflictPolicy::Skip => "skip",
            ConflictPolicy::Replace => "replace",
            ConflictPolicy::KeepBoth => "keep-both",
        }
    }
}

impl FromStr for ConflictPolicy {
    type Err = TransferError;

    /// Parses the protocol name; anything else is refused with the Python
    /// app's message.
    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::ALL
            .into_iter()
            .find(|policy| policy.as_str() == value)
            .ok_or_else(|| TransferError::failed("Choose Skip duplicates, Keep both, or Replace existing."))
    }
}

/// What one run does, with exactly the settings that operation takes:
/// copies and moves go into a destination folder under a conflict policy;
/// Trash and permanent delete take neither. A request in the app's
/// protocol becomes an operation through [`Operation::from_request`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Operation<'a> {
    /// Copy into a folder; sources are never changed.
    Copy {
        /// The URI of the folder the items go into.
        destination_folder: &'a str,
        /// What happens when an item's name is taken there.
        policy: ConflictPolicy,
    },
    /// Native move or rename into a folder, never a copy-then-delete.
    Move {
        /// The URI of the folder the items go into.
        destination_folder: &'a str,
        /// What happens when an item's name is taken there.
        policy: ConflictPolicy,
    },
    /// Move to the Trash, never falling back to a permanent delete.
    Trash,
    /// Permanent delete, only after explicit confirmation.
    Delete,
}

impl Operation<'_> {
    /// The mode of this operation.
    pub fn mode(self) -> TransferMode {
        match self {
            Operation::Copy { .. } => TransferMode::Copy,
            Operation::Move { .. } => TransferMode::Move,
            Operation::Trash => TransferMode::Trash,
            Operation::Delete => TransferMode::Delete,
        }
    }
}

/// What a [`Progress::fraction`] measures (OPS-020).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ProgressScope {
    /// How far the whole batch is: the items started, or all done.
    #[default]
    Batch,
    /// How many bytes of the file being copied are written. A full file
    /// bar never means that the batch has finished.
    File,
}

/// Progress for the transfer panel.
#[derive(Debug, Clone, PartialEq)]
pub struct Progress {
    /// Text exactly as the Python app shows it, for example
    /// `Copy: a.txt (1/3)` or `Copying a.txt · 8,192 / 35,000 bytes`.
    pub label: String,
    /// Between 0 and 1.
    pub fraction: f64,
    /// Whether `fraction` is the batch's or the current file's.
    pub scope: ProgressScope,
}

/// `part / whole` as a [`Progress::fraction`], at most 1; 0 when `whole` is
/// 0 (a file whose size is unknown or empty).
#[expect(
    clippy::cast_precision_loss,
    reason = "a progress bar needs far less precision than f64 keeps"
)]
pub(crate) fn progress_fraction(part: u64, whole: u64) -> f64 {
    if whole == 0 {
        return 0.0;
    }
    (part as f64 / whole as f64).min(1.0)
}

/// The outcome of one run. Every item ends in exactly one of `done`,
/// `skipped` or `errors`, except when a cleanup problem adds a second
/// message, or the run stopped because the user cancelled.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TransferResult {
    /// Source URIs that were copied, moved, trashed or deleted.
    pub done: Vec<String>,
    /// Source URIs left alone (name taken with Skip, or a move onto itself).
    pub skipped: Vec<String>,
    /// User-facing messages, `name: reason`, plus any staging left behind.
    pub errors: Vec<String>,
    /// The user cancelled; later items were not started.
    pub cancelled: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Ported from `desktop/tests/test_operations.py::TransferTests::test_unknown_operation_rejected`.
    /// Only the parsing can be tested:
    /// an unknown name never becomes a [`TransferMode`] or
    /// [`ConflictPolicy`], so the engine cannot be asked to run one.
    ///
    /// parity: XFER-019
    #[test]
    fn protocol_names_round_trip_and_unknown_ones_are_refused() {
        for mode in [
            TransferMode::Copy,
            TransferMode::Move,
            TransferMode::Trash,
            TransferMode::Delete,
        ] {
            assert_eq!(mode.as_str().parse::<TransferMode>(), Ok(mode));
        }
        for policy in [
            ConflictPolicy::Skip,
            ConflictPolicy::Replace,
            ConflictPolicy::KeepBoth,
        ] {
            assert_eq!(policy.as_str().parse::<ConflictPolicy>(), Ok(policy));
        }
        assert_eq!(
            "erase".parse::<TransferMode>(),
            Err(TransferError::failed("Unknown operation."))
        );
        assert_eq!(
            "overwrite".parse::<ConflictPolicy>(),
            Err(TransferError::failed(
                "Choose Skip duplicates, Keep both, or Replace existing."
            ))
        );
    }

    #[test]
    fn progress_fractions_stay_between_zero_and_one() {
        assert!(progress_fraction(0, 0).abs() < f64::EPSILON);
        assert!((progress_fraction(1, 4) - 0.25).abs() < f64::EPSILON);
        assert!((progress_fraction(9, 4) - 1.0).abs() < f64::EPSILON);
    }
}
