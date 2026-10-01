// SPDX-License-Identifier: AGPL-3.0-only
//! Unix modes of staged and published folders on backends that do not
//! implement `chmod`. Ports the remote-mode cases of
//! `v2.0.0:desktop/tests/test_operations.py`.

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
    /// The modes of the staging folders around every received file.
    fn staged_folder_modes(&self) -> Vec<u32> {
        self.staged_folder_modes.lock().expect("mode log").clone()
    }
}

impl Provider for FuseMountedDevice {
    fn uri(&self, node: &LocalNode) -> String {
        file_uri(node.local_path()).replacen("file://", "mtp://test-device", 1)
    }

    fn create_directory(&self, node: &LocalNode, cancel: Option<&Cancellation>) -> Result<(), TransferError> {
        node.local_create_directory(cancel)?;
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

/// A private source copied to a [`FuseMountedDevice`], and where its copy
/// ends up.
struct FuseCase {
    kind: NodeKind,
    /// The copied file that holds the content, below the destination folder.
    copied_file: &'static str,
    /// The published folder whose mode must stay the device's, if any.
    published_folder: Option<&'static str>,
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
    let cases = [
        FuseCase {
            kind: NodeKind::File,
            copied_file: "private",
            published_folder: None,
        },
        FuseCase {
            kind: NodeKind::Directory,
            copied_file: "private/inner",
            published_folder: Some("private"),
        },
    ];
    for case in cases {
        let fixture = Fixture::new();
        let source = fixture.source_folder.join("private");
        create_source(&source, case.kind, "android package fixture");
        set_mode(&source, 0o700);
        let device = Arc::new(FuseMountedDevice::default());

        let result = fixture.copy(device.clone(), &[&source], ConflictPolicy::Skip);

        assert!(result.errors.is_empty(), "{:?}: {result:?}", case.kind);
        assert_eq!(result.done, [file_uri(&source)]);
        let staged_modes = device.staged_folder_modes();
        assert!(!staged_modes.is_empty(), "{:?}", case.kind);
        assert!(
            staged_modes.iter().all(|mode| *mode == DEVICE_FOLDER_MODE),
            "{staged_modes:?}"
        );
        let copied = read(&fixture.destination_folder.join(case.copied_file));
        assert_eq!(copied, "android package fixture");
        if let Some(folder) = case.published_folder {
            let published_mode = mode_of(&fixture.destination_folder.join(folder));
            assert_eq!(published_mode, DEVICE_FOLDER_MODE);
        }
        fixture.assert_no_staging();
    }
}
