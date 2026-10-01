// SPDX-License-Identifier: AGPL-3.0-only
//! The previous-versions service: the snapshot sources, the snapshot roots
//! they and earlier lookups make known, and the read-only rule for them.
//!
//! Ports the `PreviousVersions` class of `v2.0.0:desktop/previous_versions.py`
//! except its lookup (`candidates`, `list`), which is in `lookup`.

use std::collections::BTreeSet;
use std::path::Path;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use super::paths::{is_conventional_snapshot, is_within};
use super::source_file::{SourceFile, MAX_SOURCES};
use super::{SnapshotLayout, SnapshotSource, VersionsError};
use crate::location::normalise;
use crate::transfer::TransferError;

/// Previous versions from snapshot and backup folders the server exposes,
/// and the rule that keeps them read-only (PROP-024).
///
/// One service is shared by every window of the app: the snapshot
/// collections a lookup found are remembered for the whole session.
#[derive(Debug)]
pub struct PreviousVersions {
    /// The configured sources, saved in the settings directory.
    source_file: SourceFile,
    /// Snapshot collections a lookup could read. They are read-only for
    /// the rest of the session, like configured ones.
    discovered_collections: Mutex<BTreeSet<String>>,
}

impl PreviousVersions {
    /// The service for the settings directory `settings_directory`
    /// (`~/.config/winspace`), where `snapshot-sources.json` is kept.
    pub fn new(settings_directory: &Path) -> Self {
        Self {
            source_file: SourceFile::new(settings_directory),
            discovered_collections: Mutex::new(BTreeSet::new()),
        }
    }

    /// The configured sources, read from the settings directory each time
    /// so that a change made by another window applies at once.
    pub fn sources(&self) -> Vec<SnapshotSource> {
        self.source_file.read()
    }

    /// Maps the live folder `live` to the snapshot collection folder
    /// `collection` and saves the sources. A mapping already saved for the
    /// same live folder is replaced. Returns the saved sources.
    ///
    /// # Errors
    ///
    /// Everything [`SnapshotSource::new`] refuses,
    /// [`VersionsError::TooManySources`] when [`MAX_SOURCES`] other live
    /// folders are mapped already, and [`VersionsError::Refused`] or
    /// [`VersionsError::Io`] when the sources cannot be saved.
    pub fn configure(
        &self,
        live: &str,
        collection: &str,
        layout: SnapshotLayout,
    ) -> Result<Vec<SnapshotSource>, VersionsError> {
        let source = SnapshotSource::new(live, collection, layout)?;
        self.source_file.update(|sources| {
            sources.retain(|saved| saved.live() != source.live());
            if sources.len() >= MAX_SOURCES {
                return Err(VersionsError::TooManySources);
            }
            sources.push(source);
            Ok(())
        })
    }

    /// Removes the mapping of the live folder `live`, if there is one, and
    /// saves the sources. Returns the saved sources.
    ///
    /// Python's `configure(..., remove=True)` also validated the snapshot
    /// folder field of the form; removing needs only the live folder.
    ///
    /// # Errors
    ///
    /// [`VersionsError::Location`] for an address the location rules
    /// refuse, and [`VersionsError::Refused`] or [`VersionsError::Io`] when
    /// the sources cannot be saved.
    pub fn remove_source(&self, live: &str) -> Result<Vec<SnapshotSource>, VersionsError> {
        let live = normalise(live)?;
        self.source_file.update(|sources| {
            sources.retain(|saved| saved.live() != live);
            Ok(())
        })
    }

    /// Every known snapshot root, sorted: the configured collections and
    /// those earlier lookups could read. The window's
    /// [`LocationContext::snapshot_roots`](crate::location::LocationContext::snapshot_roots).
    pub fn snapshot_roots(&self) -> Vec<String> {
        self.protected_locations().snapshot_roots.into_iter().collect()
    }

    /// The read-only rule as it stands now, for checking many locations
    /// with one read of the sources file, such as every row of a listing
    /// (Python's `annotate`).
    pub fn protected_locations(&self) -> ProtectedLocations {
        let mut snapshot_roots: BTreeSet<String> = self
            .sources()
            .iter()
            .map(|source| source.collection().to_owned())
            .collect();
        snapshot_roots.extend(self.discovered().iter().cloned());
        ProtectedLocations { snapshot_roots }
    }

    /// Refuses to change anything inside a snapshot or backup folder
    /// (Python's `assert_writable`). `uri` is normalised first.
    ///
    /// # Errors
    ///
    /// [`VersionsError::ReadOnly`] for a protected location, and
    /// [`VersionsError::Location`] for an address the location rules
    /// refuse.
    pub fn check_writable(&self, uri: &str) -> Result<(), VersionsError> {
        let uri = normalise(uri)?;
        // Safety rule PROP-024: snapshots and backups are never changed in
        // place; the user restores a copy elsewhere instead.
        if self.protected_locations().is_protected(&uri) {
            return Err(VersionsError::ReadOnly);
        }
        Ok(())
    }

    /// The canonical destination folder for "Restore a copy" of a previous
    /// version (PROP-025), which copies the version there with Keep both,
    /// so neither the live original nor the snapshot is replaced.
    /// `destination` is normalised first, as the address bar does.
    ///
    /// # Errors
    ///
    /// [`VersionsError::RestoreIntoSnapshot`] for a folder inside a
    /// snapshot or backup folder, and [`VersionsError::Location`] for an
    /// address the location rules refuse.
    pub fn restore_destination(&self, destination: &str) -> Result<String, VersionsError> {
        let destination = normalise(destination)?;
        // Safety rule PROP-025 (`restoreVersion` in v2.0.0:desktop/ui/app.js): a
        // restored copy goes to a live folder, never into a snapshot.
        if self.protected_locations().is_protected(&destination) {
            return Err(VersionsError::RestoreIntoSnapshot);
        }
        Ok(destination)
    }

    /// [`check_writable`](Self::check_writable) as the write guard of the
    /// transfer engine
    /// ([`TransferEngine::with_write_guard`](crate::transfer::TransferEngine::with_write_guard)),
    /// which asks it about every item of an affected tree (XFER-020). The
    /// sources are read for every question, so a mapping saved by another
    /// window applies at once.
    pub fn write_guard(
        self: &Arc<Self>,
    ) -> impl Fn(&str) -> Result<(), TransferError> + Send + Sync + 'static {
        let versions = Arc::clone(self);
        move |uri: &str| {
            versions
                .check_writable(uri)
                .map_err(|refusal| TransferError::failed(refusal.to_string()))
        }
    }

    /// Remembers a snapshot collection a lookup could read, which makes it
    /// read-only for the rest of the session.
    pub(crate) fn remember_collection(&self, collection: &str) {
        self.discovered().insert(collection.to_owned());
    }

    /// The discovered collections. The set is only ever extended by one
    /// `insert`, so a panic elsewhere cannot leave it half-changed and a
    /// poisoned lock is still safe to use.
    fn discovered(&self) -> MutexGuard<'_, BTreeSet<String>> {
        self.discovered_collections
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }
}

/// The read-only rule at one moment: conventional snapshot folders plus
/// the known snapshot roots. From [`PreviousVersions::protected_locations`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProtectedLocations {
    /// Configured and discovered snapshot collections.
    snapshot_roots: BTreeSet<String>,
}

impl ProtectedLocations {
    /// True when `uri` is inside a snapshot or backup folder and must be
    /// shown and treated as read-only. `uri` is compared as given; pass
    /// listing URIs from GIO or canonical URIs.
    pub fn is_protected(&self, uri: &str) -> bool {
        is_conventional_snapshot(uri) || self.snapshot_roots.iter().any(|root| is_within(uri, root))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A service over a fresh settings directory, which the caller keeps.
    fn service_in(directory: &tempfile::TempDir) -> PreviousVersions {
        PreviousVersions::new(directory.path())
    }

    /// parity: PROP-024
    #[test]
    fn a_discovered_collection_becomes_read_only() {
        let directory = tempfile::tempdir().unwrap();
        let versions = service_in(&directory);
        assert!(versions.check_writable("smb://nas/share/history/day").is_ok());

        versions.remember_collection("smb://nas/share/history");

        assert!(matches!(
            versions.check_writable("smb://nas/share/history/day"),
            Err(VersionsError::ReadOnly)
        ));
        assert_eq!(versions.snapshot_roots(), ["smb://nas/share/history"]);
    }

    #[test]
    fn the_write_guard_refuses_in_the_python_wording() {
        let directory = tempfile::tempdir().unwrap();
        let versions = Arc::new(service_in(&directory));
        let guard = versions.write_guard();

        let refusal = guard("smb://nas/share/.snapshot/old/file").unwrap_err();

        assert_eq!(
            refusal,
            TransferError::failed(
                "Previous-version locations are read-only in OpenXplorer. Restore a copy to a different folder first."
            )
        );
        assert_eq!(guard("smb://nas/share/live/file"), Ok(()));
    }
}
