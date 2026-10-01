// SPDX-License-Identifier: AGPL-3.0-only
//! Open in Terminal: the desktop's terminal in a checked folder.
//!
//! Ports the `openTerminal` branch of `dispatch` in `v2.0.0:desktop/winspace.py`
//! (OPEN-017, OPEN-020). Only a location crosses from the window; the
//! folder is checked against fresh metadata, the previous-versions write
//! guard and the file system, and the terminal comes from the system
//! folders ([`ox_core::integration::open_terminal_in_background`]).

use std::path::{Path, PathBuf};
use std::sync::Arc;

use ox_core::entry::{inspect, Entry, EntryError};
use ox_core::integration::{open_terminal_in_background, DirectoryChecks, Sandbox, TerminalError};
use ox_core::network::local_path;
use ox_core::transfer::Cancellation;
use ox_core::versions::{PreviousVersions, VersionsError};

/// What the terminal's folder is checked against: fresh GIO metadata, the
/// network service's local mount lookup and the previous-versions write
/// guard, as `openTerminal` passes `inspect`, `local_path` and
/// `assert_writable`.
#[derive(Debug, Clone)]
struct AppChecks {
    versions: Arc<PreviousVersions>,
}

impl DirectoryChecks for AppChecks {
    type Refusal = VersionsError;

    fn inspect(&self, uri: &str, cancel: &Cancellation) -> Result<Entry, EntryError> {
        inspect(uri, Some(cancel.cancellable()))
    }

    fn local_path(&self, uri: &str) -> Option<PathBuf> {
        local_path(uri)
    }

    fn check_writable(&self, uri: &str) -> Result<(), VersionsError> {
        self.versions.check_writable(uri)
    }
}

/// Opens the terminal in the folder `uri`, or in the folder of the file
/// `uri`, and returns the toast: "Opened GNOME Terminal in /home/you".
///
/// # Errors
///
/// The [`TerminalError`] of the folder checks or the launch, whose
/// message the window shows, such as a share without a local mount.
pub(crate) async fn open_terminal(
    uri: String,
    settings_directory: &Path,
    sandbox: Sandbox,
) -> Result<String, TerminalError> {
    let checks = AppChecks {
        versions: Arc::new(PreviousVersions::new(settings_directory)),
    };
    let launched = open_terminal_in_background(uri, checks, sandbox, Cancellation::new()).await?;
    Ok(format!(
        "Opened {} in {}",
        launched.terminal,
        launched.path.display()
    ))
}
