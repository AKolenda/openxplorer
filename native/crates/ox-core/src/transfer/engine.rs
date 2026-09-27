// SPDX-License-Identifier: AGPL-3.0-only
//! The transfer orchestration. Port of `TransferEngine` in
//! `desktop/operations.py`; see the module documentation for the rules.

use super::node::{Cancellation, NodeFactory, TransferError};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransferMode {
    Copy,
    Move,
    Trash,
    /// Permanent delete, only after explicit confirmation.
    Delete,
}

/// What to do when a destination name already exists.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConflictPolicy {
    Skip,
    Replace,
    KeepBoth,
}

/// Progress for the transfer panel. `fraction` is per file for copies.
#[derive(Debug, Clone, PartialEq)]
pub struct Progress {
    pub label: String,
    pub fraction: f64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TransferResult {
    pub done: Vec<String>,
    pub skipped: Vec<String>,
    pub errors: Vec<String>,
    pub cancelled: bool,
}

type Emit = Box<dyn FnMut(Progress) + Send>;
type AssertWritable = Box<dyn Fn(&str) -> Result<(), TransferError> + Send + Sync>;
type Sleep = Box<dyn Fn(std::time::Duration) + Send + Sync>;

pub struct TransferEngine {
    factory: NodeFactory,
    emit: Emit,
    assert_writable: Option<AssertWritable>,
    sleep: Sleep,
}

impl TransferEngine {
    pub fn new(factory: NodeFactory) -> Self {
        Self {
            factory,
            emit: Box::new(|_| {}),
            assert_writable: None,
            sleep: Box::new(std::thread::sleep),
        }
    }

    pub fn with_progress(mut self, emit: impl FnMut(Progress) + Send + 'static) -> Self {
        self.emit = Box::new(emit);
        self
    }

    /// Rejects writes into protected locations such as snapshot folders.
    pub fn with_write_guard(
        mut self,
        guard: impl Fn(&str) -> Result<(), TransferError> + Send + Sync + 'static,
    ) -> Self {
        self.assert_writable = Some(Box::new(guard));
        self
    }

    /// Replaces the delay used between device cleanup retries (tests).
    pub fn with_sleep(mut self, sleep: impl Fn(std::time::Duration) + Send + Sync + 'static) -> Self {
        self.sleep = Box::new(sleep);
        self
    }

    /// Runs one operation over `uris`. `target` is the destination folder
    /// for copies and moves.
    pub fn run(
        &mut self,
        mode: TransferMode,
        uris: &[String],
        target: Option<&str>,
        policy: ConflictPolicy,
        cancel: &Cancellation,
    ) -> Result<TransferResult, TransferError> {
        let _ = (
            &self.factory,
            &self.assert_writable,
            &self.sleep,
            mode,
            uris,
            target,
            policy,
            cancel,
        );
        (self.emit)(Progress {
            label: "0 item(s) completed".into(),
            fraction: 1.0,
        });
        Err(TransferError::NotSupported(
            "The native transfer engine is not implemented yet.".into(),
        ))
    }
}
