// SPDX-License-Identifier: AGPL-3.0-only
//! The file-safety rules of the administrator mount helper.
//!
//! Ports `secure_dir`, `exclusive_write` and the credential-file text of
//! `desktop/mount_share.py`, the helper that `sudo` runs from the plan's
//! command ([`MountPlan`](super::MountPlan)). The GUI never runs it and
//! never gains privileges.
//!
//! The helper's interactive command line (`main` in `mount_share.py`:
//! review, `SETUP`/`REMOVE` confirmation, `systemctl` and rollback) is a
//! separate `sudo` program, ported with the rest of the network service
//! (ROADMAP.md, "Network and devices"). Until then the packaged
//! `openxplorer-mount-share` keeps running `mount_share.py`, which needs
//! only `mount_support.py` and the standard library, and these rules are
//! crate-private and called only by their tests.

use std::fs::{self, DirBuilder, OpenOptions};
use std::io::{self, Write};
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt};
use std::path::{Component, Path, PathBuf};

use rustix::fs::OFlags;

use crate::location::python_strip;
use crate::private_storage::KernelOpenFlags;

/// Permission bits that let the group or others write.
const GROUP_OR_OTHER_WRITE: u32 = 0o022;
/// The mode parent directories are created with.
const PARENT_MODE: u32 = 0o755;

/// Why the helper refused a path or the credentials typed in its terminal.
#[derive(Debug, thiserror::Error)]
pub(crate) enum MountHelperError {
    /// A relative path, or one with `..`.
    #[error("Administrative paths must be absolute, without parent traversal.")]
    NotAbsolute,
    /// A directory in the chain is a symlink, not owned by root, or
    /// writable by the group or others.
    #[error("Refusing unsafe directory: {}", .0.display())]
    UnsafeDirectory(PathBuf),
    /// A user name or password with a line break or NUL, or no user name.
    #[error("Invalid credentials. Newlines and NUL characters are not supported.")]
    InvalidCredentials,
    /// `DOMAIN\` without a user name.
    #[error("Enter a username after the domain.")]
    MissingUsername,
    /// The file system refused an operation on `path`.
    #[error("{error}: {}", path.display())]
    Io {
        /// The file or directory.
        path: PathBuf,
        /// What the operating system reported.
        error: io::Error,
    },
}

impl MountHelperError {
    fn io(path: &Path, error: io::Error) -> Self {
        Self::Io {
            path: path.to_path_buf(),
            error,
        }
    }
}

/// Makes sure `path` and every directory above it are root-owned, real
/// directories that only root can write, creating a missing one with
/// `mode` (its missing parents with 0755).
///
/// Safety rule (NET-028): the helper writes root-only files only where no
/// other user can swap a directory for a symlink, so the whole existing
/// parent chain is checked, not just the last directory.
///
/// # Errors
///
/// [`MountHelperError::NotAbsolute`], [`MountHelperError::UnsafeDirectory`]
/// naming the first unsafe directory, or the I/O error of creating one.
pub(crate) fn secure_directory(path: &Path, mode: u32) -> Result<(), MountHelperError> {
    let has_parent_traversal = path
        .components()
        .any(|component| component == Component::ParentDir);
    if !path.is_absolute() || has_parent_traversal {
        return Err(MountHelperError::NotAbsolute);
    }
    if let Some(parent) = path.parent() {
        secure_directory(parent, PARENT_MODE)?;
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
    let is_root_only =
        metadata.is_dir() && metadata.uid() == 0 && metadata.mode() & GROUP_OR_OTHER_WRITE == 0;
    if !is_root_only {
        return Err(MountHelperError::UnsafeDirectory(path.to_path_buf()));
    }
    Ok(())
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

#[cfg(test)]
mod tests {
    use std::os::unix::fs::{symlink, PermissionsExt};

    use super::*;

    /// Ported from `desktop/tests/test_terminal_security.py::AdditionalSecurityTests::test_admin_helper_checks_existing_parent_chain`
    ///
    /// The temporary folder's parents (such as the world-writable `/tmp`,
    /// or a home owned by the user) are unsafe even though the directory
    /// itself looks private.
    ///
    /// parity: NET-028, SAFE-021
    #[test]
    fn an_unsafe_parent_chain_is_refused() {
        let root = tempfile::tempdir().expect("temporary folder");
        let owned = root.path().join("owned");
        DirBuilder::new()
            .mode(0o700)
            .create(&owned)
            .expect("private folder");

        let refused = secure_directory(&owned, 0o700);

        assert!(
            matches!(refused, Err(MountHelperError::UnsafeDirectory(_))),
            "{refused:?}"
        );
    }

    /// parity: SAFE-021
    #[test]
    fn relative_and_traversing_paths_are_refused() {
        for path in ["etc/winspace", "/etc/../tmp"] {
            let refused = secure_directory(Path::new(path), 0o755);
            assert!(
                matches!(refused, Err(MountHelperError::NotAbsolute)),
                "{path}: {refused:?}"
            );
        }
    }

    /// parity: NET-028, SAFE-021
    #[test]
    fn a_new_file_gets_its_mode_and_never_replaces_anything() {
        let root = tempfile::tempdir().expect("temporary folder");
        let credential = root.path().join("credential");

        write_new_file(&credential, "username=sam\n", 0o600).expect("a new file");

        let mode = fs::metadata(&credential).expect("written").permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
        assert!(write_new_file(&credential, "other", 0o600).is_err());
        assert_eq!(
            fs::read_to_string(&credential).expect("readable"),
            "username=sam\n"
        );
    }

    /// parity: SAFE-021
    #[test]
    fn a_symlink_is_never_followed() {
        let root = tempfile::tempdir().expect("temporary folder");
        let target = root.path().join("target");
        fs::write(&target, "unchanged").expect("target file");
        let link = root.path().join("link");
        symlink(&target, &link).expect("symlink");

        assert!(write_new_file(&link, "password=x\n", 0o600).is_err());
        assert_eq!(fs::read_to_string(&target).expect("readable"), "unchanged");
    }

    /// parity: NET-028
    #[test]
    fn the_credential_file_names_user_password_and_optional_domain() {
        assert_eq!(
            credential_file_text(" OFFICE\\sam ", "secret").expect("valid"),
            "username=sam\npassword=secret\ndomain=OFFICE\n"
        );
        assert_eq!(
            credential_file_text("sam", "secret").expect("valid"),
            "username=sam\npassword=secret\n"
        );
    }

    /// parity: NET-028
    #[test]
    fn credentials_that_would_break_the_file_are_refused() {
        let invalid = [("", "secret"), ("sam", "line\nbreak"), ("sam\0", "secret")];
        for (username, password) in invalid {
            let refused = credential_file_text(username, password);
            assert!(
                matches!(refused, Err(MountHelperError::InvalidCredentials)),
                "{username:?}"
            );
        }
        let without_user = credential_file_text("OFFICE\\", "secret");
        assert!(matches!(without_user, Err(MountHelperError::MissingUsername)));
    }
}
