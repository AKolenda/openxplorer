// SPDX-License-Identifier: AGPL-3.0-only
//! Measured folder sizes as the window shows them (PROP-027).
//!
//! Ports `folderSizeText`, `updateSizeLabels`, `sizeKey` and the
//! `state.folderSizes` map of `v2.0.0:desktop/ui/app.js`. A folder is measured
//! only on request; its result lasts for the window's session, is never
//! updated after later changes, and is never shown as a false zero: a
//! partial or cancelled total reads `≥ 1.2 MB`.

use std::collections::HashMap;
use std::time::UNIX_EPOCH;

use ox_core::format;
use ox_core::sizes::{FolderSize, ScanStatus};

/// The Size text while a folder is being measured.
const SCANNING: &str = "Scanning…";
/// The Size text of a folder whose scan failed.
const UNAVAILABLE: &str = "Unavailable";
/// The Size text of a folder never measured, in the details pane and
/// Properties.
pub(crate) const NOT_SCANNED: &str = crate::i18n::message_id("Not scanned");
/// The status word of a failed scan in tooltips (`status:'error'`).
const ERROR_STATUS: &str = "error";
/// What a measured size counts, when the scan gave no other reason.
const LOGICAL_BYTES: &str = crate::i18n::message_id("Logical file bytes");

/// What is known about one folder's size.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum FolderSizeState {
    /// The totals of a scan: running, complete, partial or cancelled.
    Measured(FolderSize),
    /// The scan could not read the folder, for this reason.
    Unavailable(String),
}

impl FolderSizeState {
    /// The Size text: `Scanning…`, `Unavailable`, `1.2 MB`, or `≥ 1.2 MB`
    /// for a lower bound (`folderSizeText`).
    pub(crate) fn size_text(&self) -> String {
        let size = match self {
            FolderSizeState::Unavailable(_) => return UNAVAILABLE.to_owned(),
            FolderSizeState::Measured(size) => size,
        };
        let bytes = format::pretty_bytes(size.bytes);
        match size.status {
            ScanStatus::Scanning => SCANNING.to_owned(),
            ScanStatus::Complete => bytes,
            ScanStatus::Partial(_) | ScanStatus::Cancelled => format!("≥ {bytes}"),
        }
    }

    /// The Contains text of Properties: `12 files, 1 folder`, with `≥`
    /// for a lower bound, `Scanning…` or `Unavailable` (PROP-004).
    pub(crate) fn contains_text(&self) -> String {
        let size = match self {
            FolderSizeState::Unavailable(_) => return UNAVAILABLE.to_owned(),
            FolderSizeState::Measured(size) => size,
        };
        let counts = counts_text(size.files, size.folders);
        match size.status {
            ScanStatus::Scanning => SCANNING.to_owned(),
            ScanStatus::Complete => counts,
            ScanStatus::Partial(_) | ScanStatus::Cancelled => format!("≥ {counts}"),
        }
    }

    /// The tooltip of a Size value in the details pane and Properties:
    /// `partial · 3 files · 1 skipped · 0 unreadable. <reason> <time>`
    /// (`updateSizeLabels`).
    pub(crate) fn summary_tooltip(&self) -> String {
        match self {
            FolderSizeState::Unavailable(reason) => ox_core::i18n::format_message(
                "{ERROR_STATUS} · 0 files · 0 skipped · 0 unreadable. {reason} ",
                &[
                    ("ERROR_STATUS", &(ERROR_STATUS).to_string()),
                    ("reason", &(reason).to_string()),
                ],
            ),
            FolderSizeState::Measured(size) => {
                let status = size.status.as_str();
                let reason = size.status.reason();
                let finished = finished_text(size);
                ox_core::i18n::format_message(
                    "{status} · {files} files · {skipped} skipped · {errors} unreadable. {reason} {finished}",
                    &[
                        ("status", &(status).to_string()),
                        ("files", &(size.files).to_string()),
                        ("skipped", &(size.skipped).to_string()),
                        ("errors", &(size.errors).to_string()),
                        ("reason", &(reason).to_string()),
                        ("finished", &(finished).to_string()),
                    ],
                )
            }
        }
    }

    /// The tooltip of a Size cell in the details view:
    /// `complete · Logical file bytes · <time>` (`renderRows`).
    pub(crate) fn cell_tooltip(&self) -> String {
        match self {
            FolderSizeState::Unavailable(reason) => format!("{ERROR_STATUS} · {reason} · "),
            FolderSizeState::Measured(size) => {
                let reason = match size.status.reason() {
                    "" => ox_core::i18n::gettext_static(LOGICAL_BYTES),
                    reason => reason,
                };
                format!("{} · {reason} · {}", size.status.as_str(), finished_text(size))
            }
        }
    }

    /// The bytes the Size column sorts by: what was counted, 0 when the
    /// scan failed (`itemSize`).
    pub(crate) fn sort_bytes(&self) -> u64 {
        match self {
            FolderSizeState::Measured(size) => size.bytes,
            FolderSizeState::Unavailable(_) => 0,
        }
    }

    /// True for the totals of a scan that counted every item.
    pub(crate) fn is_complete(&self) -> bool {
        matches!(self, FolderSizeState::Measured(size) if size.status == ScanStatus::Complete)
    }
}

/// When the scan ended, in the local date and time, or empty while it
/// runs.
fn finished_text(size: &FolderSize) -> String {
    let Some(finished) = size.finished_at else {
        return String::new();
    };
    let seconds = finished.duration_since(UNIX_EPOCH).map(|since| since.as_secs());
    seconds.map_or_else(|_| String::new(), |seconds| format::date_time_text(Some(seconds)))
}

/// The key a folder's result is kept under: its URI without a trailing
/// slash, so `smb://nas/share/` and `smb://nas/share` are one folder
/// (`sizeKey`).
pub(crate) fn size_key(uri: &str) -> &str {
    uri.strip_suffix('/').unwrap_or(uri)
}

/// Every folder size a window measured this session.
#[derive(Debug, Default)]
pub(crate) struct FolderSizes {
    measured: HashMap<String, FolderSizeState>,
}

impl FolderSizes {
    /// What is known about the folder at `uri`.
    pub(crate) fn get(&self, uri: &str) -> Option<&FolderSizeState> {
        self.measured.get(size_key(uri))
    }

    /// Records `state` for the folder at `uri`.
    pub(crate) fn set(&mut self, uri: &str, state: FolderSizeState) {
        self.measured.insert(size_key(uri).to_owned(), state);
    }

    /// Forgets the folder at `uri`, as if it had never been measured.
    pub(crate) fn remove(&mut self, uri: &str) {
        self.measured.remove(size_key(uri));
    }
}

/// How many files and folders: `3 files, 1 folder`, as the Contains row
/// of every Properties dialog says it.
pub(super) fn counts_text(files: u64, folders: u64) -> String {
    let files = if files == 1 {
        "1 file".to_owned()
    } else {
        ox_core::i18n::format_message("{files} files", &[("files", &(files).to_string())])
    };
    let folders = if folders == 1 {
        "1 folder".to_owned()
    } else {
        ox_core::i18n::format_message("{folders} folders", &[("folders", &(folders).to_string())])
    };
    format!("{files}, {folders}")
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use ox_core::sizes::PartialReason;

    use super::*;

    /// A result of `bytes` bytes with `status`, not finished.
    fn measured(bytes: u64, status: ScanStatus) -> FolderSizeState {
        FolderSizeState::Measured(FolderSize {
            uri: "file:///tmp/ox-test/Projects".to_owned(),
            bytes,
            files: 3,
            folders: 1,
            entries: 4,
            skipped: 1,
            errors: 0,
            status,
            elapsed: Duration::ZERO,
            finished_at: None,
        })
    }

    /// A state and the Size text app.js shows for it.
    struct SizeTextCase {
        state: FolderSizeState,
        text: &'static str,
    }

    /// parity: PROP-027
    #[test]
    fn sizes_read_scanning_unavailable_exact_or_at_least() {
        let cases = [
            SizeTextCase {
                state: measured(0, ScanStatus::Scanning),
                text: "Scanning…",
            },
            SizeTextCase {
                state: FolderSizeState::Unavailable("Permission denied".to_owned()),
                text: "Unavailable",
            },
            SizeTextCase {
                state: measured(1280, ScanStatus::Complete),
                text: "1.3 KB",
            },
            SizeTextCase {
                state: measured(0, ScanStatus::Complete),
                text: "0 bytes",
            },
            SizeTextCase {
                state: measured(1280, ScanStatus::Partial(PartialReason::EntriesExcluded)),
                text: "≥ 1.3 KB",
            },
            SizeTextCase {
                state: measured(1280, ScanStatus::Cancelled),
                text: "≥ 1.3 KB",
            },
        ];
        for case in cases {
            assert_eq!(case.state.size_text(), case.text, "{:?}", case.state);
        }
    }

    /// parity: PROP-027
    #[test]
    fn tooltips_name_the_status_counts_and_reason() {
        let partial = measured(10, ScanStatus::Partial(PartialReason::EntriesExcluded));

        let summary = partial.summary_tooltip();
        let cell = measured(10, ScanStatus::Complete).cell_tooltip();

        assert_eq!(
            summary,
            "partial · 3 files · 1 skipped · 0 unreadable. Some links, mounts, snapshot collections or \
             unreadable entries were excluded "
        );
        assert_eq!(cell, "complete · Logical file bytes · ");
    }

    #[test]
    fn a_folder_is_one_result_with_or_without_its_trailing_slash() {
        let mut sizes = FolderSizes::default();

        sizes.set("smb://nas/share/", measured(5, ScanStatus::Complete));

        assert_eq!(
            sizes.get("smb://nas/share").map(FolderSizeState::sort_bytes),
            Some(5)
        );
    }
}
