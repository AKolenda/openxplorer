// SPDX-License-Identifier: AGPL-3.0-only
//! XFER-016: a folder is never copied or moved into itself or one of its
//! descendants, whether the destination names it directly, through a
//! symbolic link, or through another host name. Ports the self-descendant
//! cases of `TransferTests` in `desktop/tests/test_operations.py` and the
//! alias guard of `_copy` in `desktop/operations.py`.

use std::fs;
use std::os::unix::fs::symlink;
use std::path::PathBuf;
use std::sync::Arc;

use ox_core::transfer::{ConflictPolicy, TransferMode};

use crate::transfer_support::{
    local::{self, LocalNode, Provider},
    *,
};

/// Ports `test_reject_self_descendant` and
/// `test_reject_symlink_destination_inside_source`.
///
/// parity: XFER-016
#[test]
fn self_and_descendant_destinations_are_rejected_including_symlink_aliases() {
    for mode in [TransferMode::Copy, TransferMode::Move] {
        let fixture = Fixture::new();
        let folder = fixture.source_folder.join("tree");
        let nested = folder.join("nested");
        fs::create_dir_all(&nested).unwrap();
        write(&folder.join("original"), "untouched");
        let alias = fixture.root.join("alias");
        symlink(&nested, &alias).unwrap();
        for destination in [&folder, &nested, &alias] {
            let mut engine = fixture.engine(local::local());

            let result = fixture.run(
                &mut engine,
                &[&folder],
                mode,
                ConflictPolicy::Replace,
                Some(destination),
            );

            assert!(result.done.is_empty());
            assert!(result.errors[0].contains("inside itself"));
            assert_eq!(read(&folder.join("original")), "untouched");
            assert!(list(&nested).is_empty());
        }
    }
}

/// One share reached under two host names, as when a server answers to
/// both `nas` and `nas.local`. Items at or below `second_name_root` have
/// `smb://host-b` URIs, all others `smb://host-a` ones, and no item has a
/// local path, so nothing proves that one folder lies inside the other.
struct ShareUnderTwoNames {
    second_name_root: PathBuf,
}

impl Provider for ShareUnderTwoNames {
    fn uri(&self, node: &LocalNode) -> String {
        let host = if node.local_path().starts_with(&self.second_name_root) {
            "smb://host-b"
        } else {
            "smb://host-a"
        };
        file_uri(node.local_path()).replacen("file://", host, 1)
    }

    fn path(&self, _node: &LocalNode) -> Option<PathBuf> {
        None
    }
}

/// Port of the alias guard in `_copy`: when the destination lies inside the
/// source under another host name, the copy meets its own staging folder
/// and stops instead of copying itself.
///
/// parity: XFER-016
#[test]
fn a_copy_into_its_own_subfolder_under_another_host_name_stops_at_its_staging() {
    let fixture = Fixture::new();
    let tree = fixture.source_folder.join("tree");
    let nested = tree.join("nested");
    fs::create_dir_all(&nested).unwrap();
    write(&tree.join("original"), "untouched");
    let share = Arc::new(ShareUnderTwoNames {
        second_name_root: nested.clone(),
    });
    let mut engine = fixture.engine(share);

    let result = fixture.run(
        &mut engine,
        &[&tree],
        TransferMode::Copy,
        ConflictPolicy::Skip,
        Some(&nested),
    );

    assert!(result.done.is_empty(), "{result:?}");
    assert_eq!(result.errors.len(), 1, "{result:?}");
    assert!(result.errors[0].contains("through an alias"), "{result:?}");
    assert!(list(&nested).is_empty(), "{:?}", list(&nested));
    assert_eq!(read(&tree.join("original")), "untouched");
}
