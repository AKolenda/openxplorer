// SPDX-License-Identifier: AGPL-3.0-only
//! Which locations lie inside a snapshot, and how the URI of an item inside
//! a snapshot is built.
//!
//! Ports `conventional_snapshot`, `within`, `child_uri` and `relative_uri`
//! from `v2.0.0:desktop/previous_versions.py`. All of them compare canonical URIs
//! as text, as the Python app does, because configured snapshot roots are
//! stored in that form.

use percent_encoding::{utf8_percent_encode, AsciiSet, NON_ALPHANUMERIC};

use crate::location::{split_location, unquote_lossy, validate_name, LocationError};

/// Folder names that hold snapshots: `.snapshot` (most NAS systems),
/// `.snapshots` (Snapper on Btrfs) and `#snapshot` (Synology over SMB).
/// Python's `MARKERS`.
///
/// These names, [`SMB_VERSION_PREFIX`] and [`ZFS_SNAPSHOT_FOLDER`] are the
/// snapshot markers of PROP-021 and PROP-024, defined only here. The
/// read-only rule ([`is_conventional_snapshot`]) and the "Previous
/// version" badge ([`snapshot_location`](super::snapshot_location()))
/// both read them, so they always agree.
pub(crate) const SNAPSHOT_FOLDER_NAMES: [&str; 3] = [".snapshot", SNAPPER_COLLECTION, "#snapshot"];

/// Snapper's snapshot collection on Btrfs, whose snapshots keep their
/// files in a [`SNAPPER_FILES_FOLDER`]: `.snapshots/<id>/snapshot`.
pub(crate) const SNAPPER_COLLECTION: &str = ".snapshots";

/// The folder inside a Snapper snapshot that holds its files.
pub(crate) const SNAPPER_FILES_FOLDER: &str = "snapshot";

/// The start of the folder names Windows "Previous Versions" shows over
/// SMB, for example `@GMT-2026.09.05-18.00.00`.
pub(crate) const SMB_VERSION_PREFIX: &str = "@GMT-";

/// The two path components of the snapshot folder ZFS exposes in every
/// dataset: `.zfs/snapshot`.
pub(crate) const ZFS_SNAPSHOT_FOLDER: [&str; 2] = [".zfs", "snapshot"];

/// The characters Python's `quote(name, safe='')` leaves alone: letters,
/// digits and `_.-~`. Everything else in a name is percent-encoded, `/`
/// included.
const NAME_UNESCAPED: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'_')
    .remove(b'.')
    .remove(b'-')
    .remove(b'~');

/// True when a decoded path component of `uri` is a snapshot folder
/// (`.snapshot`, `.snapshots`, `#snapshot` or a Windows `@GMT-…` version)
/// or `uri` lies inside `.zfs/snapshot`. Percent-encoded markers such as
/// `%23snapshot` count too. Configured snapshot roots are not considered
/// here; see [`PreviousVersions`](super::PreviousVersions).
///
/// Safety rule PROP-024 (fail closed): an address that does not split into
/// URI parts counts as a snapshot, so a location the rule cannot read is
/// never treated as writable. The Python app raised an error instead.
pub fn is_conventional_snapshot(uri: &str) -> bool {
    let Ok(parts) = split_location(uri) else {
        return true;
    };
    let decoded_path = unquote_lossy(&parts.path);
    let components: Vec<&str> = decoded_path.split('/').collect();
    let has_snapshot_folder = components.iter().any(|name| is_snapshot_folder_name(name));
    let is_in_zfs_snapshots = components
        .windows(ZFS_SNAPSHOT_FOLDER.len())
        .any(|pair| pair == ZFS_SNAPSHOT_FOLDER);
    has_snapshot_folder || is_in_zfs_snapshots
}

/// `.snapshot`, `.snapshots`, `#snapshot` or a Windows `@GMT-…` version.
fn is_snapshot_folder_name(name: &str) -> bool {
    SNAPSHOT_FOLDER_NAMES.contains(&name) || name.starts_with(SMB_VERSION_PREFIX)
}

/// True when `uri` is `root` or lies below it. Trailing slashes are
/// ignored and path boundaries respected: `file:///backups-old` is not
/// within `file:///backups`.
pub(crate) fn is_within(uri: &str, root: &str) -> bool {
    relative_uri(uri, root).is_some()
}

/// The part of `uri` below `root`, without leading slashes: `a/b.txt` for
/// `smb://nas/share/a/b.txt` below `smb://nas/share`, and empty for `root`
/// itself. `None` when `uri` is not [within](is_within) `root`.
pub(crate) fn relative_uri<'a>(uri: &'a str, root: &str) -> Option<&'a str> {
    let root = root.trim_end_matches('/');
    let rest = uri.strip_prefix(root)?;
    let is_at_boundary = rest.is_empty() || rest.starts_with('/');
    is_at_boundary.then(|| rest.trim_start_matches('/'))
}

/// The URI of the item `name` inside the folder `folder_uri`, with the name
/// escaped as a single path component.
///
/// # Errors
///
/// The [`validate_name`] error for a name that is empty, `.` or `..`, or
/// holds a slash or a control character: such a name would reach outside
/// the folder.
pub(crate) fn child_uri(folder_uri: &str, name: &str) -> Result<String, LocationError> {
    // Safety rule (`child_uri` in previous_versions.py): a snapshot name
    // read from a server never leads outside its collection folder.
    validate_name(name)?;
    let escaped_name = utf8_percent_encode(name, NAME_UNESCAPED);
    let folder_uri = folder_uri.trim_end_matches('/');
    Ok(format!("{folder_uri}/{escaped_name}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One location and whether it is inside a conventional snapshot.
    struct SnapshotCase {
        uri: &'static str,
        is_snapshot: bool,
    }

    /// Locations inside and outside conventional snapshot folders.
    const SNAPSHOT_CASES: [SnapshotCase; 9] = [
        SnapshotCase {
            uri: "smb://nas/share/.zfs/snapshot/auto-2026-09-04_16-30/notes",
            is_snapshot: true,
        },
        SnapshotCase {
            uri: "smb://nas/share/.snapshot/2026-09-05_180000",
            is_snapshot: true,
        },
        SnapshotCase {
            uri: "file:///home/.snapshots/123/snapshot/docs",
            is_snapshot: true,
        },
        SnapshotCase {
            uri: "smb://nas/share/%23snapshot/manual/children",
            is_snapshot: true,
        },
        SnapshotCase {
            uri: "smb://nas/share/@GMT-2026.09.04-16.30.00/a",
            is_snapshot: true,
        },
        SnapshotCase {
            uri: "smb://nas/share/%2Esnapshot/manual",
            is_snapshot: true,
        },
        SnapshotCase {
            uri: "smb://nas/share/folder.mp4",
            is_snapshot: false,
        },
        SnapshotCase {
            uri: "file:///home/demo/snapshot/.zfs",
            is_snapshot: false,
        },
        SnapshotCase {
            uri: "file:///home/demo/my.snapshot",
            is_snapshot: false,
        },
    ];

    /// Ported from `v2.0.0:desktop/tests/snapshot_meta.test.cjs` (the historical
    /// paths, "Encoded markers recognized" and "Ordinary share not
    /// misidentified"), for the markers `conventional_snapshot` in
    /// `v2.0.0:desktop/previous_versions.py` shares with the web UI.
    ///
    /// parity: PROP-021
    #[test]
    fn snapshot_folders_are_recognised_by_their_path_components() {
        for case in SNAPSHOT_CASES {
            assert_eq!(
                is_conventional_snapshot(case.uri),
                case.is_snapshot,
                "{}",
                case.uri
            );
        }
    }

    /// parity: PROP-024
    #[test]
    fn an_address_that_does_not_split_counts_as_a_snapshot() {
        assert!(is_conventional_snapshot("smb://[nas/share"));
    }

    /// Ported from `v2.0.0:desktop/tests/snapshot_meta.test.cjs` ("Location
    /// boundary not prefix coincidence").
    ///
    /// parity: PROP-021
    #[test]
    fn within_respects_path_boundaries() {
        assert!(!is_within("file:///backups-old/x", "file:///backups"));
        assert!(is_within("file:///backups/x", "file:///backups"));
        assert!(is_within("file:///backups/", "file:///backups"));
        assert!(is_within("file:///backups", "file:///backups/"));
    }

    #[test]
    fn relative_uri_is_the_path_below_the_root() {
        assert_eq!(
            relative_uri("smb://nas/share/a/b.txt", "smb://nas/share"),
            Some("a/b.txt")
        );
        assert_eq!(relative_uri("smb://nas/share/", "smb://nas/share"), Some(""));
        assert_eq!(relative_uri("smb://nas/shared/a", "smb://nas/share"), None);
    }

    #[test]
    fn child_names_are_escaped_as_one_path_component() {
        let uri = child_uri("smb://nas/share/.snapshot/", "Weekly #1 (~x)");

        assert_eq!(
            uri.as_deref(),
            Ok("smb://nas/share/.snapshot/Weekly%20%231%20%28~x%29")
        );
    }

    #[test]
    fn a_name_that_would_leave_the_folder_is_refused() {
        for name in ["", ".", "..", "a/b", "line\nbreak"] {
            assert!(child_uri("file:///snapshots", name).is_err(), "{name:?}");
        }
    }
}
