// SPDX-License-Identifier: AGPL-3.0-only
//! Copy, move, Trash and permanent delete.
//!
//! Ports `desktop/operations.py` and preserves its transfer safety rules:
//! - a copy is staged privately and published only when complete, so a
//!   partial copy is never visible under its final name;
//! - Replace never loses the existing item before the new one is installed;
//! - a move never degrades to copy-then-delete;
//! - Trash never falls back to permanent deletion;
//! - the engine only deletes staging it created, and reports any it cannot.
//!
//! Each rule is documented at the code that enforces it:
//!
//! | Module | Responsibility |
//! |---|---|
//! | `engine` | The public API and the per-item loop (`run`, `_run_items`) |
//! | `staged_copy` | Staging, publishing and device checks for one copy |
//! | `copy` | The recursive copy into staging (`_copy`) |
//! | `commit` | Publishing, Replace and reversible replacement |
//! | `staging` | Removing the engine's own staging (`_discard_stage`) |
//! | `guard` | Self/descendant checks and the write preflight |
//! | `modes` | Unix modes of local staging folders |
//! | `names` | Staging, backup and "Keep both" names |
//! | `labels` | Progress text |
//!
//! The Python tests in `desktop/tests/test_operations.py`,
//! `desktop/tests/test_device_staging.py` and the transfer cases in
//! `desktop/tests/test_rc2.py`, `desktop/tests/gio_integration.py` and
//! `desktop/tests/test_zip_extract.py` are ported to
//! `tests/transfer.rs` and `tests/transfer_cases/`, against temporary local
//! files and device test doubles. `tests/gio_node.rs` separately exercises
//! the production GIO adapter on local files. Native backend limitations
//! are documented in [`crate::gio_node`].

mod commit;
mod copy;
mod engine;
mod error;
mod guard;
mod labels;
mod modes;
mod names;
mod node;
mod staged_copy;
mod staging;
mod types;

pub(crate) use commit::verify_installation;
pub use engine::{TransferEngine, MAX_ITEMS};
pub use error::TransferError;
pub use guard::{check_write_tree, guard_destination, MAX_DEPTH};
pub use modes::secure_local_staging;
pub use names::{backup_name, is_own_staging_name, new_copy_name, staging_name};
pub use node::{Cancellation, Node, NodeFactory, NodeInfo, NodeKind, WriteGuard};
pub use staging::clean_staging;
pub use types::{ConflictPolicy, Progress, TransferMode, TransferResult};
