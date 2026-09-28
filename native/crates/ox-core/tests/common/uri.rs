// SPDX-License-Identifier: AGPL-3.0-only
//! The URIs of temporary test files, for the integration tests that compare
//! them with what the GIO adapter reports. `tests/transfer_support` and
//! `tests/ops_support` include this file by path.

use std::path::Path;

use gio::prelude::*;

/// The `file://` URI GIO gives the local item at `path`. Unlike
/// `ox_core::location::file_uri` it leaves `(` and `)` unescaped, exactly
/// as the GIO adapter reports the items it creates.
pub fn file_uri(path: &Path) -> String {
    gio::File::for_path(path).uri().to_string()
}
