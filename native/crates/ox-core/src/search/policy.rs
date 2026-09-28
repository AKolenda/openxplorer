// SPDX-License-Identifier: AGPL-3.0-only
//! What may be indexed below a root, and whether a root is on the network.
//!
//! Ports `root_network`, `policy` and `allowed` from
//! `desktop/index_service.py` (SRCH-030, SRCH-031).

use std::fs;
use std::path::Path;

use super::error::SearchError;
use super::mounts::{mount_for_path, read_mounts, Mount};
use super::scan::ListedItem;
use super::text::{is_at_or_below, local_path};
use crate::location::{file_uri, normalise, split_location, unquote_lossy};

/// System folders never indexed below a root.
const SYSTEM_FOLDERS: [&str; 6] = ["/proc", "/sys", "/dev", "/run", "/tmp", "/var/tmp"];

/// Folder names that hold snapshot history.
const SNAPSHOT_FOLDERS: [&str; 4] = [".snapshot", ".snapshots", "#snapshot", ".zfs"];

/// Filesystem types that are checked on a timer instead of watched.
const NETWORK_FILESYSTEMS: [&str; 7] = [
    "cifs",
    "smb3",
    "nfs",
    "nfs4",
    "sshfs",
    "fuse.sshfs",
    "fuse.gvfsd-fuse",
];

/// Where a root's files are stored, which decides how changes are found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RootStorage {
    /// On this computer: watched with inotify.
    Local,
    /// On SMB, NFS, sshfs or a `GVfs` FUSE mount: never push-watched, checked
    /// on a timer instead (SRCH-030).
    Network,
}

impl RootStorage {
    /// Where the files of `root` are stored now.
    pub(crate) fn current(root: &str) -> Self {
        Self::of(root, &read_mounts())
    }

    /// Where the files of `root` are stored, given the current `mounts`.
    ///
    /// Only a `file:` folder on a local filesystem is local. Python looked
    /// up the path of any other location, such as `sftp://host/home/demo`,
    /// in this computer's mount table, and so watched the unrelated local
    /// folder `/home/demo`; here every location that is not a local folder
    /// is checked on a timer, as a network location is.
    pub(crate) fn of(root: &str, mounts: &[Mount]) -> Self {
        let Some(path) = local_path(root) else {
            return Self::Network;
        };
        let is_network_mount = mount_for_path(&path, mounts)
            .is_some_and(|mount| NETWORK_FILESYSTEMS.contains(&mount.filesystem_type.as_str()));
        if is_network_mount {
            Self::Network
        } else {
            Self::Local
        }
    }
}

/// The part of the file system one root may index.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct IndexScope {
    root: String,
    excluded: Vec<String>,
}

impl IndexScope {
    /// The scope of `root` (`policy` in Python).
    ///
    /// Safety rule "index one filesystem, never system or index folders":
    /// below a local root, the system folders, every other mounted
    /// filesystem and the index's own directory are excluded. Choosing `/`
    /// means that filesystem only; a 2 TB volume mounted below `/media` is
    /// chosen separately. System folders are excluded only as descendants,
    /// so an explicitly chosen `/tmp/project` still indexes; otherwise its
    /// scan and every live update would silently find nothing.
    pub(crate) fn for_root(root: &str, index_directory: &Path, mounts: &[Mount]) -> Self {
        let mut scope = Self {
            root: root.to_owned(),
            excluded: Vec::new(),
        };
        let Some(root_path) = local_path(root) else {
            return scope;
        };
        let root_prefix = format!("{}/", root_path.to_string_lossy().trim_end_matches('/'));
        let system_folders = SYSTEM_FOLDERS
            .iter()
            .filter(|folder| folder.starts_with(&root_prefix))
            .map(|folder| file_uri(Path::new(folder)));
        scope.excluded.extend(system_folders);
        let index_directory =
            fs::canonicalize(index_directory).unwrap_or_else(|_| index_directory.to_path_buf());
        scope.excluded.push(file_uri(&index_directory));
        let nested_mounts = mounts
            .iter()
            .filter(|mount| mount.path != root_path && mount.path.starts_with(&root_path))
            .map(|mount| file_uri(&mount.path));
        scope.excluded.extend(nested_mounts);
        scope
    }

    /// Whether `uri` may be indexed (`allowed` in Python): inside the root,
    /// not excluded, and not in snapshot history.
    ///
    /// Safety rule "no snapshot history": `.snapshot`, `.snapshots`,
    /// `#snapshot` and `.zfs` folders would multiply the index by every
    /// retained version.
    pub(crate) fn admits(&self, uri: &str) -> bool {
        if !is_at_or_below(uri, &self.root) {
            return false;
        }
        if self.excluded.iter().any(|excluded| is_at_or_below(uri, excluded)) {
            return false;
        }
        !self.is_in_snapshot_folder(uri)
    }

    /// The items of `batch` that may be indexed, with canonical URIs.
    ///
    /// Safety rule "symlinks and virtual items are never followed or
    /// indexed" (`_run` in Python): they could lead outside the root.
    ///
    /// # Errors
    ///
    /// [`SearchError::Location`] when an item's URI does not normalise, for
    /// example because its name holds a control character; Python's
    /// `put_batch` refuses the batch the same way.
    pub(crate) fn admit(&self, batch: Vec<ListedItem>) -> Result<Vec<ListedItem>, SearchError> {
        let mut admitted = Vec::with_capacity(batch.len());
        for mut item in batch {
            if !item.is_indexable() {
                continue;
            }
            item.uri = normalise(&item.uri)?;
            if self.admits(&item.uri) {
                admitted.push(item);
            }
        }
        Ok(admitted)
    }

    /// Whether a folder between the root and `uri` is snapshot history.
    fn is_in_snapshot_folder(&self, uri: &str) -> bool {
        let root_path = decoded_path(&self.root);
        let path = decoded_path(uri);
        let relative = path
            .get(root_path.trim_end_matches('/').len()..)
            .unwrap_or_default();
        relative
            .split('/')
            .any(|component| SNAPSHOT_FOLDERS.contains(&component))
    }
}

/// The decoded path of a location.
fn decoded_path(uri: &str) -> String {
    split_location(uri)
        .map(|parts| unquote_lossy(&parts.path))
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    /// Python's `IndexPolicyTests` use a cache directory that need not exist.
    const INDEX_DIRECTORY: &str = "/home/test/.cache/winspace";

    fn scope(root: &str, mounts: &[Mount]) -> IndexScope {
        IndexScope::for_root(root, Path::new(INDEX_DIRECTORY), mounts)
    }

    /// Ported from `desktop/tests/test_release.py::IndexPolicyTests::test_whole_disk_still_excludes_tmp`
    /// parity: SRCH-031
    #[test]
    fn whole_disk_still_excludes_tmp() {
        assert!(!scope("file:///", &[]).admits("file:///tmp/private"));
    }

    /// Ported from `desktop/tests/test_release.py::IndexPolicyTests::test_explicit_tmp_subtree_is_indexable`
    /// parity: SRCH-031
    #[test]
    fn explicit_tmp_subtree_is_indexable() {
        let root = "file:///tmp/my-project";
        assert!(scope(root, &[]).admits("file:///tmp/my-project/notes.txt"));
    }

    /// Ported from `desktop/tests/test_release.py::IndexPolicyTests::test_selected_var_excludes_its_tmp_child`
    /// parity: SRCH-031
    #[test]
    fn selected_var_excludes_its_tmp_child() {
        assert!(!scope("file:///var", &[]).admits("file:///var/tmp/file"));
    }

    /// Ported from `desktop/tests/test_release.py::IndexPolicyTests::test_index_database_remains_excluded`
    /// parity: SRCH-031
    #[test]
    fn index_database_remains_excluded() {
        let home = scope("file:///home/test", &[]);
        assert!(!home.admits("file:///home/test/.cache/winspace/search.sqlite3"));
        assert!(home.admits("file:///home/test/.cache/other"));
    }

    /// Ported from `desktop/tests/test_release.py::IndexPolicyTests::test_nested_mounts_remain_excluded`
    /// parity: SRCH-031
    #[test]
    fn nested_mounts_remain_excluded() {
        let mounts = [Mount::new("/mnt/data/other-volume", "ext4")];
        assert!(!scope("file:///mnt/data", &mounts).admits("file:///mnt/data/other-volume/file"));
    }

    /// Ported from `desktop/tests/test_v05.py::LiveTests::test_root_exclusions`
    /// parity: SRCH-031
    #[test]
    fn root_exclusions() {
        assert!(!scope("file:///", &[]).admits("file:///proc/1"));
        let without_exclusions = IndexScope {
            root: "file:///".to_owned(),
            excluded: Vec::new(),
        };
        assert!(!without_exclusions.admits("file:///.snapshots/1"));
        assert!(!without_exclusions.admits("file:///.zfs/snapshot"));
    }

    /// The root's own mount is not "another filesystem", and a volume
    /// mounted below `/` is (SRCH-031).
    #[test]
    fn choosing_the_disk_leaves_out_other_mounted_volumes() {
        let mounts = [Mount::new("/", "ext4"), Mount::new("/media/demo/Backup", "ext4")];
        let disk = scope("file:///", &mounts);
        assert!(disk.admits("file:///home/demo/a.txt"));
        assert!(!disk.admits("file:///media/demo/Backup/a.txt"));
        assert!(scope("file:///media/demo/Backup", &mounts).admits("file:///media/demo/Backup/a.txt"));
    }

    #[test]
    fn items_outside_the_root_are_never_admitted() {
        let root = scope("file:///data", &[]);
        assert!(!root.admits("file:///data2/a"));
        assert!(!root.admits("smb://nas/share/a"));
    }

    /// parity: SRCH-030
    #[test]
    fn smb_and_network_mounts_are_network_roots() {
        let mounts = [
            Mount::new("/", "ext4"),
            Mount::new(PathBuf::from("/mnt/nas"), "nfs4"),
            Mount::new("/mnt/usb", "vfat"),
        ];
        assert_eq!(RootStorage::of("smb://nas/share", &mounts), RootStorage::Network);
        assert_eq!(
            RootStorage::of("file:///mnt/nas/projects", &mounts),
            RootStorage::Network
        );
        assert_eq!(RootStorage::of("file:///mnt/usb", &mounts), RootStorage::Local);
        assert_eq!(RootStorage::of("file:///home/demo", &mounts), RootStorage::Local);
    }

    /// A remote location that is not SMB is never watched as if it were
    /// the local folder with the same path.
    ///
    /// parity: SRCH-030
    #[test]
    fn other_remote_locations_are_network_roots() {
        let mounts = [Mount::new("/", "ext4")];
        assert_eq!(
            RootStorage::of("sftp://host/home/demo", &mounts),
            RootStorage::Network
        );
        assert_eq!(RootStorage::of("dav://host/files", &mounts), RootStorage::Network);
    }
}
