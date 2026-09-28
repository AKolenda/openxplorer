// SPDX-License-Identifier: AGPL-3.0-only
//! Named pipes for the integration tests that check that a FIFO in place of
//! a file never blocks. `tests/transfer_support` and `tests/ops_create.rs`
//! include this file by path; the crate's unit tests have their own in
//! `src/test_support.rs`, which integration tests cannot reach.

use std::path::Path;
use std::process::Command;

/// Creates a named pipe (FIFO) at `path` with the system `mkfifo`.
pub fn make_fifo(path: &Path) {
    let status = Command::new("mkfifo")
        .arg(path)
        .status()
        .expect("mkfifo (GNU coreutils) is required for the FIFO safety tests");
    assert!(
        status.success(),
        "mkfifo failed to create the FIFO fixture: {status}"
    );
}
