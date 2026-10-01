// SPDX-License-Identifier: AGPL-3.0-only
//! Open in Terminal: choosing the folder and the terminal, and starting it.
//!
//! Folders and processes are real; the metadata, local-path lookup and
//! write guard are test doubles, as in Python.
//!
//! | Case file | What it covers | Ports |
//! |---|---|---|
//! | `folder` | The folder a terminal opens in | `TerminalTests` of `v2.0.0:desktop/tests/test_terminal_security.py`, `DispatchTests` of `v2.0.0:desktop/tests/test_rc2.py` |
//! | `launch` | Which terminal, and starting it | `TerminalTests` of `v2.0.0:desktop/tests/test_terminal_security.py` |

mod integration_support;

use std::fs;
use std::path::{Path, PathBuf};

use tempfile::TempDir;

/// A new, empty temporary folder.
fn temporary_folder() -> TempDir {
    tempfile::tempdir().expect("temporary folder")
}

/// `folder` with every link resolved, as the terminal receives it.
fn real_path(folder: &Path) -> PathBuf {
    fs::canonicalize(folder).expect("an existing folder")
}

#[path = "integration_terminal/folder.rs"]
mod folder;
#[path = "integration_terminal/launch.rs"]
mod launch;
