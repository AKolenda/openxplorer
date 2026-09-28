// SPDX-License-Identifier: AGPL-3.0-only
//! Fixtures shared by the unit tests of several modules.

use std::fs;
use std::os::unix::fs::MetadataExt;
use std::path::Path;
use std::process::Command;

use tempfile::TempDir;

/// The permission bits of `path` itself, not of a link's target, for
/// example `0o600`, including the set-id and sticky bits.
pub(crate) fn permission_bits(path: &Path) -> u32 {
    fs::symlink_metadata(path).expect("the path exists").mode() & 0o7777
}

/// A new temporary folder for one test, removed when dropped.
pub(crate) fn temporary_folder() -> TempDir {
    tempfile::tempdir().expect("the test home has room for a temporary folder")
}

/// Creates a named pipe (FIFO) at `path` with the system `mkfifo`, for the
/// safety tests that check that a FIFO in place of a file never blocks.
pub(crate) fn make_fifo(path: &Path) {
    let status = Command::new("mkfifo")
        .arg(path)
        .status()
        .expect("mkfifo (GNU coreutils) is required for the FIFO safety tests");
    assert!(
        status.success(),
        "mkfifo failed to create the FIFO fixture: {status}"
    );
}
