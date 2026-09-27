// SPDX-License-Identifier: AGPL-3.0-only
//! Helpers shared by the settings tests.

use std::fs;
use std::os::unix::fs::MetadataExt;
use std::path::Path;

/// The permission bits of `path`, for example `0o600`.
pub(super) fn mode(path: &Path) -> u32 {
    fs::metadata(path).expect("the path exists").mode() & 0o777
}
