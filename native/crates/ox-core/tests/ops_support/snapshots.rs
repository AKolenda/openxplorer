// SPDX-License-Identifier: AGPL-3.0-only
//! A write protection like the app's previous-versions guard
//! (`PreviousVersions.assert_writable` in `desktop/previous_versions.py`):
//! every location with a `.snapshot` folder in its path is read-only.

use ox_core::ops::WriteProtection;
use ox_core::transfer::TransferError;

/// The refusal the Python app shows for a protected location.
pub const READ_ONLY: &str =
    "Previous-version locations are read-only in OpenXplorer. Restore a copy to a different folder first.";

/// Protects every location inside a `.snapshot` folder.
pub fn snapshot_protection() -> WriteProtection {
    WriteProtection::new(|uri: &str| {
        if uri.split('/').any(|part| part == ".snapshot") {
            Err(TransferError::failed(READ_ONLY))
        } else {
            Ok(())
        }
    })
}
