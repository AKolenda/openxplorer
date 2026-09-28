// SPDX-License-Identifier: AGPL-3.0-only
//! Helpers shared by the `ops_*` test binaries that touch files: awaiting
//! an operation and addressing temporary files by URI. Every binary that
//! includes this module uses all of it; the helpers only some binaries use
//! are in the files next to it, which those binaries include by path:
//!
//! | File | Helper |
//! |---|---|
//! | `folders.rs` | A source and a destination folder |
//! | `snapshots.rs` | The previous-versions write protection |
//! | `private_trash.rs` | The check that the Recycle Bin is the test run's own |

#[path = "../common/uri.rs"]
mod uri;

use std::future::Future;

pub use uri::file_uri;

/// Awaits `operation` on a private main context, the way the interface
/// awaits it on the GTK main loop.
pub fn block_on<F: Future>(operation: F) -> F::Output {
    glib::MainContext::new().block_on(operation)
}
