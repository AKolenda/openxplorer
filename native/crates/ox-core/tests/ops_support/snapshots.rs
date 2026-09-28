// SPDX-License-Identifier: AGPL-3.0-only
//! The app's previous-versions write protection for the `ops_*` tests: the
//! guard double of the transfer tests (`tests/transfer_support/versions.rs`,
//! a port of `PreviousVersions.assert_writable` in
//! `desktop/previous_versions.py`), installed as an operation's
//! [`WriteProtection`].

#[path = "../transfer_support/versions.rs"]
mod versions;

use ox_core::ops::WriteProtection;

use versions::PreviousVersions;
pub use versions::READ_ONLY;

/// Protects every location in a conventional snapshot folder
/// (`.snapshot`, `.snapshots`, `#snapshot`, `@GMT-…`, `.zfs/snapshot`).
pub fn snapshot_protection() -> WriteProtection {
    snapshot_protection_with_folders(&[])
}

/// Like [`snapshot_protection`], and also protects each configured snapshot
/// folder in `configured`, given as `(live folder, its snapshot folder)`
/// URI pairs, as the previous-versions settings configure them.
pub fn snapshot_protection_with_folders(configured: &[(&str, &str)]) -> WriteProtection {
    let versions = PreviousVersions::new();
    for (live, snapshots) in configured {
        versions.configure(live, snapshots);
    }
    WriteProtection::new(versions.guard())
}
