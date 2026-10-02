// SPDX-License-Identifier: AGPL-3.0-only
//! The result of a folder-size scan, as it is published while scanning
//! and when the scan ends.
//!
//! Ports the result dictionary of `scan_folder` in
//! `v2.0.0:desktop/folder_sizes.py`: its counters, `status` and `reason`.

use std::fmt;
use std::time::{Duration, SystemTime};

/// How far a scan got.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScanStatus {
    /// Still scanning; the totals so far.
    Scanning,
    /// Every item was counted.
    Complete,
    /// The totals are a lower bound, for the reason given.
    Partial(PartialReason),
    /// The user cancelled; the totals are what was counted until then.
    Cancelled,
}

impl ScanStatus {
    /// The name the Python app reports: `scanning`, `complete`, `partial`
    /// or `cancelled`.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Scanning => "scanning",
            Self::Complete => "complete",
            Self::Partial(_) => "partial",
            Self::Cancelled => "cancelled",
        }
    }

    /// Why the totals are incomplete, in the Python app's words; empty
    /// while scanning and for a complete scan.
    pub fn reason(self) -> &'static str {
        match self {
            Self::Scanning | Self::Complete => "",
            Self::Partial(reason) => reason.as_str(),
            Self::Cancelled => crate::i18n::gettext_static("Cancelled by user"),
        }
    }
}

/// Why a scan's totals are a lower bound.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PartialReason {
    /// The time limit ran out between two folders.
    TimeLimitReached,
    /// The entry limit, or the time limit inside a folder, was reached.
    ScanLimitReached,
    /// Links, special files, mounts, snapshot collections or unreadable
    /// items were left out.
    EntriesExcluded,
}

impl PartialReason {
    /// The reason in the Python app's words.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::TimeLimitReached => crate::i18n::gettext_static("Time limit reached"),
            Self::ScanLimitReached => crate::i18n::gettext_static("Scan limit reached"),
            Self::EntriesExcluded => crate::i18n::gettext_static(
                "Some links, mounts, snapshot collections or unreadable entries were excluded",
            ),
        }
    }
}

impl fmt::Display for PartialReason {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// The totals of a folder-size scan. The size is the sum of the logical
/// sizes of regular files, not the space they take on disk, and never a
/// fabricated total: see [`status`](Self::status).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FolderSize {
    /// The canonical URI of the scanned folder.
    pub uri: String,
    /// The logical bytes of the files counted.
    pub bytes: u64,
    /// Regular files counted; a hard-linked file counts once.
    pub files: u64,
    /// Subfolders counted.
    pub folders: u64,
    /// Items looked at, against the entry limit.
    pub entries: u64,
    /// Items left out: links, special files, mounts, other filesystems,
    /// snapshot collections and files of unknown size.
    pub skipped: u64,
    /// Items and subfolders that could not be read ("unreadable").
    pub errors: u64,
    /// How far the scan got, and why the totals are incomplete.
    pub status: ScanStatus,
    /// How long the scan has run.
    pub elapsed: Duration,
    /// When the scan ended; `None` while scanning. The result is not
    /// updated after later changes, so the window shows this time.
    pub finished_at: Option<SystemTime>,
}

impl FolderSize {
    /// The totals of a scan of `uri` that has not counted anything yet.
    pub(crate) fn new(uri: String) -> Self {
        Self {
            uri,
            bytes: 0,
            files: 0,
            folders: 0,
            entries: 0,
            skipped: 0,
            errors: 0,
            status: ScanStatus::Scanning,
            elapsed: Duration::ZERO,
            finished_at: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A status and the `status` and `reason` the Python app reports for
    /// it.
    struct StatusCase {
        status: ScanStatus,
        name: &'static str,
        reason: &'static str,
    }

    /// Every status, with every reason a partial result can have.
    const STATUS_CASES: [StatusCase; 6] = [
        StatusCase {
            status: ScanStatus::Scanning,
            name: "scanning",
            reason: "",
        },
        StatusCase {
            status: ScanStatus::Complete,
            name: "complete",
            reason: "",
        },
        StatusCase {
            status: ScanStatus::Cancelled,
            name: "cancelled",
            reason: "Cancelled by user",
        },
        StatusCase {
            status: ScanStatus::Partial(PartialReason::TimeLimitReached),
            name: "partial",
            reason: "Time limit reached",
        },
        StatusCase {
            status: ScanStatus::Partial(PartialReason::ScanLimitReached),
            name: "partial",
            reason: "Scan limit reached",
        },
        StatusCase {
            status: ScanStatus::Partial(PartialReason::EntriesExcluded),
            name: "partial",
            reason: "Some links, mounts, snapshot collections or unreadable entries were excluded",
        },
    ];

    /// parity: PROP-029
    #[test]
    fn statuses_and_reasons_use_the_python_wording() {
        for case in STATUS_CASES {
            let reported = (case.status.as_str(), case.status.reason());

            assert_eq!(reported, (case.name, case.reason), "{:?}", case.status);
        }
    }
}
