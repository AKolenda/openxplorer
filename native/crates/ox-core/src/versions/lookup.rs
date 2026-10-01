// SPDX-License-Identifier: AGPL-3.0-only
//! Finding the previous versions of one item (PROP-032).
//!
//! Ports `PreviousVersions.candidates` and `PreviousVersions.list` from
//! `v2.0.0:desktop/previous_versions.py`. The lookup reads the metadata of
//! snapshot folders the server already exposes. It is not an SMB
//! `FSCTL_SRV_ENUMERATE_SNAPSHOTS` client and never creates snapshots, so
//! finding nothing does not prove that there is no history.

use std::cmp::Reverse;
use std::sync::Arc;

use super::paths::{child_uri, relative_uri, SNAPPER_FILES_FOLDER};
use super::provider::{CollectionListing, GioSnapshotProvider, SnapshotProvider};
use super::{PreviousVersions, SnapshotDate, SnapshotLayout, SnapshotSource, VersionsError};
use crate::entry::{Entry, EntryError};
use crate::location::{normalise, split_location, ItemKind, LocationError, LocationKind};
use crate::transfer::Cancellation;

/// The most versions one lookup returns.
pub const MAX_VERSIONS: usize = 100;

/// The most entries read from one snapshot collection folder.
const MAX_SNAPSHOTS_PER_COLLECTION: usize = 100;

/// The most warnings one lookup returns.
const MAX_WARNINGS: usize = 8;

/// The name under which the app presents where versions come from.
pub const PROVIDER_NAME: &str = "Exposed snapshot folders";

/// The explanation shown when a lookup finds no version.
pub const NO_VERSIONS_FOUND: &str = "No matching previous versions were found in readable snapshot \
                                     folders. This does not prove that your server has no snapshots \
                                     or backups.";

/// The collection folders looked for at the root of an SMB share. `#` is
/// escaped because it is part of a URI.
const SHARE_COLLECTIONS: [(&str, SnapshotLayout); 4] = [
    (".snapshot", SnapshotLayout::Direct),
    ("%23snapshot", SnapshotLayout::Direct),
    (".zfs/snapshot", SnapshotLayout::Direct),
    (".snapshots", SnapshotLayout::Snapper),
];

/// The collection folders looked for next to other items. `#snapshot` is
/// a Synology SMB convention, so it is only looked for on shares.
const NEARBY_COLLECTIONS: [(&str, SnapshotLayout); 3] = [
    (".snapshot", SnapshotLayout::Direct),
    (".zfs/snapshot", SnapshotLayout::Direct),
    (".snapshots", SnapshotLayout::Snapper),
];

/// One previous version of an item: the item as it is inside one
/// snapshot. A previous version is always read-only (PROP-024).
#[derive(Debug, Clone, PartialEq)]
pub struct PreviousVersion {
    /// The item inside the snapshot, queried without following links. Its
    /// `uri` is where the version is: Browse opens it, and Restore a copy
    /// copies from it.
    pub entry: Entry,
    /// The snapshot's folder name, which is the version's name. Dates are
    /// read from it, never from `snapshot_modified` (PROP-020).
    pub label: String,
    /// The snapshot's own folder, where a tab browsing the version starts
    /// its "Previous version" badge.
    pub snapshot_root: String,
    /// The snapshot collection folder the version was found in (Python's
    /// `source`).
    pub collection: String,
    /// When the snapshot folder was last modified, in seconds since the
    /// Unix epoch. This is not when the snapshot was taken.
    pub snapshot_modified: Option<u64>,
}

impl PreviousVersion {
    /// The date in the snapshot's name, if it has one (PROP-020). Never
    /// [`snapshot_modified`](Self::snapshot_modified): that is when the
    /// snapshot folder last changed, which can describe the live folder
    /// rather than when the snapshot was taken.
    pub fn date(&self) -> Option<SnapshotDate> {
        SnapshotDate::from_snapshot_name(&self.label)
    }
}

/// The outcome of one lookup.
#[derive(Debug, Clone, PartialEq)]
pub struct VersionList {
    /// The versions found, by snapshot name in descending order, so names
    /// that start with a date list the newest first.
    pub versions: Vec<PreviousVersion>,
    /// The snapshot collection folders that could be read (Python's
    /// `sources`).
    pub collections: Vec<String>,
    /// Why some collections or snapshots could not be checked, at most
    /// eight. An item missing from a snapshot is not a warning.
    pub warnings: Vec<String>,
    /// True when a collection held more than 100 entries or the lookup
    /// stopped at [`MAX_VERSIONS`].
    pub is_truncated: bool,
    /// The configured sources whose live folder holds the item.
    pub configured: Vec<SnapshotSource>,
}

impl VersionList {
    /// [`NO_VERSIONS_FOUND`] when no version was found.
    pub fn message(&self) -> Option<&'static str> {
        self.versions.is_empty().then_some(NO_VERSIONS_FOUND)
    }
}

impl PreviousVersions {
    /// Finds the previous versions of the item at `uri`, a file or folder
    /// as `kind` says, by reading snapshot collections through `provider`.
    /// Every collection that can be read becomes a known snapshot root.
    /// This blocks; see
    /// [`find_versions_in_background`](Self::find_versions_in_background).
    ///
    /// Where it looks: the configured source whose live folder holds the
    /// item most specifically; without one, the collection folders at the
    /// root of an SMB share, or else next to the item (in its folder for a
    /// file). A filesystem root cannot be inferred from a path, so local
    /// items are only looked up nearby unless a source is configured.
    ///
    /// # Errors
    ///
    /// [`VersionsError::Location`] for an address the location rules
    /// refuse, and [`VersionsError::Cancelled`] once `cancel` is cancelled.
    /// Everything else becomes a warning in the list.
    pub fn find_versions(
        &self,
        provider: &dyn SnapshotProvider,
        uri: &str,
        kind: ItemKind,
        cancel: &Cancellation,
    ) -> Result<VersionList, VersionsError> {
        let uri = normalise(uri)?;
        let configured = self.sources();
        let mut search = VersionSearch::new(provider, &uri, cancel);
        for source in candidate_sources(&configured, &uri, kind) {
            check_cancelled(cancel)?;
            let Some(listing) = search.read_collection(&source)? else {
                continue;
            };
            self.remember_collection(source.collection());
            search.check_snapshots(&source, &listing.snapshots)?;
            if search.is_full() {
                break;
            }
        }
        let covering = configured
            .into_iter()
            .filter(|source| source.covers(&uri))
            .collect();
        Ok(search.finish(covering))
    }

    /// [`find_versions`](Self::find_versions) with GIO, on a GIO worker
    /// thread, so the window can await it without blocking. Cancelling
    /// `cancel` also aborts the GIO call in progress.
    ///
    /// # Errors
    ///
    /// As [`find_versions`](Self::find_versions).
    ///
    /// # Panics
    ///
    /// Re-raises a panic of the lookup on the worker thread, which is a bug.
    pub async fn find_versions_in_background(
        self: Arc<Self>,
        uri: String,
        kind: ItemKind,
        cancel: Cancellation,
    ) -> Result<VersionList, VersionsError> {
        let worker =
            gio::spawn_blocking(move || self.find_versions(&GioSnapshotProvider, &uri, kind, &cancel));
        match worker.await {
            Ok(outcome) => outcome,
            // A panicking lookup is a bug; it surfaces where it is awaited.
            Err(panic) => std::panic::resume_unwind(panic),
        }
    }
}

/// The sources to read for the item at the canonical `uri`, in order.
fn candidate_sources(configured: &[SnapshotSource], uri: &str, kind: ItemKind) -> Vec<SnapshotSource> {
    // `min_by_key` keeps the first of equally long live folders, as the
    // stable sort of the Python app does.
    let most_specific = configured
        .iter()
        .filter(|source| source.covers(uri))
        .min_by_key(|source| Reverse(source.live().len()));
    if let Some(source) = most_specific {
        return vec![source.clone()];
    }
    if let Some(share_root) = smb_share_root(uri) {
        return nearby_sources(&share_root, &SHARE_COLLECTIONS);
    }
    let folder = match kind {
        ItemKind::Folder => uri,
        ItemKind::File => uri.rsplit_once('/').map_or(uri, |(parent, _)| parent),
    };
    nearby_sources(folder, &NEARBY_COLLECTIONS)
}

/// `smb://server/share` for a location inside an SMB share; `None` for
/// other locations and for a whole server.
fn smb_share_root(uri: &str) -> Option<String> {
    let parts = split_location(uri).ok()?;
    if parts.kind() != LocationKind::Smb {
        return None;
    }
    let share = parts.path.trim_matches('/').split('/').next()?;
    if share.is_empty() {
        return None;
    }
    Some(format!("smb://{}/{share}", parts.authority))
}

/// One source per collection folder name, all for the live folder `live`.
fn nearby_sources(live: &str, collections: &[(&str, SnapshotLayout)]) -> Vec<SnapshotSource> {
    collections
        .iter()
        .map(|&(name, layout)| SnapshotSource::nearby(live, name, layout))
        .collect()
}

/// The folder of the snapshot named `name` in the collection of `source`.
fn snapshot_root(source: &SnapshotSource, name: &str) -> Result<String, LocationError> {
    let root = child_uri(source.collection(), name)?;
    Ok(match source.layout() {
        SnapshotLayout::Direct => root,
        SnapshotLayout::Snapper => format!("{root}/{SNAPPER_FILES_FOLDER}"),
    })
}

/// Stops the lookup once the user cancelled.
fn check_cancelled(cancel: &Cancellation) -> Result<(), VersionsError> {
    if cancel.is_cancelled() {
        Err(VersionsError::Cancelled)
    } else {
        Ok(())
    }
}

/// Why one snapshot could not be checked for the item.
#[derive(Debug, thiserror::Error)]
enum SnapshotProblem {
    /// The snapshot's name cannot be part of a location.
    #[error(transparent)]
    Name(#[from] LocationError),
    /// The item is not below the source's live folder.
    #[error("This item is outside the live folder.")]
    OutsideLiveFolder,
    /// The item could not be queried in the snapshot.
    #[error(transparent)]
    Read(#[from] EntryError),
}

/// The state of one lookup.
struct VersionSearch<'a> {
    provider: &'a dyn SnapshotProvider,
    /// The canonical URI of the item whose versions are wanted.
    uri: &'a str,
    cancel: &'a Cancellation,
    versions: Vec<PreviousVersion>,
    collections: Vec<String>,
    warnings: Vec<String>,
    is_truncated: bool,
}

impl<'a> VersionSearch<'a> {
    fn new(provider: &'a dyn SnapshotProvider, uri: &'a str, cancel: &'a Cancellation) -> Self {
        Self {
            provider,
            uri,
            cancel,
            versions: Vec::new(),
            collections: Vec::new(),
            warnings: Vec::new(),
            is_truncated: false,
        }
    }

    /// True once [`MAX_VERSIONS`] versions were found.
    fn is_full(&self) -> bool {
        self.versions.len() >= MAX_VERSIONS
    }

    /// Lists the collection of `source`, or records why it could not be
    /// read and returns `None`.
    fn read_collection(
        &mut self,
        source: &SnapshotSource,
    ) -> Result<Option<CollectionListing>, VersionsError> {
        let collection = source.collection();
        match self
            .provider
            .list_collection(collection, MAX_SNAPSHOTS_PER_COLLECTION, self.cancel)
        {
            Ok(listing) => {
                self.collections.push(collection.to_owned());
                self.is_truncated |= listing.has_more;
                Ok(Some(listing))
            }
            Err(error) => {
                check_cancelled(self.cancel)?;
                self.warnings.push(format!("{collection}: {error}"));
                Ok(None)
            }
        }
    }

    /// Looks for the item in each snapshot folder of the collection of
    /// `source`. Links and files in the collection are not snapshots.
    fn check_snapshots(&mut self, source: &SnapshotSource, snapshots: &[Entry]) -> Result<(), VersionsError> {
        for snapshot in snapshots {
            check_cancelled(self.cancel)?;
            // Safety rule PROP-032: only real folders in a collection are
            // snapshots; a link is never followed out of the collection.
            if !snapshot.is_dir || snapshot.is_symlink {
                continue;
            }
            match self.version_in(source, snapshot) {
                Ok(Some(version)) => self.versions.push(version),
                Ok(None) => {}
                Err(problem) => self.report_problem(snapshot, &problem)?,
            }
            if self.is_full() {
                self.is_truncated = true;
                break;
            }
        }
        Ok(())
    }

    /// The item in `snapshot`, or `None` when it is a link there.
    fn version_in(
        &self,
        source: &SnapshotSource,
        snapshot: &Entry,
    ) -> Result<Option<PreviousVersion>, SnapshotProblem> {
        let snapshot_root = snapshot_root(source, &snapshot.name)?;
        let relative = relative_uri(self.uri, source.live()).ok_or(SnapshotProblem::OutsideLiveFolder)?;
        let item_uri = if relative.is_empty() {
            snapshot_root.clone()
        } else {
            format!("{snapshot_root}/{relative}")
        };
        let mut entry = self.provider.inspect(&item_uri, self.cancel)?;
        // Safety rule (`list` in previous_versions.py): a link inside a
        // snapshot is never offered as a version, so Browse and Restore
        // never leave the snapshot through it.
        if entry.is_symlink {
            return Ok(None);
        }
        entry.uri = item_uri;
        Ok(Some(PreviousVersion {
            entry,
            label: snapshot.name.clone(),
            snapshot_root,
            collection: source.collection().to_owned(),
            snapshot_modified: snapshot.modified,
        }))
    }

    /// Records why `snapshot` could not be checked, unless the item is
    /// simply missing from it.
    fn report_problem(&mut self, snapshot: &Entry, problem: &SnapshotProblem) -> Result<(), VersionsError> {
        check_cancelled(self.cancel)?;
        // An item missing from an older snapshot is normal, while
        // permission and network failures must not look like "no history".
        if matches!(problem, SnapshotProblem::Read(EntryError::NotFound(_))) {
            return Ok(());
        }
        self.warnings.push(format!("{}: {problem}", snapshot.name));
        Ok(())
    }

    /// The list, with the newest-named versions first.
    fn finish(self, configured: Vec<SnapshotSource>) -> VersionList {
        let mut versions = self.versions;
        // A stable sort, so versions with the same name keep the order of
        // their collections, as in the Python app.
        versions.sort_by(|first, second| second.label.cmp(&first.label));
        let mut warnings = self.warnings;
        warnings.truncate(MAX_WARNINGS);
        VersionList {
            versions,
            collections: self.collections,
            warnings,
            is_truncated: self.is_truncated,
            configured,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The collection URIs of `sources`.
    fn collections(sources: &[SnapshotSource]) -> Vec<&str> {
        sources.iter().map(SnapshotSource::collection).collect()
    }

    /// parity: PROP-032
    #[test]
    fn an_smb_item_is_looked_up_at_its_share_root() {
        let sources = candidate_sources(&[], "smb://nas/share/work/report.pdf", ItemKind::File);

        assert_eq!(
            collections(&sources),
            [
                "smb://nas/share/.snapshot",
                "smb://nas/share/%23snapshot",
                "smb://nas/share/.zfs/snapshot",
                "smb://nas/share/.snapshots",
            ]
        );
        assert!(sources.iter().all(|source| source.live() == "smb://nas/share"));
        assert_eq!(sources[3].layout(), SnapshotLayout::Snapper);
    }

    /// parity: PROP-032
    #[test]
    fn a_local_file_is_looked_up_next_to_it_and_a_folder_inside_it() {
        let for_file = candidate_sources(&[], "file:///srv/data/report.pdf", ItemKind::File);
        let for_folder = candidate_sources(&[], "file:///srv/data", ItemKind::Folder);

        let expected = [
            "file:///srv/data/.snapshot",
            "file:///srv/data/.zfs/snapshot",
            "file:///srv/data/.snapshots",
        ];
        assert_eq!(collections(&for_file), expected);
        assert_eq!(collections(&for_folder), expected);
        assert!(for_file.iter().all(|source| source.live() == "file:///srv/data"));
    }

    /// parity: PROP-032
    #[test]
    fn the_most_specific_configured_source_is_the_only_candidate() {
        let configured = [
            SnapshotSource::new(
                "smb://nas/share",
                "smb://nas/backups/share",
                SnapshotLayout::Direct,
            )
            .unwrap(),
            SnapshotSource::new(
                "smb://nas/share/work",
                "smb://nas/backups/work",
                SnapshotLayout::Snapper,
            )
            .unwrap(),
            SnapshotSource::new(
                "smb://nas/other",
                "smb://nas/backups/other",
                SnapshotLayout::Direct,
            )
            .unwrap(),
        ];

        let sources = candidate_sources(&configured, "smb://nas/share/work/a.txt", ItemKind::File);

        assert_eq!(sources, [configured[1].clone()]);
    }

    #[test]
    fn a_whole_server_is_not_a_share() {
        assert_eq!(smb_share_root("smb://nas/"), None);
        assert_eq!(smb_share_root("file:///srv"), None);
        assert_eq!(
            smb_share_root("smb://nas:1445/share/a").as_deref(),
            Some("smb://nas:1445/share")
        );
    }

    #[test]
    fn a_snapper_snapshot_keeps_its_files_in_a_snapshot_folder() {
        let snapper = SnapshotSource::nearby("file:///home", ".snapshots", SnapshotLayout::Snapper);
        let direct = SnapshotSource::nearby("file:///home", ".snapshot", SnapshotLayout::Direct);

        assert_eq!(
            snapshot_root(&snapper, "42").as_deref(),
            Ok("file:///home/.snapshots/42/snapshot")
        );
        assert_eq!(
            snapshot_root(&direct, "daily 1").as_deref(),
            Ok("file:///home/.snapshot/daily%201")
        );
    }
}
