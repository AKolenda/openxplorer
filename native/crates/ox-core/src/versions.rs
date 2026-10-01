// SPDX-License-Identifier: AGPL-3.0-only
//! Previous versions from snapshot and backup folders, and the rule that
//! keeps those folders read-only.
//!
//! Ports `v2.0.0:desktop/previous_versions.py`, its GIO provider
//! (`SnapshotProvider` in `v2.0.0:desktop/file_services.py`) and the snapshot
//! name rules of `v2.0.0:desktop/ui/snapshot-meta.js`. Snapshots are found
//! only where a server or filesystem already exposes them as readable
//! folders: `.snapshot`, `#snapshot` and `.zfs/snapshot` collections,
//! Snapper's `.snapshots/<id>/snapshot` on Btrfs, and backup folders the
//! user maps to a live folder. Nothing here creates snapshots or asks an
//! SMB server for its shadow copies.
//!
//! The rules this module keeps:
//!
//! - PROP-024: every location inside a snapshot or backup folder is
//!   read-only. [`PreviousVersions::check_writable`] refuses changes there,
//!   and [`PreviousVersions::write_guard`] makes the transfer engine check
//!   every item of an affected tree (XFER-020). The user restores a copy
//!   to a live folder instead, which
//!   [`PreviousVersions::restore_destination`] checks (PROP-025).
//! - PROP-023: a snapshot collection never contains its live folder, at
//!   most [`MAX_SOURCES`] sources are saved, and the sources file is
//!   replaced atomically as a private file.
//! - PROP-032: a lookup reads metadata only, without following links, and
//!   reports what it could not read instead of claiming there is no
//!   history.
//! - PROP-020: a snapshot's date is read only from its name, never from
//!   its folder's modification time.
//!
//! | Module | Responsibility |
//! |---|---|
//! | `paths` | Conventional snapshot folders and URI containment |
//! | `source` | [`SnapshotSource`] and [`SnapshotLayout`] |
//! | `source_file` | Reading and saving `snapshot-sources.json` |
//! | `service` | [`PreviousVersions`]: sources, snapshot roots and the read-only rule |
//! | `lookup` | Finding the versions of an item |
//! | `provider` | [`SnapshotProvider`] and its GIO implementation |
//! | `snapshot_date` | [`SnapshotDate`]: the date in a snapshot's name |
//! | `snapshot_location` | [`snapshot_location()`]: which snapshot a location is in |
//! | `error` | [`VersionsError`] |

mod error;
mod lookup;
mod paths;
mod provider;
mod service;
mod snapshot_date;
mod snapshot_location;
mod source;
mod source_file;

pub use error::VersionsError;
pub use lookup::{PreviousVersion, VersionList, MAX_VERSIONS, NO_VERSIONS_FOUND, PROVIDER_NAME};
pub use paths::is_conventional_snapshot;
pub use provider::{CollectionListing, GioSnapshotProvider, SnapshotProvider};
pub use service::{PreviousVersions, ProtectedLocations};
pub use snapshot_date::{
    SnapshotDate, SnapshotTime, DATE_FROM_NAME, DATE_UNAVAILABLE, NO_DATE_EXPLANATION, NO_DATE_IN_NAME,
};
pub use snapshot_location::{snapshot_location, SnapshotLocation, SNAPSHOT_COLLECTION};
pub use source::{SnapshotLayout, SnapshotSource};
pub use source_file::MAX_SOURCES;
