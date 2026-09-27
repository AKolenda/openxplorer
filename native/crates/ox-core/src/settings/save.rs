// SPDX-License-Identifier: AGPL-3.0-only
//! The settings lock and the atomic, private replace of `settings.json`.
//!
//! Ports the `flock` of `settings_mutation` and the temporary-file-and-
//! rename of `Settings.save` in `desktop/core.py`, built on the private
//! storage checks in the `storage` module. Keeping a damaged file as a
//! backup ([`OldFile::KeepAsBackup`]) goes beyond the Python app.

use std::fs::{self, File, OpenOptions, Permissions};
use std::io::{self, Write};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use super::error::WithPath;
use super::storage::{private_directory, private_file, PrivateFileOptions, FILE_MODE};
use super::SettingsError;

/// An exclusive `flock` on `settings.lock`, released when dropped.
#[derive(Debug)]
pub(crate) struct SettingsLock {
    _locked_file: File,
}

impl SettingsLock {
    /// Name of the lock file inside the settings directory.
    pub(crate) const FILE_NAME: &'static str = "settings.lock";

    /// Makes `directory` private and blocks until the lock is held.
    pub(crate) fn acquire(directory: &Path) -> Result<Self, SettingsError> {
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
pub(crate) enum OldFile {
    /// Let the new file replace it.
    Discard,
    /// Rename it to `<name>.unreadable-<unix seconds>-<random>` first.
    KeepAsBackup,
}

/// Atomically replaces `target` with `contents`: a private temporary file
/// in the same directory is written, flushed to disk and renamed over the
/// target, so readers see either the old or the new file, never a mix.
/// Returns where the old file was kept, if it was.
///
/// Safety rule "never write through a link" (`Settings.save` in core.py):
/// an existing target must itself be a private file, so a symlinked or
/// hard-linked target is refused rather than replaced.
pub(crate) fn replace_private_file(
    target: &Path,
    prefix: &str,
    contents: &[u8],
    old_file: OldFile,
) -> Result<Option<PathBuf>, SettingsError> {
    let directory = parent_directory(target);
    private_directory(directory)?;
    match private_file(target, PrivateFileOptions::default()) {
        Err(error) if !error.is_not_found() => return Err(error),
        _ => {}
    }
    let (mut file, temporary) = create_unique_file(directory, prefix)?;
    let published = write_and_publish(&mut file, &temporary, target, contents, old_file);
    if published.is_err() {
        // The rename did not happen; do not leave the partial copy behind.
        let _ = fs::remove_file(&temporary);
    }
    let backup = published?;
    sync_directory(directory);
    Ok(backup)
}

/// Writes and flushes the temporary file, keeps the old target if asked,
/// and renames the temporary file over the target. The explicit `fchmod`
/// makes the mode exactly 0600 whatever the umask.
///
/// The old file is moved aside only once the new contents are on disk, and
/// moved back if the final rename fails, so `target` is never left missing.
fn write_and_publish(
    file: &mut File,
    temporary: &Path,
    target: &Path,
    contents: &[u8],
    old_file: OldFile,
) -> Result<Option<PathBuf>, SettingsError> {
    file.set_permissions(Permissions::from_mode(FILE_MODE))
        .with_path(temporary)?;
    file.write_all(contents).with_path(temporary)?;
    file.sync_all().with_path(temporary)?;
    let backup = match old_file {
        OldFile::KeepAsBackup => move_aside(target)?,
        OldFile::Discard => None,
    };
    if let Err(error) = fs::rename(temporary, target) {
        if let Some(backup) = &backup {
            // Best effort: if this fails too, the backup still holds it.
            let _ = fs::rename(backup, target);
        }
        return Err(SettingsError::io(target, error));
    }
    Ok(backup)
}

/// Renames `path` to an unused `<name>.unreadable-<unix seconds>-<random>`
/// beside it and returns the new path, or `None` if `path` no longer
/// exists. The file keeps its contents, owner and mode.
///
/// The new name is first reserved with an empty private file, so the
/// rename can only replace that placeholder, never another file.
fn move_aside(path: &Path) -> Result<Option<PathBuf>, SettingsError> {
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    let prefix = format!("{name}.unreadable-{}-", unix_seconds());
    let (placeholder, backup) = create_unique_file(parent_directory(path), &prefix)?;
    drop(placeholder);
    let renamed = fs::rename(path, &backup);
    if renamed.is_err() {
        let _ = fs::remove_file(&backup);
    }
    match renamed {
        Ok(()) => Ok(Some(backup)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(SettingsError::io(path, error)),
    }
}

/// Creates a new private file named `<prefix><random UUID>` in
/// `directory`, like Python's `tempfile.mkstemp`. `create_new` (`O_CREAT |
/// O_EXCL`) never opens an existing file or follows a symlink, so a taken
/// name fails instead of being overwritten.
fn create_unique_file(directory: &Path, prefix: &str) -> Result<(File, PathBuf), SettingsError> {
    let path = directory.join(format!("{prefix}{}", glib::uuid_string_random()));
    let file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(FILE_MODE)
        .open(&path)
        .with_path(&path)?;
    Ok((file, path))
}

/// Makes a rename in `directory` durable. A failure here does not undo the
/// rename, so it is ignored.
fn sync_directory(directory: &Path) {
    if let Ok(handle) = File::open(directory) {
        let _ = handle.sync_all();
    }
}

/// The directory containing `path`; the current directory for a bare name.
fn parent_directory(path: &Path) -> &Path {
    path.parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or(Path::new("."))
}

/// Seconds since the Unix epoch; 0 if the clock is set before it.
fn unix_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs())
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::{symlink, MetadataExt};

    use super::*;

    fn mode(path: &Path) -> u32 {
        fs::metadata(path).unwrap().mode() & 0o777
    }

    /// parity: SET-012, SAFE-009
    #[test]
    fn replace_writes_a_private_file_and_refuses_symlinks() {
        let root = tempfile::tempdir().unwrap();
        let target = root.path().join("settings.json");
        replace_private_file(&target, ".settings-", b"one", OldFile::Discard).unwrap();
        replace_private_file(&target, ".settings-", b"two", OldFile::Discard).unwrap();
        assert_eq!(fs::read(&target).unwrap(), b"two");
        assert_eq!(mode(&target), 0o600);
        let leftovers = fs::read_dir(root.path()).unwrap().count();
        assert_eq!(leftovers, 1, "no temporary files remain");

        let other = root.path().join("other");
        fs::write(&other, "{}").unwrap();
        let link = root.path().join("linked.json");
        symlink(&other, &link).unwrap();
        let replaced = replace_private_file(&link, ".settings-", b"x", OldFile::KeepAsBackup);
        assert!(replaced.is_err());
        assert!(link.is_symlink());
        assert_eq!(fs::read_to_string(&other).unwrap(), "{}");
    }

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
        assert_eq!(mode(&backup), 0o600);
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
}
