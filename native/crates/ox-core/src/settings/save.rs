// SPDX-License-Identifier: AGPL-3.0-only
//! The settings lock and the atomic, private replace of `settings.json`.
//!
//! Ports the `flock` of `settings_mutation` and the temporary-file-and-
//! rename of `Settings.save` in `v2.0.0:desktop/core.py`, built on the checks and
//! the atomic replace in `crate::private_storage`, whose [`StorageError`]
//! every step here returns. Keeping an unreadable file as a backup
//! ([`OldFile::KeepAsBackup`]) goes beyond the Python app.

use std::fs::{self, File};
use std::io;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::private_storage::{
    parent_directory, private_directory, private_file, private_file_if_present, replace_file_with,
    reserve_unique_name, PrivateFileOptions, StorageError, WithPath,
};

/// An exclusive `flock` on `settings.lock`, released when dropped.
#[derive(Debug)]
pub(super) struct SettingsLock {
    _locked_file: File,
}

impl SettingsLock {
    /// Name of the lock file inside the settings directory.
    pub(super) const FILE_NAME: &'static str = "settings.lock";

    /// Makes `directory` private and blocks until the lock is held.
    ///
    /// # Errors
    ///
    /// Everything [`private_directory`] and [`private_file`] refuse for the
    /// directory and the lock file (a symlinked `settings.lock` fails with
    /// [`StorageError::Io`]), and a failing `flock`.
    pub(super) fn acquire(directory: &Path) -> Result<Self, StorageError> {
        private_directory(directory)?;
        let path = directory.join(Self::FILE_NAME);
        let options = PrivateFileOptions {
            create: true,
            writable: true,
            allow_unlinked: false,
        };
        let file = private_file(&path, options)?;
        // `File::lock` is `flock(fd, LOCK_EX)` on Linux: the lock Python's
        // `fcntl.flock` takes in `settings_mutation` (core.py), so a Rust
        // change and a Python change never interleave. The interop tests
        // check both directions.
        file.lock().with_path(&path)?;
        Ok(Self { _locked_file: file })
    }
}

/// What [`replace_private_file`] does with the file it replaces.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum OldFile {
    /// Let the new file replace it.
    Discard,
    /// Rename it to `<name>.unreadable-<unix seconds>-<random>` first.
    KeepAsBackup,
}

/// Atomically replaces `target` with `contents`: a private temporary file
/// named `<prefix><random>` in the same directory is written, flushed to
/// disk and renamed over the target ([`replace_file_with`]), so readers
/// see either the old or the new file, never a mix. Returns where the old
/// file was kept, if it was.
///
/// Safety rule "never write through a link" (`Settings.save` in core.py):
/// an existing target must itself be a private file, so a symlinked or
/// hard-linked target is refused rather than replaced.
///
/// # Errors
///
/// Everything [`private_directory`] and [`private_file`] refuse for the
/// directory and an existing target, and [`StorageError::Io`] if writing,
/// keeping the old file or the final rename fails. On any error the
/// temporary file is removed and `target` is unchanged.
pub(super) fn replace_private_file(
    target: &Path,
    prefix: &str,
    contents: &[u8],
    old_file: OldFile,
) -> Result<Option<PathBuf>, StorageError> {
    private_directory(parent_directory(target))?;
    // Safety rule "never write through a link": a symlinked or hard-linked
    // target is refused here, before anything is written. Only the check
    // matters; the opened file is closed at once.
    private_file_if_present(target, PrivateFileOptions::default())?;
    replace_file_with(target, prefix, contents, |temporary| {
        keep_old_file_and_publish(temporary, target, old_file)
    })
}

/// Keeps the old target if asked, then renames the written `temporary`
/// file over `target`. The old file is moved aside only once the new
/// contents are on disk.
fn keep_old_file_and_publish(
    temporary: &Path,
    target: &Path,
    old_file: OldFile,
) -> Result<Option<PathBuf>, StorageError> {
    let backup = match old_file {
        OldFile::KeepAsBackup => move_aside(target)?,
        OldFile::Discard => None,
    };
    publish(temporary, target, backup.as_deref())?;
    Ok(backup)
}

/// Renames the written `temporary` file over `target`.
///
/// Safety rule "the settings file is never left missing": if the rename
/// fails after the old file was moved to `backup`, the old file is renamed
/// back to `target`.
fn publish(temporary: &Path, target: &Path, backup: Option<&Path>) -> Result<(), StorageError> {
    let Err(error) = fs::rename(temporary, target) else {
        return Ok(());
    };
    if let Some(backup) = backup {
        // Safety rule "the settings file is never left missing". Best
        // effort: if this fails too, the backup still holds the old
        // contents.
        let _ = fs::rename(backup, target);
    }
    Err(StorageError::io(target, error))
}

/// Renames `path` to an unused `<name>.unreadable-<unix seconds>-<random>`
/// beside it and returns the new path, or `None` if `path` no longer
/// exists. The file keeps its contents, owner and mode.
///
/// The new name is first reserved with an empty private file, so the
/// rename can only replace that placeholder, never another file.
fn move_aside(path: &Path) -> Result<Option<PathBuf>, StorageError> {
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    let prefix = format!("{name}.unreadable-{}-", unix_seconds());
    let backup = reserve_unique_name(parent_directory(path), &prefix)?;
    let Err(error) = fs::rename(path, &backup) else {
        return Ok(Some(backup));
    };
    // The empty placeholder is useless without the rename; removing it is
    // best effort.
    let _ = fs::remove_file(&backup);
    if error.kind() == io::ErrorKind::NotFound {
        Ok(None)
    } else {
        Err(StorageError::io(path, error))
    }
}

/// Seconds since the Unix epoch; 0 if the clock is set before it.
fn unix_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs())
}

#[cfg(test)]
mod tests {
    use std::fs::Permissions;
    use std::os::unix::fs::{symlink, PermissionsExt};

    use super::*;
    use crate::private_storage::FILE_MODE;
    use crate::test_support::permission_bits;

    /// parity: SET-012, SAFE-009
    #[test]
    fn replace_writes_a_private_file_and_leaves_no_temporary_file() {
        let root = tempfile::tempdir().unwrap();
        let target = root.path().join("settings.json");

        replace_private_file(&target, ".settings-", b"one", OldFile::Discard).unwrap();
        replace_private_file(&target, ".settings-", b"two", OldFile::Discard).unwrap();

        assert_eq!(fs::read(&target).unwrap(), b"two");
        assert_eq!(permission_bits(&target), 0o600);
        let leftovers = fs::read_dir(root.path()).unwrap().count();
        assert_eq!(leftovers, 1, "no temporary files remain");
    }

    /// parity: SET-012, SAFE-009
    #[test]
    fn replace_refuses_a_symlinked_target_and_leaves_its_target_unchanged() {
        let root = tempfile::tempdir().unwrap();
        let other = root.path().join("other");
        fs::write(&other, "{}").unwrap();
        let link = root.path().join("linked.json");
        symlink(&other, &link).unwrap();

        let replaced = replace_private_file(&link, ".settings-", b"x", OldFile::KeepAsBackup);

        assert!(replaced.is_err());
        assert!(link.is_symlink());
        assert_eq!(fs::read_to_string(&other).unwrap(), "{}");
    }

    /// parity: SET-013
    #[test]
    fn a_kept_old_file_is_renamed_beside_the_new_one() {
        let root = tempfile::tempdir().unwrap();
        let target = root.path().join("settings.json");
        fs::write(&target, "{bad").unwrap();
        fs::set_permissions(&target, Permissions::from_mode(FILE_MODE)).unwrap();

        let backup = replace_private_file(&target, ".settings-", b"{}", OldFile::KeepAsBackup)
            .unwrap()
            .expect("the old file existed");

        assert_eq!(fs::read_to_string(&target).unwrap(), "{}");
        assert_eq!(backup.parent(), Some(root.path()));
        let name = backup.file_name().unwrap().to_string_lossy();
        assert!(name.starts_with("settings.json.unreadable-"), "{name}");
        assert_eq!(fs::read_to_string(&backup).unwrap(), "{bad");
        assert_eq!(permission_bits(&backup), 0o600);
        assert_eq!(fs::read_dir(root.path()).unwrap().count(), 2);
    }

    #[test]
    fn keeping_a_missing_old_file_leaves_no_placeholder() {
        let root = tempfile::tempdir().unwrap();
        let target = root.path().join("settings.json");
        let backup = replace_private_file(&target, ".settings-", b"{}", OldFile::KeepAsBackup).unwrap();
        assert_eq!(backup, None);
        assert_eq!(fs::read_dir(root.path()).unwrap().count(), 1);
    }

    /// Safety rule "the settings file is never left missing".
    /// parity: SET-012
    #[test]
    fn a_failed_publish_puts_the_kept_file_back() {
        let root = tempfile::tempdir().unwrap();
        let target = root.path().join("settings.json");
        let backup = root.path().join("settings.json.unreadable-1-kept");
        fs::write(&backup, "{bad").unwrap();
        let missing_temporary = root.path().join(".settings-missing");

        let published = publish(&missing_temporary, &target, Some(&backup));

        assert!(published.is_err());
        assert_eq!(fs::read_to_string(&target).unwrap(), "{bad");
        assert!(!backup.exists());
    }

    // Safety rule "a failed save leaves no temporary file" is enforced and
    // tested once, in `private_storage::replace`
    // (`a_failed_save_removes_its_temporary_file`).
}
