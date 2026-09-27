// SPDX-License-Identifier: AGPL-3.0-only
//! Fixtures shared by the unit tests of several modules.

use std::path::Path;
use std::process::Command;

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
