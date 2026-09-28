// SPDX-License-Identifier: AGPL-3.0-only
//! The atomic replace of a private file: the new contents are written to a
//! private temporary file beside the target, flushed to disk and renamed
//! over it, so a reader sees the old or the new file, never a mix.
//!
//! Ports the `tempfile.mkstemp`, `os.fchmod`, `os.fsync` and `os.replace`
//! sequence that `Settings.save` in `desktop/core.py` and
//! `PreviousVersions.configure` in `desktop/previous_versions.py` share.
//! Whether an existing target may be replaced is the caller's decision:
//! the settings refuse a linked `settings.json` first, while the snapshot
//! sources replace a link as `os.replace` does.

use std::fs::{self, File, OpenOptions, Permissions};
use std::io::Write;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

use super::{StorageError, WithPath, FILE_MODE};
use crate::random::{random_hex, NAME_BYTES};

/// Atomically replaces `target` with `contents`: a private temporary file
/// named `<prefix><random>` beside it is written, flushed to disk and
/// renamed over `target`.
///
/// # Errors
///
/// [`StorageError::Io`] when the temporary file cannot be created, written
/// or flushed, or the rename fails. On any error the temporary file is
/// removed and `target` is unchanged.
pub(crate) fn replace_file_atomically(
    target: &Path,
    prefix: &str,
    contents: &[u8],
) -> Result<(), StorageError> {
    replace_file_with(target, prefix, contents, |written| {
        fs::rename(written, target).with_path(target)
    })
}

/// [`replace_file_atomically`] for a caller that puts the new file in
/// place itself: `publish` receives the path of the written and flushed
/// temporary file and renames it over `target`, for example after moving
/// the old file aside. Returns what `publish` returns.
///
/// # Errors
///
/// As [`replace_file_atomically`], and the error of `publish`.
pub(crate) fn replace_file_with<T>(
    target: &Path,
    prefix: &str,
    contents: &[u8],
    publish: impl FnOnce(&Path) -> Result<T, StorageError>,
) -> Result<T, StorageError> {
    let directory = parent_directory(target);
    let mut temporary = UniqueFile::create(directory, prefix)?;
    let published = temporary
        .write_private(contents)
        .and_then(|()| publish(&temporary.path));
    if published.is_err() {
        // Safety rule "a failed save leaves no temporary file" (the
        // `finally` of `configure` in previous_versions.py, and
        // `Settings.save` in core.py): the new file never took the
        // target's place, so the copy is useless. Removing it is best
        // effort; a leftover is private and named with the prefix.
        let _ = fs::remove_file(&temporary.path);
        return published;
    }
    sync_directory(directory);
    published
}

/// Creates an empty private file named `<prefix><random>` in `directory`
/// and returns its path, so that a later rename to that path can only
/// replace this placeholder, never another file.
///
/// # Errors
///
/// [`StorageError::Io`] when the file cannot be created.
pub(crate) fn reserve_unique_name(directory: &Path, prefix: &str) -> Result<PathBuf, StorageError> {
    let placeholder = UniqueFile::create(directory, prefix)?;
    Ok(placeholder.path)
}

/// The directory containing `path`; the current directory for a bare name.
pub(crate) fn parent_directory(path: &Path) -> &Path {
    path.parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or(Path::new("."))
}

/// A newly created private file with a name no other file had, like the
/// pair Python's `tempfile.mkstemp` returns.
#[derive(Debug)]
struct UniqueFile {
    file: File,
    path: PathBuf,
}

impl UniqueFile {
    /// Creates `<prefix><32 random hex digits>` in `directory` with mode
    /// 0600.
    /// `create_new` (`O_CREAT | O_EXCL`) never opens an existing file or
    /// follows a symlink, so a taken name fails instead of being
    /// overwritten, and the cleanup above never removes another file.
    fn create(directory: &Path, prefix: &str) -> Result<Self, StorageError> {
        let digits = random_hex(NAME_BYTES).with_path(directory)?;
        let path = directory.join(format!("{prefix}{digits}"));
        let file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(FILE_MODE)
            .open(&path)
            .with_path(&path)?;
        Ok(Self { file, path })
    }

    /// Writes `contents` and flushes them to disk. The explicit `fchmod`
    /// makes the mode exactly 0600 whatever the umask, as Python's
    /// `os.fchmod` does.
    fn write_private(&mut self, contents: &[u8]) -> Result<(), StorageError> {
        self.file
            .set_permissions(Permissions::from_mode(FILE_MODE))
            .with_path(&self.path)?;
        self.file.write_all(contents).with_path(&self.path)?;
        self.file.sync_all().with_path(&self.path)
    }
}

/// Makes a rename in `directory` durable. A failure here does not undo the
/// rename, so it is ignored.
fn sync_directory(directory: &Path) {
    if let Ok(handle) = File::open(directory) {
        let _ = handle.sync_all();
    }
}

#[cfg(test)]
mod tests {
    use std::io;
    use std::os::unix::fs::symlink;

    use super::*;
    use crate::test_support::mode;

    /// The number of entries in `directory`.
    fn entry_count(directory: &Path) -> usize {
        fs::read_dir(directory).expect("list the directory").count()
    }

    /// parity: SET-012, PROP-023
    #[test]
    fn a_replace_writes_a_private_file_and_leaves_no_temporary_file() {
        let root = tempfile::tempdir().unwrap();
        let target = root.path().join("snapshot-sources.json");
        fs::write(&target, "old").unwrap();

        replace_file_atomically(&target, ".versions-", b"new").unwrap();

        assert_eq!(fs::read(&target).unwrap(), b"new");
        assert_eq!(mode(&target), 0o600);
        assert_eq!(entry_count(root.path()), 1, "no temporary file remains");
    }

    /// Like `os.replace`, a symlinked target is replaced by the new file,
    /// and the file it pointed to is never written.
    #[test]
    fn a_symlinked_target_is_replaced_not_written_through() {
        let root = tempfile::tempdir().unwrap();
        let other = root.path().join("other");
        fs::write(&other, "unchanged").unwrap();
        let link = root.path().join("linked.json");
        symlink(&other, &link).unwrap();

        replace_file_atomically(&link, ".versions-", b"new").unwrap();

        assert!(!link.is_symlink());
        assert_eq!(fs::read(&link).unwrap(), b"new");
        assert_eq!(fs::read_to_string(&other).unwrap(), "unchanged");
    }

    /// Safety rule "a failed save leaves no temporary file".
    /// parity: SET-012
    #[test]
    fn a_failed_save_removes_its_temporary_file() {
        let root = tempfile::tempdir().unwrap();
        let target = root.path().join("settings.json");
        let refusal = StorageError::io(&target, io::Error::other("the rename is refused"));

        let saved = replace_file_with(&target, ".settings-", b"{}", |_| Err::<(), _>(refusal));

        assert!(saved.is_err());
        assert_eq!(entry_count(root.path()), 0, "no temporary file remains");
    }

    #[test]
    fn a_reserved_name_is_an_empty_private_file() {
        let root = tempfile::tempdir().unwrap();

        let reserved = reserve_unique_name(root.path(), "settings.json.unreadable-").unwrap();

        let name = reserved.file_name().unwrap().to_string_lossy();
        assert!(name.starts_with("settings.json.unreadable-"), "{name}");
        assert_eq!(fs::read(&reserved).unwrap(), b"");
        assert_eq!(mode(&reserved), 0o600);
    }
}
