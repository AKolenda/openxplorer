// SPDX-License-Identifier: AGPL-3.0-only
//! Which snapshot a location is inside (PROP-021), for the "Previous
//! version" badge and banner of a tab.
//!
//! Ports `location` in `v2.0.0:desktop/ui/snapshot-meta.js`. A location is inside
//! a snapshot when a path component marks one (`.snapshot/<name>`,
//! `.snapshots/<id>/snapshot`, `#snapshot/<name>`,
//! `.zfs/snapshot/<name>` or `@GMT-…`), percent-encoded or not, or when it
//! lies below a configured or discovered snapshot collection. The markers
//! are those of the read-only rule, defined once in `paths`.

use super::paths::{
    relative_uri, SMB_VERSION_PREFIX, SNAPPER_COLLECTION, SNAPPER_FILES_FOLDER, SNAPSHOT_FOLDER_NAMES,
    ZFS_SNAPSHOT_FOLDER,
};
use super::SnapshotDate;
use crate::location::{decode_uri_component, split_location};

/// The label of a snapshot collection folder itself, which holds
/// snapshots rather than being one.
pub const SNAPSHOT_COLLECTION: &str = "Snapshot collection";

/// Where among the snapshots a location is: at a snapshot collection
/// folder itself, or inside one snapshot.
///
/// The variant, not the label text, tells the two apart, so a snapshot
/// that happens to be named "Snapshot collection" is still a snapshot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SnapshotLocation {
    /// The location is a snapshot collection folder, which holds snapshots
    /// rather than being one.
    Collection {
        /// The collection folder.
        root: String,
    },
    /// The location is a snapshot's own folder or lies inside it.
    Snapshot {
        /// The snapshot's own folder.
        root: String,
        /// The snapshot's name, which carries its date (PROP-020).
        name: String,
    },
}

impl SnapshotLocation {
    /// The folder a tab's "Previous version" badge covers: the snapshot's
    /// own folder, or the collection folder.
    pub fn root(&self) -> &str {
        match self {
            Self::Collection { root } | Self::Snapshot { root, .. } => root,
        }
    }

    /// The text the banner shows: the snapshot's name, or
    /// [`SNAPSHOT_COLLECTION`] for a collection.
    pub fn label(&self) -> &str {
        match self {
            Self::Collection { .. } => SNAPSHOT_COLLECTION,
            Self::Snapshot { name, .. } => name,
        }
    }

    /// The date in the snapshot's name (PROP-020); `None` for a
    /// collection and for a name without a date.
    pub fn date(&self) -> Option<SnapshotDate> {
        match self {
            Self::Collection { .. } => None,
            Self::Snapshot { name, .. } => SnapshotDate::from_snapshot_name(name),
        }
    }
}

/// The snapshot `uri` is inside, if any. `snapshot_roots` are the known
/// snapshot collections; when several hold `uri`, the longest counts.
/// `None` also for an address that does not split into URI parts or has
/// a malformed escape.
pub fn snapshot_location(uri: &str, snapshot_roots: &[String]) -> Option<SnapshotLocation> {
    let parts = split_location(uri).ok()?;
    let encoded: Vec<&str> = parts.path.split('/').collect();
    let decoded: Vec<String> = encoded
        .iter()
        .copied()
        .map(decode_uri_component)
        .collect::<Option<_>>()?;
    match find_marker(&decoded) {
        Some(Marker::Collection) => Some(SnapshotLocation::Collection { root: uri.to_owned() }),
        Some(Marker::Snapshot { last }) => {
            let snapshot_path = encoded[..=last].join("/");
            Some(SnapshotLocation::Snapshot {
                root: format!("{}://{}{snapshot_path}", parts.scheme, parts.authority),
                name: snapshot_name(&decoded, last).to_owned(),
            })
        }
        None => configured_snapshot(uri, snapshot_roots),
    }
}

/// What the first snapshot marker in a path says.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Marker {
    /// The path is a collection folder itself.
    Collection,
    /// The path is inside a snapshot whose folder ends at component
    /// `last`.
    Snapshot { last: usize },
}

/// The first snapshot marker among the decoded path `components`.
fn find_marker(components: &[String]) -> Option<Marker> {
    let is_empty_at = |index: usize| components.get(index).is_none_or(String::is_empty);
    for (index, name) in components.iter().enumerate() {
        if SNAPSHOT_FOLDER_NAMES.contains(&name.as_str()) {
            let snapshot = index + 1;
            if is_empty_at(snapshot) {
                return Some(Marker::Collection);
            }
            let is_snapper = name == SNAPPER_COLLECTION
                && components
                    .get(snapshot + 1)
                    .is_some_and(|next| next == SNAPPER_FILES_FOLDER);
            let last = if is_snapper { snapshot + 1 } else { snapshot };
            return Some(Marker::Snapshot { last });
        }
        let is_zfs_collection = components
            .get(index..index + ZFS_SNAPSHOT_FOLDER.len())
            .is_some_and(|pair| pair == ZFS_SNAPSHOT_FOLDER);
        if is_zfs_collection {
            let snapshot = index + ZFS_SNAPSHOT_FOLDER.len();
            if is_empty_at(snapshot) {
                return Some(Marker::Collection);
            }
            return Some(Marker::Snapshot { last: snapshot });
        }
        if name.starts_with(SMB_VERSION_PREFIX) {
            return Some(Marker::Snapshot { last: index });
        }
    }
    None
}

/// The name of the snapshot whose folder ends at component `last`: the
/// id of a Snapper snapshot, else the folder's own name.
fn snapshot_name(components: &[String], last: usize) -> &str {
    let is_snapper_files_folder =
        components[last] == SNAPPER_FILES_FOLDER && last >= 2 && components[last - 2] == SNAPPER_COLLECTION;
    if is_snapper_files_folder {
        &components[last - 1]
    } else {
        &components[last]
    }
}

/// The snapshot below the longest of `snapshot_roots` that holds `uri`.
fn configured_snapshot(uri: &str, snapshot_roots: &[String]) -> Option<SnapshotLocation> {
    let mut roots: Vec<&String> = snapshot_roots.iter().collect();
    // A stable sort, so equally long roots keep their order, as in
    // snapshot-meta.js.
    roots.sort_by_key(|root| std::cmp::Reverse(root.len()));
    roots.into_iter().find_map(|root| snapshot_below(uri, root))
}

/// The snapshot folder below the collection `root` that holds `uri`, or
/// the collection itself.
fn snapshot_below(uri: &str, root: &str) -> Option<SnapshotLocation> {
    let below = relative_uri(uri, root)?;
    let collection = root.trim_end_matches('/');
    let snapshot = below.split('/').next().unwrap_or_default();
    if snapshot.is_empty() {
        return Some(SnapshotLocation::Collection {
            root: collection.to_owned(),
        });
    }
    Some(SnapshotLocation::Snapshot {
        root: format!("{collection}/{snapshot}"),
        name: decode_uri_component(snapshot)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A location inside a snapshot and the label shown for it.
    struct LabelCase {
        uri: &'static str,
        label: &'static str,
    }

    const LABEL_CASES: [LabelCase; 6] = [
        LabelCase {
            uri: "smb://nas/share/.zfs/snapshot/auto-2026-09-04_16-30/notes",
            label: "auto-2026-09-04_16-30",
        },
        LabelCase {
            uri: "smb://nas/share/.snapshot/2026-09-05_180000",
            label: "2026-09-05_180000",
        },
        LabelCase {
            uri: "file:///home/.snapshots/123/snapshot/docs",
            label: "123",
        },
        LabelCase {
            uri: "smb://nas/share/%23snapshot/manual/children",
            label: "manual",
        },
        LabelCase {
            uri: "smb://nas/share/@GMT-2026.09.04-16.30.00/a",
            label: "@GMT-2026.09.04-16.30.00",
        },
        LabelCase {
            uri: "file:///backups/.zfs/snapshot",
            label: SNAPSHOT_COLLECTION,
        },
    ];

    /// The label of `uri` without configured roots.
    fn label_of(uri: &str) -> Option<String> {
        let location = snapshot_location(uri, &[])?;
        Some(location.label().to_owned())
    }

    /// Ported from `v2.0.0:desktop/tests/snapshot_meta.test.cjs` ("Historical
    /// path …").
    ///
    /// parity: PROP-021
    #[test]
    fn snapshots_are_labelled_by_their_names() {
        for case in LABEL_CASES {
            assert_eq!(label_of(case.uri).as_deref(), Some(case.label), "{}", case.uri);
        }
    }

    /// Ported from `v2.0.0:desktop/tests/snapshot_meta.test.cjs` ("Custom
    /// configured roots recognized").
    ///
    /// parity: PROP-021
    #[test]
    fn a_folder_below_a_configured_root_is_a_snapshot() {
        let roots = ["smb://nas/backup".to_owned()];

        let location = snapshot_location("smb://nas/backup/nightly/files", &roots);

        assert_eq!(
            location,
            Some(SnapshotLocation::Snapshot {
                root: "smb://nas/backup/nightly".into(),
                name: "nightly".into(),
            })
        );
    }

    /// Ported from `v2.0.0:desktop/tests/snapshot_meta.test.cjs` ("Ordinary share
    /// not misidentified").
    ///
    /// parity: PROP-021
    #[test]
    fn an_ordinary_share_is_not_a_snapshot() {
        assert_eq!(snapshot_location("smb://nas/share/folder.mp4", &[]), None);
    }

    /// Ported from `v2.0.0:desktop/tests/snapshot_meta.test.cjs` ("Encoded markers
    /// recognized").
    ///
    /// parity: PROP-021
    #[test]
    fn encoded_markers_are_recognised() {
        assert_eq!(
            label_of("smb://nas/share/%2Esnapshot/manual").as_deref(),
            Some("manual")
        );
    }

    /// Ported from `v2.0.0:desktop/tests/snapshot_meta.test.cjs` ("Snapshot root
    /// excludes descendants").
    ///
    /// parity: PROP-021
    #[test]
    fn the_root_is_the_snapshot_folder_not_the_descendant() {
        let location = snapshot_location("smb://nas/share/.zfs/snapshot/nightly/child", &[]);

        assert_eq!(
            location.as_ref().map(SnapshotLocation::root),
            Some("smb://nas/share/.zfs/snapshot/nightly")
        );
    }

    /// A Snapper snapshot's root is its `snapshot` folder, and a
    /// configured collection itself is labelled as a collection.
    ///
    /// parity: PROP-021
    #[test]
    fn snapper_roots_and_configured_collections_are_located() {
        let roots = [
            "smb://nas/backup".to_owned(),
            "smb://nas/backup/deeper".to_owned(),
        ];

        let snapper = snapshot_location("file:///home/.snapshots/123/snapshot/docs", &[]);
        let collection = snapshot_location("smb://nas/backup/", &roots);
        let deeper = snapshot_location("smb://nas/backup/deeper/day%201/a", &roots);

        assert_eq!(
            snapper.as_ref().map(SnapshotLocation::root),
            Some("file:///home/.snapshots/123/snapshot")
        );
        assert_eq!(
            collection.as_ref().map(SnapshotLocation::label),
            Some(SNAPSHOT_COLLECTION)
        );
        assert_eq!(
            collection,
            Some(SnapshotLocation::Collection {
                root: "smb://nas/backup".into(),
            })
        );
        assert_eq!(
            deeper,
            Some(SnapshotLocation::Snapshot {
                root: "smb://nas/backup/deeper/day%201".into(),
                name: "day 1".into(),
            })
        );
    }

    /// A snapshot named like the collection label is still a snapshot,
    /// with its own folder as the root.
    ///
    /// parity: PROP-021
    #[test]
    fn a_snapshot_named_like_the_collection_label_is_a_snapshot() {
        let roots = ["smb://nas/backup".to_owned()];

        let location = snapshot_location("smb://nas/backup/Snapshot%20collection/x", &roots);

        assert_eq!(
            location,
            Some(SnapshotLocation::Snapshot {
                root: "smb://nas/backup/Snapshot%20collection".into(),
                name: SNAPSHOT_COLLECTION.into(),
            })
        );
    }

    /// A snapshot's date comes from its name; a collection has none.
    ///
    /// parity: PROP-020, PROP-021
    #[test]
    fn only_a_snapshot_has_a_date() {
        let dated = snapshot_location("smb://nas/share/.snapshot/daily-2026-09-05/a", &[]);
        let collection = snapshot_location("smb://nas/share/.snapshot/", &[]);

        let date = dated.and_then(|location| location.date());
        assert_eq!(date.map(|date| date.date_text()).as_deref(), Some("2026-09-05"));
        assert_eq!(collection.and_then(|location| location.date()), None);
    }

    /// `decodeURIComponent` throws on a malformed escape, and
    /// snapshot-meta.js then reports no snapshot.
    ///
    /// parity: PROP-021
    #[test]
    fn a_malformed_escape_is_not_a_snapshot() {
        assert_eq!(snapshot_location("smb://nas/share/.snapshot/50%/a", &[]), None);
    }
}
