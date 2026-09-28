// SPDX-License-Identifier: AGPL-3.0-only
//! Which snapshot a location is inside (PROP-021), for the "Previous
//! version" badge and banner of a tab.
//!
//! Ports `location` in `desktop/ui/snapshot-meta.js`. A location is inside
//! a snapshot when a path component marks one (`.snapshot/<name>`,
//! `.snapshots/<id>/snapshot`, `#snapshot/<name>`,
//! `.zfs/snapshot/<name>` or `@GMT-…`), percent-encoded or not, or when it
//! lies below a configured or discovered snapshot collection.

use percent_encoding::percent_decode_str;

use super::paths::relative_uri;
use crate::location::split_location;

/// The label of a snapshot collection folder itself, which holds
/// snapshots rather than being one.
pub const SNAPSHOT_COLLECTION: &str = "Snapshot collection";

/// Folder names whose subfolders are snapshots.
const COLLECTION_NAMES: [&str; 3] = [".snapshot", ".snapshots", "#snapshot"];

/// Snapper's collection, whose snapshots keep their files in a
/// `snapshot` subfolder: `.snapshots/<id>/snapshot`.
const SNAPPER_COLLECTION: &str = ".snapshots";

/// The subfolder of a Snapper snapshot, and the ZFS collection's name
/// after `.zfs`.
const SNAPSHOT_FOLDER: &str = "snapshot";

/// The start of a Windows "Previous Versions" folder name.
const SMB_VERSION_PREFIX: &str = "@GMT-";

/// The snapshot a location is inside.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SnapshotLocation {
    /// The snapshot's own folder, or the collection folder itself.
    pub root: String,
    /// The snapshot's name, or [`SNAPSHOT_COLLECTION`] for a collection.
    pub label: String,
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
        .map(decode_component)
        .collect::<Option<_>>()?;
    match find_marker(&decoded) {
        Some(Marker::Collection) => Some(SnapshotLocation {
            root: uri.to_owned(),
            label: SNAPSHOT_COLLECTION.to_owned(),
        }),
        Some(Marker::Snapshot { last }) => Some(SnapshotLocation {
            root: format!(
                "{}://{}{}",
                parts.scheme,
                parts.authority,
                encoded[..=last].join("/")
            ),
            label: snapshot_name(&decoded, last).to_owned(),
        }),
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
        if COLLECTION_NAMES.contains(&name.as_str()) {
            let snapshot = index + 1;
            if is_empty_at(snapshot) {
                return Some(Marker::Collection);
            }
            let is_snapper = name == SNAPPER_COLLECTION
                && components
                    .get(snapshot + 1)
                    .is_some_and(|next| next == SNAPSHOT_FOLDER);
            let last = if is_snapper { snapshot + 1 } else { snapshot };
            return Some(Marker::Snapshot { last });
        }
        if name == ".zfs"
            && components
                .get(index + 1)
                .is_some_and(|next| next == SNAPSHOT_FOLDER)
        {
            let snapshot = index + 2;
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
        components[last] == SNAPSHOT_FOLDER && last >= 2 && components[last - 2] == SNAPPER_COLLECTION;
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
        return Some(SnapshotLocation {
            root: collection.to_owned(),
            label: SNAPSHOT_COLLECTION.to_owned(),
        });
    }
    Some(SnapshotLocation {
        root: format!("{collection}/{snapshot}"),
        label: decode_component(snapshot)?,
    })
}

/// JavaScript's `decodeURIComponent`: `None` for a `%` that does not
/// start a two-digit hex escape, or escapes that are not UTF-8.
fn decode_component(component: &str) -> Option<String> {
    let has_only_complete_escapes = component.split('%').skip(1).all(starts_with_hex_pair);
    if !has_only_complete_escapes {
        return None;
    }
    let decoded = percent_decode_str(component).decode_utf8().ok()?;
    Some(decoded.into_owned())
}

/// True when `text` starts with two hex digits.
fn starts_with_hex_pair(text: &str) -> bool {
    let pair = text.as_bytes().get(..2);
    pair.is_some_and(|pair| pair.iter().all(u8::is_ascii_hexdigit))
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
        snapshot_location(uri, &[]).map(|location| location.label)
    }

    /// Ported from `desktop/tests/snapshot_meta.test.cjs` ("Historical
    /// path …").
    ///
    /// parity: PROP-021
    #[test]
    fn snapshots_are_labelled_by_their_names() {
        for case in LABEL_CASES {
            assert_eq!(label_of(case.uri).as_deref(), Some(case.label), "{}", case.uri);
        }
    }

    /// Ported from `desktop/tests/snapshot_meta.test.cjs` ("Custom
    /// configured roots recognized").
    ///
    /// parity: PROP-021
    #[test]
    fn a_folder_below_a_configured_root_is_a_snapshot() {
        let roots = ["smb://nas/backup".to_owned()];

        let location = snapshot_location("smb://nas/backup/nightly/files", &roots);

        assert_eq!(
            location,
            Some(SnapshotLocation {
                root: "smb://nas/backup/nightly".into(),
                label: "nightly".into(),
            })
        );
    }

    /// Ported from `desktop/tests/snapshot_meta.test.cjs` ("Ordinary share
    /// not misidentified").
    ///
    /// parity: PROP-021
    #[test]
    fn an_ordinary_share_is_not_a_snapshot() {
        assert_eq!(snapshot_location("smb://nas/share/folder.mp4", &[]), None);
    }

    /// Ported from `desktop/tests/snapshot_meta.test.cjs` ("Encoded markers
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

    /// Ported from `desktop/tests/snapshot_meta.test.cjs` ("Snapshot root
    /// excludes descendants").
    ///
    /// parity: PROP-021
    #[test]
    fn the_root_is_the_snapshot_folder_not_the_descendant() {
        let location = snapshot_location("smb://nas/share/.zfs/snapshot/nightly/child", &[]);

        assert_eq!(
            location.map(|location| location.root).as_deref(),
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

        let snapper_root = snapper.map(|location| location.root);
        assert_eq!(
            snapper_root.as_deref(),
            Some("file:///home/.snapshots/123/snapshot")
        );
        assert_eq!(
            collection.map(|location| location.label).as_deref(),
            Some(SNAPSHOT_COLLECTION)
        );
        assert_eq!(
            deeper,
            Some(SnapshotLocation {
                root: "smb://nas/backup/deeper/day%201".into(),
                label: "day 1".into(),
            })
        );
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
