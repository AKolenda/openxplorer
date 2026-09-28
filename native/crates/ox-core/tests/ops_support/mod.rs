// SPDX-License-Identifier: AGPL-3.0-only
//! Helpers shared by the `ops_*` test binaries that touch files: awaiting
//! an operation and addressing temporary files by URI. Every binary that
//! includes this module uses all of it.

use std::future::Future;
use std::path::Path;

use gio::prelude::*;

/// Awaits `operation` on a private main context, the way the interface
/// awaits it on the GTK main loop.
pub fn block_on<F: Future>(operation: F) -> F::Output {
    glib::MainContext::new().block_on(operation)
}

/// The `file://` URI GIO gives the local item at `path`.
pub fn file_uri(path: &Path) -> String {
    gio::File::for_path(path).uri().to_string()
}
