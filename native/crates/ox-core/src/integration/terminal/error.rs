// SPDX-License-Identifier: AGPL-3.0-only
//! Why Open in Terminal was refused or failed, in the words of
//! `desktop/terminal_integration.py`.

use std::io;
use std::path::PathBuf;

use crate::entry::EntryError;
use crate::location::LocationError;

/// A refused or failed Open in Terminal. `Display` is the message the
/// window shows.
#[derive(Debug, thiserror::Error)]
pub enum TerminalError {
    /// None of the supported terminals is installed (OPEN-018).
    #[error("No supported terminal is installed. On Zorin, install GNOME Terminal with: sudo apt install gnome-terminal")]
    NoTerminal,
    /// The terminal's executable is not an absolute path.
    #[error("Unsupported terminal executable.")]
    UnsupportedTerminal,
    /// The folder is not an absolute path, is too long or holds a control
    /// character.
    #[error("The terminal requires a valid absolute local directory.")]
    InvalidDirectory,
    /// The folder turned out to be something else.
    #[error("The terminal destination is not a directory.")]
    NotADirectory,
    /// The user may not enter the folder.
    #[error("You do not have permission to enter this directory.")]
    NoPermission,
    /// The location is a server's list of shares.
    #[error("Open a network share first. A server listing is not a terminal directory.")]
    ServerListing,
    /// The item's folder is a server's list of shares.
    #[error("Open a network share before opening a terminal.")]
    ShareNeeded,
    /// The item is a link, a special file or of unknown type.
    #[error("Open the real folder first; links and special files are not terminal destinations.")]
    LinkOrSpecialFile,
    /// The item is neither a regular file nor a folder.
    #[error("Select a regular file or a directory.")]
    NotFileOrFolder,
    /// The folder is inside a snapshot or backup (OPEN-017).
    #[error("Previous-version locations cannot be opened in Terminal. Restore a copy first.")]
    PreviousVersion,
    /// A read-only location, refused by the app's write guard; the message
    /// is the guard's.
    #[error("{0}")]
    Protected(String),
    /// The share has no local path (OPEN-006).
    #[error(
        "This SMB folder needs a local mount before Terminal can use it. Connect to the share and \
         install gvfs-fuse, or use a persistent CIFS mount. This does not open an SSH session."
    )]
    NeedsLocalMount,
    /// The terminal exited at once with a failure (OPEN-020).
    #[error(
        "{terminal} could not start (exit {code}). Check your terminal installation and desktop session."
    )]
    CouldNotStart {
        /// The terminal's name, for example "GNOME Terminal".
        terminal: &'static str,
        /// The exit status, or the negated signal number.
        code: i32,
    },
    /// The user cancelled.
    #[error("Operation cancelled.")]
    Cancelled,
    /// The location is not one the app can open.
    #[error(transparent)]
    Location(#[from] LocationError),
    /// The item could not be queried; [`EntryError::needs_mount`] asks the
    /// caller to mount the share and try again.
    #[error(transparent)]
    Entry(#[from] EntryError),
    /// Resolving the folder or starting the terminal failed.
    #[error("{error}: {}", path.display())]
    Io {
        /// The folder or program the operation was on.
        path: PathBuf,
        /// What the operating system reported.
        error: io::Error,
    },
}
