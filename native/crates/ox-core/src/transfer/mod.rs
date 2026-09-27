// SPDX-License-Identifier: AGPL-3.0-only
//! Copy, move, Trash and permanent delete.
//!
//! Ports `desktop/operations.py` and its safety rules exactly:
//! - a copy is staged privately and published only when complete, so a
//!   partial copy is never visible under its final name;
//! - Replace never loses the existing item before the new one is installed;
//! - a move never degrades to copy-then-delete;
//! - Trash never falls back to permanent deletion;
//! - the engine only deletes staging it created, and reports any it cannot.
//!
//! The Python tests in `desktop/tests/test_operations.py`,
//! `desktop/tests/test_device_staging.py` and the transfer cases in
//! `desktop/tests/test_v05.py` are the specification for `engine.rs`.

mod engine;
mod node;

pub use engine::{ConflictPolicy, Progress, TransferEngine, TransferMode, TransferResult};
pub use node::{Cancellation, Node, NodeFactory, NodeInfo, NodeKind, TransferError, WriteGuard};
