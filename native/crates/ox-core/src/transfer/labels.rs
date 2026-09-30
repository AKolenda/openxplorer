// SPDX-License-Identifier: AGPL-3.0-only
//! Progress text for the transfer panel, word for word as
//! `desktop/operations.py` emits it.

use super::types::TransferMode;

/// `Copy: report.pdf (2/5)`, emitted when an item starts. Permanent delete
/// reads `Delete`, the other modes use their title-cased name.
pub(crate) fn item_label(mode: TransferMode, name: &str, position: usize, total: usize) -> String {
    let verb = match mode {
        TransferMode::Copy => "Copy",
        TransferMode::Move => "Move",
        TransferMode::Trash => "Trash",
        TransferMode::Delete => "Delete",
    };
    format!("{verb}: {name} ({position}/{total})")
}

/// `Copying report.pdf · 8,192 / 35,000 bytes`, emitted while bytes move.
pub(crate) fn copy_label(name: &str, current: u64, total: u64) -> String {
    format!(
        "Copying {name} · {} / {} bytes",
        group_thousands(current),
        group_thousands(total)
    )
}

/// Shown while the free-space check measures the items (XFER-028), as
/// Dolphin shows its examining phase. Not in the Python app, which had no
/// such check.
pub(crate) const CHECKING_SPACE_LABEL: &str = "Checking free space…";

/// `3 item(s) completed`, emitted once at the end of every run.
pub(crate) fn completed_label(done: usize) -> String {
    format!("{done} item(s) completed")
}

/// Formats like Python's `{value:,}`: `35000` becomes `35,000`.
fn group_thousands(value: u64) -> String {
    let digits = value.to_string();
    let mut grouped = String::with_capacity(digits.len() + digits.len() / 3);
    for (index, digit) in digits.chars().enumerate() {
        let remaining = digits.len() - index;
        if index > 0 && remaining.is_multiple_of(3) {
            grouped.push(',');
        }
        grouped.push(digit);
    }
    grouped
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thousands_are_grouped_like_python() {
        assert_eq!(group_thousands(0), "0");
        assert_eq!(group_thousands(999), "999");
        assert_eq!(group_thousands(1000), "1,000");
        assert_eq!(group_thousands(35_000), "35,000");
        assert_eq!(group_thousands(1_234_567), "1,234,567");
    }

    #[test]
    fn labels_match_the_python_strings() {
        assert_eq!(item_label(TransferMode::Copy, "a.txt", 1, 3), "Copy: a.txt (1/3)");
        assert_eq!(item_label(TransferMode::Move, "a", 2, 2), "Move: a (2/2)");
        assert_eq!(item_label(TransferMode::Trash, "a", 1, 1), "Trash: a (1/1)");
        assert_eq!(item_label(TransferMode::Delete, "a", 1, 1), "Delete: a (1/1)");
        assert_eq!(
            copy_label("big", 8192, 100_000),
            "Copying big · 8,192 / 100,000 bytes"
        );
        assert_eq!(completed_label(2), "2 item(s) completed");
    }
}
