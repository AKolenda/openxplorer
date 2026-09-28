// SPDX-License-Identifier: AGPL-3.0-only
//! Test double of a `GVfs` MTP destination: `DeviceNode` in
//! `desktop/tests/test_device_staging.py`.
//!
//! It follows the adapter contract `GioNode` provides for `mtp://`, as
//! measured on a Pixel 9 with `GVfs` 1.54.4: a same-folder move is a
//! non-overwriting rename (`set_display_name`); a cross-folder move keeps
//! the item's name; a cross-folder move under a different name is refused;
//! one-step overwrite is unsupported. Every move and relist is recorded.
//!
//! [`Device::without_move_object`] models devices without MTP `MoveObject`
//! (Android 7 and 8): `GVfs` refuses every cross-folder move there, while a
//! same-folder rename still works. [`Phone`] adds the stale path cache of
//! `GVfs` MTP and copies within one device.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use ox_core::transfer::{Cancellation, Node, TransferError};

use super::file_uri;
use super::local::{local_path_of, LocalNode, Provider};

/// One device call, for assertions (Python `CALLS`).
#[derive(Debug, Clone, PartialEq, Eq)]
enum Call {
    /// `move_native` of an item.
    Move(RecordedMove),
    /// `refresh_listing` of a folder.
    Refresh(PathBuf),
}

/// One `move_native` the device was asked for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordedMove {
    /// The item's path before the move.
    pub from: PathBuf,
    /// The path it was asked to move to.
    pub to: PathBuf,
}

/// Whether a device implements MTP `MoveObject`, which cross-folder moves
/// need.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum MoveObject {
    /// A cross-folder move keeps the item's name (a Pixel 9).
    #[default]
    Supported,
    /// Every cross-folder move fails with "not supported", as `do_move` in
    /// the MTP backend of `GVfs` reports without the capability.
    Unsupported,
}

/// The device behaviour and its call log.
#[derive(Default)]
pub struct Device {
    calls: Mutex<Vec<Call>>,
    move_object: MoveObject,
}

impl Device {
    /// A device without MTP `MoveObject`, such as a phone with Android 7 or 8.
    pub fn without_move_object() -> Self {
        Self {
            move_object: MoveObject::Unsupported,
            ..Self::default()
        }
    }

    /// Every recorded call, in order.
    fn calls(&self) -> Vec<Call> {
        self.calls.lock().expect("call log").clone()
    }

    /// The recorded moves, in order.
    pub fn moves(&self) -> Vec<RecordedMove> {
        let calls = self.calls();
        calls
            .into_iter()
            .filter_map(|call| match call {
                Call::Move(recorded) => Some(recorded),
                Call::Refresh(_) => None,
            })
            .collect()
    }

    /// The folders that were relisted.
    pub fn refreshes(&self) -> Vec<PathBuf> {
        let calls = self.calls();
        calls
            .into_iter()
            .filter_map(|call| match call {
                Call::Refresh(folder) => Some(folder),
                Call::Move(_) => None,
            })
            .collect()
    }

    /// Asserts the device was only asked for moves it can do: a rename in
    /// one folder, or a move to another folder under the same name.
    pub fn assert_only_moves_a_device_can_do(&self) {
        for RecordedMove { from, to } in self.moves() {
            let same_folder = from.parent() == to.parent();
            let same_name = from.file_name() == to.file_name();
            assert!(
                same_folder || same_name,
                "impossible device move {} -> {}",
                from.display(),
                to.display()
            );
        }
    }

    /// Adds `call` to the log.
    fn record(&self, call: Call) {
        self.calls.lock().expect("call log").push(call);
    }
}

impl Provider for Device {
    fn has_sibling_staging(&self) -> bool {
        true
    }

    fn move_native(
        &self,
        node: &LocalNode,
        target: &dyn Node,
        cancel: Option<&Cancellation>,
    ) -> Result<(), TransferError> {
        let target_path = local_path_of(target);
        self.record(Call::Move(RecordedMove {
            from: node.local_path().to_path_buf(),
            to: target_path.clone(),
        }));
        if let Some(cancel) = cancel {
            cancel.check()?;
        }
        if node.local_path().parent() == target_path.parent() {
            if std::fs::symlink_metadata(&target_path).is_ok() {
                return Err(TransferError::Exists(format!(
                    "An item named “{}” already exists.",
                    target.display_name()
                )));
            }
            return node.local_move_native(target, cancel);
        }
        if self.move_object == MoveObject::Unsupported {
            return Err(TransferError::NotSupported("Operation not supported".into()));
        }
        if node.name() != target.name() {
            return Err(TransferError::failed(
                "This device can move an item to another folder or rename it, but not both in one step.",
            ));
        }
        node.local_move_native(target, cancel)
    }

    fn replace_native(
        &self,
        _node: &LocalNode,
        _target: &dyn Node,
        _cancel: Option<&Cancellation>,
    ) -> Result<(), TransferError> {
        Err(TransferError::ReplaceUnsupported(
            "This device cannot replace an item in one step.".into(),
        ))
    }

    fn refresh_listing(&self, node: &LocalNode, _cancel: Option<&Cancellation>) -> Result<(), TransferError> {
        self.record(Call::Refresh(node.local_path().to_path_buf()));
        Ok(())
    }
}

/// A cross-folder move the phone has not forgotten yet: its old path still
/// resolves to the moved object.
struct StaleMove {
    old: PathBuf,
    new: PathBuf,
}

/// A phone whose object paths behave like `GVfs` MTP: after a cross-folder
/// move the old path keeps resolving to the moved object until its folder
/// is listed again.
#[derive(Default)]
pub struct Phone {
    /// The device behaviour and its call log.
    pub device: Device,
    /// Copies come from the same phone, so they keep the source's name
    /// (MTP `CopyObject`).
    same_device_copy: bool,
    stale_move: Mutex<Option<StaleMove>>,
}

impl Phone {
    /// A phone that `device` describes, receiving copies of items that are
    /// already on it.
    pub fn with_same_device_copies(device: Device) -> Self {
        Self {
            device,
            same_device_copy: true,
            stale_move: Mutex::default(),
        }
    }

    /// True while an old path of a cross-folder move still resolves.
    pub fn has_stale_move(&self) -> bool {
        self.stale_move.lock().expect("stale move").is_some()
    }

    /// The moved object that `path` still resolves to, if `path` is the
    /// old path of a move the phone has not forgotten.
    fn stale_target(&self, path: &Path) -> Option<PathBuf> {
        let stale = self.stale_move.lock().expect("stale move");
        let stale = stale.as_ref()?;
        (stale.old == path).then(|| stale.new.clone())
    }
}

impl Provider for Phone {
    fn base(&self) -> Option<&dyn Provider> {
        Some(&self.device)
    }

    fn uri(&self, node: &LocalNode) -> String {
        file_uri(node.local_path()).replacen("file://", "mtp://test-device", 1)
    }

    fn path(&self, _node: &LocalNode) -> Option<PathBuf> {
        None
    }

    fn native_copy_keeps_name(&self, _node: &LocalNode, _target_folder: &dyn Node) -> bool {
        self.same_device_copy
    }

    fn copy_file(
        &self,
        node: &LocalNode,
        target: &dyn Node,
        cancel: &Cancellation,
        progress: &mut dyn FnMut(u64, u64),
    ) -> Result<(), TransferError> {
        if self.same_device_copy {
            assert_eq!(node.name(), target.name(), "CopyObject retains its source name");
        }
        node.local_copy_file(target, cancel, progress)
    }

    fn move_native(
        &self,
        node: &LocalNode,
        target: &dyn Node,
        cancel: Option<&Cancellation>,
    ) -> Result<(), TransferError> {
        self.device.move_native(node, target, cancel)?;
        let destination = local_path_of(target);
        if node.local_path().parent() != destination.parent() {
            let stale = StaleMove {
                old: node.local_path().to_path_buf(),
                new: destination,
            };
            *self.stale_move.lock().expect("stale move") = Some(stale);
        }
        Ok(())
    }

    fn exists(&self, node: &LocalNode, _cancel: Option<&Cancellation>) -> bool {
        match self.stale_target(node.local_path()) {
            Some(moved) => moved.exists(),
            None => node.local_exists(),
        }
    }

    fn delete(&self, node: &LocalNode) -> Result<(), TransferError> {
        // GVfs can still resolve the old path to the moved object. Correct
        // publication must relist before cleanup gets here.
        match self.stale_target(node.local_path()) {
            Some(moved) => {
                fs::remove_file(moved)?;
                Ok(())
            }
            None => node.local_delete(),
        }
    }

    fn refresh_listing(&self, node: &LocalNode, cancel: Option<&Cancellation>) -> Result<(), TransferError> {
        self.device.refresh_listing(node, cancel)?;
        let mut stale = self.stale_move.lock().expect("stale move");
        let relisted_old_folder = stale
            .as_ref()
            .is_some_and(|stale| stale.old.parent() == Some(node.local_path()));
        if relisted_old_folder {
            *stale = None;
        }
        Ok(())
    }
}
