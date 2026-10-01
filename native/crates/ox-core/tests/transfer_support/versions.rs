// SPDX-License-Identifier: AGPL-3.0-only
//! Test double of the write guard the app passes to the engine:
//! `PreviousVersions.assert_writable` in `v2.0.0:desktop/previous_versions.py`.
//!
//! A location is protected when a path component is a conventional snapshot
//! folder (`.snapshot`, `.snapshots`, `#snapshot`, `@GMT-…`, `.zfs/snapshot`)
//! or it lies within a configured snapshot folder. Only the guard is
//! mirrored; listing and restoring previous versions are not.

use std::sync::{Arc, Mutex};

use percent_encoding::percent_decode_str;

use ox_core::transfer::TransferError;

const MARKERS: [&str; 3] = [".snapshot", ".snapshots", "#snapshot"];

/// The message the Python app shows for a protected location.
pub const READ_ONLY: &str =
    "Previous-version locations are read-only in OpenXplorer. Restore a copy to a different folder first.";

/// Configured snapshot folders plus the conventional markers.
#[derive(Default)]
pub struct PreviousVersions {
    snapshot_roots: Mutex<Vec<String>>,
}

impl PreviousVersions {
    /// A guard with no configured snapshot folders.
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// Declares `snapshots` as the snapshot folder of `live`.
    pub fn configure(&self, live: &str, snapshots: &str) {
        assert_ne!(
            live, snapshots,
            "the snapshot folder must differ from the live folder"
        );
        self.snapshot_roots
            .lock()
            .expect("snapshot roots")
            .push(snapshots.to_string());
    }

    /// Refuses protected locations with [`READ_ONLY`]. Ported from
    /// `PreviousVersions.assert_writable` in `v2.0.0:desktop/previous_versions.py`,
    /// which raises the refusal where this returns it.
    ///
    /// # Errors
    ///
    /// [`READ_ONLY`] for a location inside a snapshot folder.
    pub fn check_writable(&self, uri: &str) -> Result<(), TransferError> {
        let configured = self.snapshot_roots.lock().expect("snapshot roots");
        let is_in_configured_snapshot = configured.iter().any(|root| is_within(uri, root));
        if is_conventional_snapshot(uri) || is_in_configured_snapshot {
            Err(TransferError::failed(READ_ONLY))
        } else {
            Ok(())
        }
    }

    /// The guard as the engine takes it.
    pub fn guard(self: &Arc<Self>) -> impl Fn(&str) -> Result<(), TransferError> + Send + Sync + 'static {
        let versions = Arc::clone(self);
        move |uri: &str| versions.check_writable(uri)
    }
}

/// Ported from `conventional_snapshot` in `v2.0.0:desktop/previous_versions.py`.
fn is_conventional_snapshot(uri: &str) -> bool {
    let after_scheme = uri.split_once("://").map_or(uri, |(_, rest)| rest);
    let path = after_scheme.find('/').map_or("", |slash| &after_scheme[slash..]);
    let decoded = percent_decode_str(path).decode_utf8_lossy();
    let parts: Vec<&str> = decoded.split('/').collect();
    let marked = parts
        .iter()
        .any(|part| MARKERS.contains(part) || part.starts_with("@GMT-"));
    let zfs = parts.windows(2).any(|pair| pair == [".zfs", "snapshot"]);
    marked || zfs
}

/// Ported from `within` in `v2.0.0:desktop/previous_versions.py`.
fn is_within(uri: &str, root: &str) -> bool {
    let root = root.trim_end_matches('/');
    uri.trim_end_matches('/') == root || uri.starts_with(&format!("{root}/"))
}
