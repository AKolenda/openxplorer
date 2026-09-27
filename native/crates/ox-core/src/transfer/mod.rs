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
//! | `names` | Staging, backup and validated child names |
//! | `labels` | Progress text |
//! | `error` | [`TransferError`] and how backend errors map onto it |
//!
//! The transfer cases of `desktop/tests/test_operations.py`,
//! `desktop/tests/test_device_staging.py`, `desktop/tests/test_rc2.py` and
//! the engine cases of `desktop/tests/gio_integration.py` are ported to
//! `tests/transfer.rs` and `tests/transfer_cases/`, against temporary local
//! files, device test doubles and the production GIO adapter; each port
//! names the Python test it comes from. The ZIP extraction cases of
//! `desktop/tests/test_zip_extract.py` wait for the ZIP extractor. Native
//! backend limitations are documented in [`crate::gio_node`].

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
pub(crate) use guard::nesting_error;
pub use guard::{check_write_tree, guard_destination, SourceChange, MAX_DEPTH};
pub use modes::secure_local_staging;
pub use names::{backup_name, is_own_staging_name, staging_name};
pub use node::{Cancellation, ItemIdentity, Node, NodeFactory, NodeInfo, NodeKind, WriteGuard};
pub use staging::clean_staging;
pub(crate) use staging::STAGING_LEVELS;
pub use types::{ConflictPolicy, Progress, TransferMode, TransferResult};
