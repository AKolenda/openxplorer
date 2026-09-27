// SPDX-License-Identifier: AGPL-3.0-only
//! Copy, move, Trash and permanent delete.
//!
//! Ports `desktop/operations.py` and preserves its transfer safety rules.
//! Each rule is named by the id of the feature in
//! `native/parity/features.toml` that specifies it. The code that enforces a
//! rule names that id in a comment, and the tests that prove it carry a
//! parity marker with the same id. The main rules:
//!
//! - XFER-001: a copy is staged privately and published only when complete,
//!   so a partial copy is never visible under its final name.
//! - XFER-002: the engine only deletes staging it created itself, and
//!   XFER-003: it reports any it cannot remove, with its location.
//! - XFER-007: publishing never overwrites a name that appeared meanwhile.
//! - XFER-009 and XFER-010: Replace never loses the existing item before
//!   the new one is installed.
//! - XFER-011: a move never degrades to copy-then-delete.
//! - XFER-014: Trash never falls back to permanent deletion.
//! - XFER-020: a protected location anywhere in an affected tree stops the
//!   item before anything changes.
//!
//! | Module | Responsibility |
//! |---|---|
//! | `engine` | The public API and the per-item loop (`run`, `_run_items`) |
//! | `request` | Validating a run's request: its operation, items and destination folder |
//! | `batch` | What a run does with each item, and the settings its items share |
//! | `conflicts` | Skip, Keep both and Replace: the destination name |
//! | `staged_copy` | Staging, publishing and device checks for one copy |
//! | `copy` | The recursive copy into staging (`_copy`) |
//! | `commit` | Publishing, Replace and reversible replacement |
//! | `staging` | Removing the engine's own staging (`_discard_stage`) |
//! | `containment` | Refusing to place a folder inside itself |
//! | `guard` | The write-guard preflight and the nesting limit |
//! | `modes` | Unix modes of local staging folders |
//! | `names` | Staging, backup and validated child names |
//! | `labels` | Progress text |
//! | `relisting` | Relisting the folders moves took items from (MTP) |
//! | `node` | The [`Node`] storage abstraction the engine works on |
//! | `cancellation` | [`Cancellation`], the user's stop request |
//! | `types` | Operations, modes, conflict policies, progress and the run's result |
//! | `error` | [`TransferError`] and how backend errors map onto it |
//!
//! Every test of `desktop/tests/test_operations.py` and
//! `desktop/tests/test_device_staging.py`, and the engine cases of
//! `desktop/tests/gio_integration.py`, is ported to `tests/transfer.rs`,
//! `tests/transfer_cases/` and `tests/gio_node.rs`, against temporary local
//! files, device test doubles and the production GIO adapter; each port
//! names the Python test it comes from. Still to come with the features
//! they test: the rename cases of `gio_integration.py`, the bridge dispatch
//! of `desktop/tests/test_rc2.py` (its engine half is ported) and the ZIP
//! extraction cases of `desktop/tests/test_zip_extract.py`. Native backend
//! limitations are documented in [`crate::gio_node`].

mod batch;
mod cancellation;
mod commit;
mod conflicts;
mod containment;
mod copy;
mod engine;
mod error;
mod guard;
mod labels;
mod modes;
mod names;
mod node;
mod relisting;
mod request;
mod staged_copy;
mod staging;
mod types;

pub(crate) use cancellation::check_cancelled;
pub use cancellation::Cancellation;
pub(crate) use commit::verify_installation;
pub use engine::TransferEngine;
pub use error::TransferError;
pub(crate) use guard::nesting_error;
pub use guard::MAX_DEPTH;
pub(crate) use modes::PRIVATE_DIRECTORY_MODE;
pub use names::is_own_staging_name;
pub use node::{ItemIdentity, Node, NodeFactory, NodeInfo, NodeKind, WriteGuard};
pub use request::MAX_ITEMS;
pub(crate) use staging::{clean_staging, STAGING_LEVELS};
pub use types::{ConflictPolicy, Operation, Progress, TransferMode, TransferResult};
