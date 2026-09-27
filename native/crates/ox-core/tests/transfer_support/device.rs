// SPDX-License-Identifier: AGPL-3.0-only
//! Test double of a GVfs MTP destination: `DeviceNode` in
//! `desktop/tests/test_device_staging.py`.
//!
//! It follows the adapter contract `GioNode` provides for `mtp://`, as
//! measured on a Pixel 9 with GVfs 1.54.4: a same-folder move is a
//! non-overwriting rename (`set_display_name`); a cross-folder move keeps
//! the item's name; a cross-folder move under a different name is refused;
//! one-step overwrite is unsupported. Every move and relist is recorded.

use std::path::PathBuf;
use std::sync::Mutex;

use ox_core::transfer::{Cancellation, Node, TransferError};

use super::local::{local_path_of, LocalNode, Provider};

/// One device call, for assertions (Python `CALLS`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Call {
    /// `move_native` from the first path to the second.
    Move(PathBuf, PathBuf),
    /// `refresh_listing` of a folder.
    Refresh(PathBuf),
}

/// The device behaviour and its call log.
#[derive(Default)]
pub struct Device {
    calls: Mutex<Vec<Call>>,
}

impl Device {
    /// Every recorded call, in order.
    pub fn calls(&self) -> Vec<Call> {
        self.calls.lock().expect("call log").clone()
    }

    /// The recorded moves as `(from, to)`.
    pub fn moves(&self) -> Vec<(PathBuf, PathBuf)> {
        let calls = self.calls();
        calls
            .into_iter()
            .filter_map(|call| match call {
                Call::Move(from, to) => Some((from, to)),
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
                Call::Move(..) => None,
            })
            .collect()
    }

    /// Asserts the device was only asked for moves it can do: a rename in
    /// one folder, or a move to another folder under the same name.
    pub fn assert_only_same_folder_renames_across_names(&self) {
        for (from, to) in self.moves() {
            let same_folder = from.parent() == to.parent();
            let same_name = from.file_name() == to.file_name();
            assert!(
                same_folder || same_name,
                "impossible device move {from:?} -> {to:?}"
            );
        }
    }

    fn record(&self, call: Call) {
        self.calls.lock().expect("call log").push(call);
    }
}

impl Provider for Device {
    fn stage_as_sibling(&self) -> bool {
        true
    }

    fn move_native(
        &self,
        node: &LocalNode,
        target: &dyn Node,
        cancel: Option<&Cancellation>,
    ) -> Result<(), TransferError> {
        let target_path = local_path_of(target);
        self.record(Call::Move(node.local_path().to_path_buf(), target_path.clone()));
        if let Some(cancel) = cancel {
            cancel.check()?;
        }
        if node.local_path().parent() == target_path.parent() {
            if std::fs::symlink_metadata(&target_path).is_ok() {
                return Err(TransferError::Exists(format!(
                    "An item named “{}” already exists.",
                    target.name()
                )));
            }
            return node.local_move_native(target, cancel);
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
