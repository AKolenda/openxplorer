// SPDX-License-Identifier: AGPL-3.0-only
//! The administrator mount helper, `openxplorer-mount-share`.
//!
//! Ports `v2.0.0:desktop/mount_share.py`, the program that `sudo` runs from the
//! plan's command ([`MountPlan`](super::MountPlan)) in the user's own
//! terminal. The GUI never runs it and never gains privileges. It prints
//! the plan, asks for `SETUP` (or `REMOVE`) and the SMB account, writes a
//! root-only credential file and two systemd units without replacing
//! anything, and rolls back on failure. It never edits fstab and never
//! deletes shared or local files.
//!
//! | Module | Responsibility |
//! |---|---|
//! | this file | The file-safety rules and the credential file |
//! | `arguments` | The command line, as `argparse` read it |
//! | `host` | The computer the helper changes: files, systemd, the desktop account |
//! | `terminal` | Questions, answers and a password typed without echo |
//! | `setup` | Setting up, removing and printing a managed mount |

use std::fs::{self, DirBuilder, OpenOptions};
use std::io::{self, Write};
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt};
use std::path::{Component, Path, PathBuf};

use rustix::fs::OFlags;

use super::MountPlanError;
use crate::location::python_strip;
use crate::private_storage::KernelOpenFlags;

mod arguments;
mod host;
mod setup;
mod terminal;
#[cfg(test)]
mod tests;

use arguments::{Request, HELP, USAGE};
use host::{Host, SystemSystemd};
use terminal::ConsoleTerminal;

/// Permission bits that let the group or others write.
const GROUP_OR_OTHER_WRITE: u32 = 0o022;
/// The mode parent directories are created with.
const PARENT_MODE: u32 = 0o755;
/// The user ID of root, who owns every file the helper manages.
const ROOT_UID: u32 = 0;

/// Why the helper stopped, in the words `mount_share.py` printed after
/// `Mount setup: `.
#[derive(Debug, thiserror::Error)]
pub(crate) enum MountHelperError {
    /// A relative path, or one with `..`.
    #[error(
        "{}",
        crate::i18n::gettext("Administrative paths must be absolute, without parent traversal.")
    )]
    NotAbsolute,
    /// A directory in the chain is a symlink, not owned by root, or
    /// writable by the group or others.
    #[error("{}", crate::i18n::format_message("Refusing unsafe directory: {path}", &[("path", &.0.display().to_string())]))]
    UnsafeDirectory(PathBuf),
    /// A user name or password with a line break or NUL, or no user name.
    #[error(
        "{}",
        crate::i18n::gettext("Invalid credentials. Newlines and NUL characters are not supported.")
    )]
    InvalidCredentials,
    /// `DOMAIN\` without a user name.
    #[error("{}", crate::i18n::gettext("Enter a username after the domain."))]
    MissingUsername,
    /// Run from a root login, or `SUDO_UID` names root.
    #[error(
        "{}",
        crate::i18n::gettext("Run with sudo from your normal desktop account, not a root login.")
    )]
    RootLogin,
    /// The desktop account is not in the user database.
    #[error("{}", crate::i18n::format_message("No account has the user ID {user_id}.", &[("user_id", .0.as_str())]))]
    UnknownAccount(String),
    /// The share cannot be planned.
    #[error(transparent)]
    Plan(#[from] MountPlanError),
    /// A change was asked for without `sudo`.
    #[error(
        "{}",
        crate::i18n::gettext("This operation requires sudo. The GUI itself must stay unprivileged.")
    )]
    NotAdministrator,
    /// Standard input is not a terminal, so nobody can review the change.
    #[error(
        "{}",
        crate::i18n::gettext("Run in a terminal so you can review and confirm the change.")
    )]
    NotATerminal,
    /// A managed file to remove is missing, a symlink or not root's.
    #[error(
        "{}",
        crate::i18n::gettext("Missing or unsafe managed files; refusing automatic removal.")
    )]
    UnsafeManagedFiles,
    /// A managed unit no longer has the text the helper wrote.
    #[error(
        "{}",
        crate::i18n::gettext("Unit files have been edited. Remove them manually after review.")
    )]
    EditedUnits,
    /// `mount.cifs` is not installed.
    #[error(
        "{}",
        crate::i18n::gettext("Install the cifs-utils package first, then run this command again.")
    )]
    MissingCifsUtils,
    /// A unit, the credential file or the mount point already exists.
    #[error("{}", crate::i18n::format_message("This mount already exists or a path is in use. Nothing was overwritten.\nRemoval command: {command}", &[("command", .0.as_str())]))]
    AlreadyExists(String),
    /// `systemctl` failed or took too long.
    #[error("{}", crate::i18n::format_message("systemctl {command} {reason}", &[("command", command), ("reason", reason)]))]
    Systemctl {
        /// The arguments, joined with spaces.
        command: String,
        /// What went wrong, such as `failed with exit status 1`.
        reason: String,
    },
    /// The file system refused an operation on `path`.
    #[error("{error}: {}", path.display())]
    Io {
        /// The file or directory.
        path: PathBuf,
        /// What the operating system reported.
        error: io::Error,
    },
    /// Reading an answer or writing to the terminal failed.
    #[error("{0}")]
    Terminal(#[source] io::Error),
}

impl MountHelperError {
    fn io(path: &Path, error: io::Error) -> Self {
        Self::Io {
            path: path.to_path_buf(),
            error,
        }
    }
}

/// The folders the helper may write in: everything below `root`, where
/// every directory must belong to `owner`.
///
/// On a real system this is `/` and root; tests use a temporary folder
/// and their own user.
#[derive(Debug, Clone)]
pub(crate) struct AdministrativeTree {
    root: PathBuf,
    owner: u32,
}

impl AdministrativeTree {
    /// The whole file system, owned by root.
    pub(crate) fn system() -> Self {
        Self {
            root: PathBuf::from("/"),
            owner: ROOT_UID,
        }
    }

    /// Where the absolute system path `path` is inside this tree.
    pub(crate) fn path(&self, path: &Path) -> PathBuf {
        self.root.join(path.strip_prefix("/").unwrap_or(path))
    }

    /// The user every administrative folder and managed file belongs to.
    pub(crate) fn owner(&self) -> u32 {
        self.owner
    }

    /// Makes sure `path` and every directory above it, up to the tree's
    /// root, are real directories of the owner that only the owner can
    /// write, creating a missing one with `mode` (its missing parents with
    /// 0755).
    ///
    /// Safety rule (NET-028): the helper writes root-only files only where
    /// no other user can swap a directory for a symlink, so the whole
    /// existing parent chain is checked, not just the last directory.
    ///
    /// # Errors
    ///
    /// [`MountHelperError::NotAbsolute`], [`MountHelperError::UnsafeDirectory`]
    /// naming the first unsafe directory, or the I/O error of creating one.
    pub(crate) fn secure_directory(&self, path: &Path, mode: u32) -> Result<(), MountHelperError> {
        let has_parent_traversal = path
            .components()
            .any(|component| component == Component::ParentDir);
        if !path.is_absolute() || has_parent_traversal {
            return Err(MountHelperError::NotAbsolute);
        }
        if path != self.root {
            if let Some(parent) = path.parent() {
                self.secure_directory(parent, PARENT_MODE)?;
            }
        }
        let metadata = match fs::symlink_metadata(path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                DirBuilder::new()
                    .mode(mode)
                    .create(path)
                    .map_err(|error| MountHelperError::io(path, error))?;
                fs::symlink_metadata(path).map_err(|error| MountHelperError::io(path, error))?
            }
            Err(error) => return Err(MountHelperError::io(path, error)),
        };
        let is_owner_only =
            metadata.is_dir() && metadata.uid() == self.owner && metadata.mode() & GROUP_OR_OTHER_WRITE == 0;
        if !is_owner_only {
            return Err(MountHelperError::UnsafeDirectory(path.to_path_buf()));
        }
        Ok(())
    }
}

/// Writes `text` to a new file at `path` with `mode` and flushes it to
/// disk.
///
/// Safety rule (NET-028): an existing file or symlink is never
/// overwritten or followed; the helper refuses instead.
///
/// # Errors
///
/// The I/O error, including `AlreadyExists` for an existing path and
/// `ELOOP` for a symlink.
pub(crate) fn write_new_file(path: &Path, text: &str, mode: u32) -> Result<(), MountHelperError> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .kernel_flags(OFlags::NOFOLLOW)
        .mode(mode)
        .open(path)
        .map_err(|error| MountHelperError::io(path, error))?;
    file.write_all(text.as_bytes())
        .and_then(|()| file.sync_all())
        .map_err(|error| MountHelperError::io(path, error))
}

/// The text of the `mount.cifs` credential file for the user name typed in
/// the helper's terminal (`DOMAIN\user` allowed) and `password`.
///
/// # Errors
///
/// [`MountHelperError::InvalidCredentials`] for an empty user name or a
/// line break or NUL in either, and [`MountHelperError::MissingUsername`]
/// for `DOMAIN\` alone.
pub(crate) fn credential_file_text(typed_username: &str, password: &str) -> Result<String, MountHelperError> {
    let username = python_strip(typed_username);
    let is_line_safe = |text: &str| !text.contains(['\r', '\n', '\0']);
    if username.is_empty() || !is_line_safe(username) || !is_line_safe(password) {
        return Err(MountHelperError::InvalidCredentials);
    }
    let (domain, username) = username.split_once('\\').unwrap_or(("", username));
    if username.is_empty() {
        return Err(MountHelperError::MissingUsername);
    }
    let domain_line = if domain.is_empty() {
        String::new()
    } else {
        format!("domain={domain}\n")
    };
    Ok(format!("username={username}\npassword={password}\n{domain_line}"))
}

/// Runs `openxplorer-mount-share` with the command-line `arguments`
/// (without the program name) on this computer and returns its exit
/// status: 0 when done, 1 when cancelled or refused, 2 for a usage error.
pub fn mount_share_command(arguments: impl IntoIterator<Item = String>) -> u8 {
    let request = match arguments::parse(arguments) {
        Ok(request) => request,
        Err(error) => {
            eprintln!("{USAGE}\nopenxplorer-mount-share: error: {error}");
            return 2;
        }
    };
    let arguments = match request {
        Request::Help => {
            println!("{HELP}");
            return 0;
        }
        Request::Run(arguments) => arguments,
    };
    let mut systemd = SystemSystemd;
    let result = host::invoking_account().and_then(|account| {
        let mut host = Host::system(&mut systemd);
        setup::run(&arguments, &account, &mut host, &mut ConsoleTerminal)
    });
    match result {
        Ok(outcome) => outcome.exit_status(),
        Err(error) => {
            eprintln!("Mount setup: {error}");
            1
        }
    }
}
