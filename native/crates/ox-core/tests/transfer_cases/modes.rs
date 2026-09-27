// SPDX-License-Identifier: AGPL-3.0-only
//! Unix modes of staged and published folders on backends that do not
//! implement `chmod`. Ports the remote-mode cases of
//! `desktop/tests/test_operations.py`.

use std::fs;
use std::sync::{Arc, Mutex};

use ox_core::transfer::{Cancellation, ConflictPolicy, Node, NodeKind, TransferError};

use crate::transfer_support::{
    local::{local_path_of, LocalNode, Provider},
    *,
};

/// The mode a [`FuseMountedDevice`] gives every folder it creates. Any
/// `chmod` by the engine would change it.
const DEVICE_FOLDER_MODE: u32 = 0o751;

/// A `GVfs` FUSE view of a phone: `mtp://` URIs that also have a local path,
/// on a backend without `chmod`, like `MtpBackedLocalNode` in the Python
/// tests. It records the modes of the staging folders around every file it
/// receives.
#[derive(Default)]
struct FuseMountedDevice {
    staged_folder_modes: Mutex<Vec<u32>>,
}

impl FuseMountedDevice {
    fn staged_folder_modes(&self) -> Vec<u32> {
        self.staged_folder_modes.lock().expect("mode log").clone()
    }
}

impl Provider for FuseMountedDevice {
    fn uri(&self, node: &LocalNode) -> String {
        uri(node.local_path()).replacen("file://", "mtp://test-device", 1)
    }

    fn mkdir(&self, node: &LocalNode, cancel: Option<&Cancellation>) -> Result<(), TransferError> {
        node.local_mkdir(cancel)?;
        set_mode(node.local_path(), DEVICE_FOLDER_MODE);
        Ok(())
    }

    fn copy_file(
        &self,
        node: &LocalNode,
        target: &dyn Node,
        cancel: &Cancellation,
        progress: &mut dyn FnMut(u64, u64),
    ) -> Result<(), TransferError> {
        let target_path = local_path_of(target);
        let staged_modes: Vec<u32> = target_path
            .ancestors()
            .skip(1)
            .filter(|folder| folder.ancestors().any(is_staging_path))
            .map(mode_of)
            .collect();
        self.staged_folder_modes
            .lock()
            .expect("mode log")
            .extend(staged_modes);
        node.local_copy_file(target, cancel, progress)
    }
}

/// Ports `test_mtp_backed_copy_with_fuse_path_does_not_require_chmod`,
/// `test_remote_directory_copy_never_applies_unix_modes` and
/// `test_remote_staging_does_not_attempt_unix_chmod`: neither the staging
/// folders nor the published folder of an `mtp:` item get Unix modes, even
/// though a local path exists.
///
/// parity: XFER-004, XFER-005
#[test]
fn device_copies_with_a_fuse_path_never_change_unix_modes() {
    for kind in [NodeKind::File, NodeKind::Directory] {
        let fixture = Fixture::new();
        let source = fixture.source_folder.join("private");
        if kind == NodeKind::Directory {
            fs::create_dir(&source).expect("create the source folder");
            set_mode(&source, 0o700);
            write(&source.join("data"), "data");
        } else {
            write(&source, "android package fixture");
        }
        let device = Arc::new(FuseMountedDevice::default());
        let result = fixture.copy(device.clone(), &[&source], ConflictPolicy::Skip);
        assert!(result.errors.is_empty(), "{result:?}");
        assert_eq!(result.done, [uri(&source)]);
        let staged_modes = device.staged_folder_modes();
        assert!(!staged_modes.is_empty());
        assert!(
            staged_modes.iter().all(|mode| *mode == DEVICE_FOLDER_MODE),
            "{staged_modes:?}"
        );
        if kind == NodeKind::Directory {
            assert_eq!(
                mode_of(&fixture.destination_folder.join("private")),
                DEVICE_FOLDER_MODE
            );
            assert_eq!(read(&fixture.destination_folder.join("private/data")), "data");
        } else {
            assert_eq!(
                read(&fixture.destination_folder.join("private")),
                "android package fixture"
            );
        }
        fixture.assert_no_staging();
    }
}
