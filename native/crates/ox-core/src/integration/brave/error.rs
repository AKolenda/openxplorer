// SPDX-License-Identifier: AGPL-3.0-only
//! Why a Brave download-folder change or restore was refused or failed,
//! in the words of `v2.0.0:desktop/brave_integration.py`.

use std::io;
use std::path::PathBuf;

/// A refused or failed Brave change. `Display` is the message the Brave
/// dialog shows.
#[derive(Debug, thiserror::Error)]
pub enum BraveError {
    /// The consent checkbox was not ticked for a sync (INT-020).
    #[error("{}", crate::i18n::gettext("Confirm updating the selected Brave profiles."))]
    SyncNotConfirmed,
    /// The consent checkbox was not ticked for a restore (INT-021).
    #[error(
        "{}",
        crate::i18n::gettext("Confirm restoring the previous Brave download directory.")
    )]
    RestoreNotConfirmed,
    /// Not 1 to 40 profiles, or a profile named twice.
    #[error("{}", crate::i18n::gettext("Select 1–40 distinct Brave profiles."))]
    InvalidSelection,
    /// The download folder contains a NUL character.
    #[error("{}", crate::i18n::gettext("Invalid download directory."))]
    InvalidDirectory,
    /// The download folder is not an absolute path.
    #[error("{}", crate::i18n::gettext("Use an absolute download directory."))]
    RelativeDirectory,
    /// The download folder does not exist, is not a folder, or is not
    /// writable and enterable.
    #[error(
        "{}",
        crate::i18n::gettext("Downloads must be an existing writable local or persistent-mount path.")
    )]
    UnusableDirectory,
    /// The download folder is volatile (`/run`, `/proc`, `/sys`, `/dev`),
    /// the home folder or `/`.
    #[error("{}", crate::i18n::gettext("Use a persistent, dedicated download directory."))]
    NotDedicated,
    /// Brave is running, or its processes cannot be inspected.
    #[error("{}", crate::i18n::gettext("Fully quit Brave, including background processes, then retry. OpenXplorer will not force it to close."))]
    BraveRunningBeforeSync,
    /// Brave is running, or its processes cannot be inspected.
    #[error("{}", crate::i18n::gettext("Fully quit Brave before restoring."))]
    BraveRunningBeforeRestore,
    /// The profile is not one of the detected native profiles.
    #[error("{}", crate::i18n::gettext("Choose a detected native Brave profile. Custom, Snap and Flatpak profiles require manual browser settings."))]
    UnknownProfile,
    /// `download` or `savefile` in the preferences is not an object.
    #[error("{}", crate::i18n::gettext("Unsupported browser preference structure."))]
    UnsupportedStructure,
    /// The backup folder is a symlink.
    #[error("{}", crate::i18n::gettext("Refusing a symlinked backup directory."))]
    SymlinkedBackupFolder,
    /// Brave started, or its preferences changed, while a profile was
    /// being updated.
    #[error(
        "{}",
        crate::i18n::gettext("Brave started or its preferences changed. Close Brave and retry.")
    )]
    ChangedDuringSync,
    /// Brave started, or its preferences changed, while restoring.
    #[error(
        "{}",
        crate::i18n::gettext("Browser preferences changed; retry after closing Brave.")
    )]
    ChangedDuringRestore,
    /// No undo record exists for the profile.
    #[error(
        "{}",
        crate::i18n::gettext("No previous download setting was recorded for this profile.")
    )]
    NoRecord,
    /// The user or Brave changed both download folders since the sync.
    #[error(
        "{}",
        crate::i18n::gettext(
            "Brave settings changed since OpenXplorer last updated them. Nothing was overwritten."
        )
    )]
    ChangedSinceSync,
    /// The preference file is a symlink.
    #[error("{}", crate::i18n::gettext("Refusing a symlinked browser preference file."))]
    SymlinkedPreferences,
    /// The preference file is not a regular file owned by the user, or is
    /// over 32 MB.
    #[error(
        "{}",
        crate::i18n::gettext("Browser preferences are not a supported private regular file.")
    )]
    NotPrivateRegularFile,
    /// The preference file holds JSON that is not an object.
    #[error("{}", crate::i18n::gettext("Browser preferences are not an object."))]
    NotAnObject,
    /// The preference file or undo record is not valid JSON of the
    /// expected shape.
    #[error("{0}")]
    InvalidJson(#[from] serde_json::Error),
    /// The app runs in a Flatpak sandbox, which cannot see or change
    /// the host's Brave profiles.
    #[error(
        "{}",
        crate::i18n::gettext(
            "OpenXplorer runs as a Flatpak and cannot change Brave's settings. \
         Set the download folder in brave://settings/downloads."
        )
    )]
    Sandboxed,
    /// Reading or writing a file failed.
    #[error("{error}: {}", path.display())]
    Io {
        /// The file the operation was on.
        path: PathBuf,
        /// What the operating system reported.
        error: io::Error,
    },
}

impl BraveError {
    /// An I/O error on `path`.
    pub(super) fn io(path: impl Into<PathBuf>, error: io::Error) -> Self {
        Self::Io {
            path: path.into(),
            error,
        }
    }
}
