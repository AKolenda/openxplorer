// SPDX-License-Identifier: AGPL-3.0-only
//! Progress text for the transfer panel, word for word as
//! `v2.0.0:desktop/operations.py` emits it.

use super::types::TransferMode;

/// `Copy: report.pdf (2/5)`, emitted when an item starts. Permanent delete
/// reads `Delete`, the other modes use their title-cased name.
pub(crate) fn item_label(mode: TransferMode, name: &str, position: usize, total: usize) -> String {
    let message = match mode {
        TransferMode::Copy => crate::i18n::message_id("Copy: {name} ({position}/{total})"),
        TransferMode::Move => crate::i18n::message_id("Move: {name} ({position}/{total})"),
        TransferMode::Trash => crate::i18n::message_id("Recycle: {name} ({position}/{total})"),
        TransferMode::Delete => crate::i18n::message_id("Delete: {name} ({position}/{total})"),
    };
    crate::i18n::format_message(
        message,
        &[
            ("name", name),
            ("position", &position.to_string()),
            ("total", &total.to_string()),
        ],
    )
}

/// `Copying report.pdf · 8,192 / 35,000 bytes`, emitted while bytes move.
pub(crate) fn copy_label(name: &str, current: u64, total: u64) -> String {
    crate::i18n::format_message(
        "Copying {name} · {current} / {total} bytes",
        &[
            ("name", name),
            ("current", &crate::format::group_digits(current)),
            ("total", &crate::format::group_digits(total)),
        ],
    )
}

/// Shown while the free-space check measures the items (XFER-028), as
/// Dolphin shows its examining phase. Not in the Python app, which had no
/// such check.
pub(crate) const CHECKING_SPACE_LABEL: &str = crate::i18n::message_id("Checking free space…");

/// `3 item(s) completed`, emitted once at the end of every run.
pub(crate) fn completed_label(done: usize) -> String {
    crate::i18n::format_message("{done} item(s) completed", &[("done", &(done).to_string())])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn english_groups_thousands_with_commas_as_python_does() {
        let commas = crate::format::DigitGrouping::COMMAS;
        assert_eq!(commas.group(0), "0");
        assert_eq!(commas.group(999), "999");
        assert_eq!(commas.group(1000), "1,000");
        assert_eq!(commas.group(35_000), "35,000");
        assert_eq!(commas.group(1_234_567), "1,234,567");
    }

    #[test]
    fn labels_match_the_python_strings() {
        assert_eq!(item_label(TransferMode::Copy, "a.txt", 1, 3), "Copy: a.txt (1/3)");
        assert_eq!(item_label(TransferMode::Move, "a", 2, 2), "Move: a (2/2)");
        assert_eq!(item_label(TransferMode::Trash, "a", 1, 1), "Recycle: a (1/1)");
        assert_eq!(item_label(TransferMode::Delete, "a", 1, 1), "Delete: a (1/1)");
        // The numbers are grouped as the numeric locale groups them.
        let grouped = crate::format::group_digits;
        assert_eq!(
            copy_label("big", 8192, 100_000),
            format!("Copying big · {} / {} bytes", grouped(8192), grouped(100_000))
        );
        assert_eq!(completed_label(2), "2 item(s) completed");
    }
}
