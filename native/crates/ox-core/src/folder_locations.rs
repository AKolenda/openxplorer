// SPDX-License-Identifier: AGPL-3.0-only
//! Moving an XDG standard folder (Downloads, Documents, ...) to another
//! existing folder, safely and reversibly.
//!
//! Ports `FolderLocations.validate`, `apply` and `snapshot` of
//! `desktop/folder_locations.py` (PROP-031); the Properties Location tab
//! (PROP-017) is its interface. Reading `user-dirs.dirs` is
//! [`places::FolderLocations`](crate::places::FolderLocations).
//!
//! The safety rules, each enforced and documented where it applies:
//!
//! - **No files move.** Only the configuration changes; the old folder and
//!   everything in it stay where they are.
//! - **Only persistent destinations** ([`check`]): an existing folder the
//!   user can write to, never a temporary or per-login `GVfs` path, the
//!   home folder or `/`; an `smb://` address only through a stable kernel
//!   CIFS mount.
//! - **Explicit consent** ([`Consent`]) before a change.
//! - **A private backup first** ([`history`]): `user-dirs.dirs` is copied
//!   to a 0600 file in a 0700 folder before `xdg-user-dirs-update` runs.
//! - **Verified, not assumed** ([`FolderRelocation::apply`]): the file is
//!   read back and must name the new path.
//!
//! | Module | Responsibility |
//! |---|---|
//! | `check` | Validating a destination |
//! | `history` | The backup and the "Use previous" history |
//! | `updater` | Running `xdg-user-dirs-update` |

mod check;
mod history;
mod updater;

#[cfg(test)]
mod tests;

use std::io;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use crate::location::LocationError;
use crate::network::{read_mount_table, MountEntry};
use crate::places::{FolderLocations, KnownFolder};
use crate::private_storage::StorageError;

pub use check::CheckedLocation;
pub use updater::{UserDirsUpdater, XdgUserDirsUpdate};

/// Reads the mount table; tests supply their own.
pub type MountReader = Box<dyn Fn() -> io::Result<Vec<MountEntry>> + Send + Sync>;

/// Why a standard folder could not be checked or moved, in the Python
/// app's words.
#[derive(Debug, thiserror::Error)]
pub enum RelocationError {
    /// The address itself is invalid.
    #[error(transparent)]
    Address(#[from] LocationError),
    /// An `smb://` address without a kernel mount of its share.
    #[error(
        "This SMB folder is not mounted at a stable Linux path. Use “Set up network mount”, or mount it \
         with CIFS first. A sidebar bookmark alone is not enough."
    )]
    NotMounted,
    /// Under `/run`, `/tmp` or `/var/tmp`.
    #[error("Use a persistent location, not a temporary or per-login GVfs path.")]
    Temporary,
    /// On a `GVfs` FUSE mount.
    #[error("GVfs session paths cannot be used as persistent standard folders.")]
    GvfsSession,
    /// Missing, or not a folder.
    #[error("The new location must be an existing folder.")]
    NotAFolder,
    /// The home folder itself or `/`.
    #[error("Choose a dedicated folder, not your entire home directory or the filesystem root.")]
    WholeHomeOrRoot,
    /// Not writable or not enterable by this user.
    #[error("You do not have write access to this folder.")]
    NotWritable,
    /// The path could not be resolved for another reason.
    #[error("The new location could not be read: {0}.")]
    Unreadable(io::Error),
    /// `/proc/self/mountinfo` could not be read.
    #[error("The mount table could not be read: {0}.")]
    MountTable(io::Error),
    /// [`apply`](FolderRelocation::apply) without [`Consent::Given`].
    #[error("Confirm the new location before applying it.")]
    NotConfirmed,
    /// `xdg-user-dirs-update` is not installed.
    #[error("Install xdg-user-dirs before changing a standard folder.")]
    UpdaterMissing,
    /// `xdg-user-dirs-update` failed or did not finish in time.
    #[error("The XDG folder setting could not be changed.")]
    UpdaterFailed,
    /// `user-dirs.dirs` does not name the new path after the update.
    #[error(
        "The folder configuration did not retain the requested path. A backup was saved; no user files \
         were moved."
    )]
    NotRetained,
    /// The backup or the history could not be written.
    #[error(transparent)]
    Storage(#[from] StorageError),
}

/// Whether the user agreed to change the folder: the Location tab's
/// "Change the system folder location; leave existing files in place."
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Consent {
    /// The box is ticked.
    Given,
    /// The box is not ticked.
    Missing,
}

/// Where a standard folder is, where it was before, and its default: what
/// the Location tab starts from (`FolderLocations.snapshot`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StandardFolderLocation {
    /// The folder.
    pub folder: KnownFolder,
    /// Where `user-dirs.dirs` puts it now.
    pub path: PathBuf,
    /// `~/<Label>`, for "Restore default".
    pub default_path: PathBuf,
    /// Where it was before the last change made here, for "Use previous".
    pub previous_path: Option<PathBuf>,
}

/// What [`FolderRelocation::apply`] did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LocationChange {
    /// The folder is already there; nothing was written.
    Unchanged,
    /// The folder moved; `backup` holds the previous `user-dirs.dirs`.
    Changed {
        /// The private copy of the configuration before the change.
        backup: PathBuf,
    },
}

/// A standard folder moved by [`FolderRelocation::apply`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppliedLocation {
    /// The checked destination, with the folder's previous path.
    pub location: CheckedLocation,
    /// Whether the configuration changed.
    pub change: LocationChange,
}

/// Checks and changes where the standard folders are.
pub struct FolderRelocation {
    locations: FolderLocations,
    state_directory: PathBuf,
    updater: Box<dyn UserDirsUpdater>,
    read_mounts: MountReader,
    temporary_roots: Vec<PathBuf>,
    /// One change at a time in this process (`self.lock` in Python), so
    /// two backups and history writes never interleave.
    applying: Mutex<()>,
}

impl std::fmt::Debug for FolderRelocation {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("FolderRelocation")
            .field("locations", &self.locations)
            .field("state_directory", &self.state_directory)
            .finish_non_exhaustive()
    }
}

impl FolderRelocation {
    /// Moves the folders of `locations`, keeping backups and the history
    /// in `state_directory` (`~/.config/winspace`), through the desktop's
    /// `xdg-user-dirs-update` and this process's mount table.
    pub fn new(locations: FolderLocations, state_directory: PathBuf) -> Self {
        Self {
            locations,
            state_directory,
            updater: Box::new(XdgUserDirsUpdate::detect()),
            read_mounts: Box::new(read_mount_table),
            temporary_roots: check::TEMPORARY_ROOTS.map(PathBuf::from).to_vec(),
            applying: Mutex::new(()),
        }
    }

    /// The same, changing the configuration through `updater`.
    #[must_use]
    pub fn with_updater(self, updater: impl UserDirsUpdater + 'static) -> Self {
        Self {
            updater: Box::new(updater),
            ..self
        }
    }

    /// The same, reading mounts with `read_mounts`.
    #[must_use]
    pub fn with_mount_reader(self, read_mounts: MountReader) -> Self {
        Self { read_mounts, ..self }
    }

    /// The same, refusing destinations under `roots` instead of `/run`,
    /// `/tmp` and `/var/tmp`, for tests whose folders are all temporary.
    #[cfg(test)]
    fn with_temporary_roots(self, roots: Vec<PathBuf>) -> Self {
        Self {
            temporary_roots: roots,
            ..self
        }
    }

    /// Where `folder` is now, its default and its previous path. Reads
    /// `user-dirs.dirs` and the history.
    pub fn location_of(&self, folder: KnownFolder) -> StandardFolderLocation {
        let paths = self.locations.read_paths();
        let previous_path = history::previous_path(&self.state_directory, folder);
        StandardFolderLocation {
            folder,
            path: paths.path(folder).to_owned(),
            default_path: self.locations.home().join(folder.label()),
            previous_path,
        }
    }

    /// Checks that `value` (a path, `~/…`, a `file:` URI or an `smb://`
    /// address) can hold `folder` (`validate` in Python).
    ///
    /// # Errors
    ///
    /// A [`RelocationError`] naming the rule the destination breaks.
    pub fn check(&self, folder: KnownFolder, value: &str) -> Result<CheckedLocation, RelocationError> {
        let mounts = (self.read_mounts)().map_err(RelocationError::MountTable)?;
        let surroundings = check::Surroundings {
            home: self.locations.home(),
            mounts: &mounts,
            temporary_roots: &self.temporary_roots,
        };
        let path = check::destination(value, surroundings)?;
        let previous = self.locations.read_paths().path(folder).to_owned();
        Ok(CheckedLocation::new(folder, path, &mounts, previous))
    }

    /// Checks `value` again and makes it `folder`'s location: backs up
    /// `user-dirs.dirs`, runs `xdg-user-dirs-update --set`, reads the file
    /// back and records the previous path. No files are moved.
    ///
    /// # Errors
    ///
    /// [`RelocationError::NotConfirmed`] without consent, a failed check,
    /// a failed backup or update, or [`RelocationError::NotRetained`] when
    /// the file does not name the new path afterwards.
    pub fn apply(
        &self,
        folder: KnownFolder,
        value: &str,
        consent: Consent,
    ) -> Result<AppliedLocation, RelocationError> {
        // Safety rule "explicit consent" (`confirmed is not True` in
        // folder_locations.py): nothing changes without the ticked box.
        if consent != Consent::Given {
            return Err(RelocationError::NotConfirmed);
        }
        let _one_change_at_a_time = self
            .applying
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let location = self.check(folder, value)?;
        if location.path == location.previous {
            return Ok(AppliedLocation {
                location,
                change: LocationChange::Unchanged,
            });
        }
        let backup = history::back_up(&self.state_directory, self.locations.user_dirs_file())?;
        self.updater.set_folder(folder, &location.path)?;
        self.verify_retained(folder, &location.path)?;
        history::record(&self.state_directory, &location, &backup)?;
        Ok(AppliedLocation {
            location,
            change: LocationChange::Changed { backup },
        })
    }

    /// Safety rule "verified, not assumed" (the re-read of `self.paths()`
    /// in `folder_locations.py`): the configuration is read back instead of
    /// trusting the updater's exit status.
    fn verify_retained(&self, folder: KnownFolder, path: &Path) -> Result<(), RelocationError> {
        let configured = self.locations.read_paths();
        if configured.path(folder) == path {
            Ok(())
        } else {
            Err(RelocationError::NotRetained)
        }
    }
}
