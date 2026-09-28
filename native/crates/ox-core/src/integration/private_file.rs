// SPDX-License-Identifier: AGPL-3.0-only
//! Replacing a small file atomically with a private (0600) one.
//!
//! Ports `DesktopIntegration._save` in `desktop/desktop_integration.py`,
//! `atomic_text` in `desktop/reveal_integration.py` and `atomic_bytes` in
//! `desktop/brave_integration.py`, which are the same steps: write a
//! temporary file beside the target, make it 0600, flush it to disk, then
//! rename it over the target.

use std::fs::{self, File, OpenOptions, Permissions};
use std::io::{self, Write};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

use crate::private_storage::FILE_MODE;
use crate::random::{random_hex, NAME_BYTES};

/// Replaces `path` with a private file holding `contents`.
///
/// Safety rule "an integration file is never half written" (`_save`,
/// `atomic_text` and `atomic_bytes` in the Python modules): the new
/// contents are complete and on disk before a rename puts them in place,
/// so a crash leaves either the old file or the new one. The temporary
/// file is created exclusively and never follows a symlink, and it is
/// removed if any step fails.
///
/// `prefix` starts the temporary file's name, as it did in Python
/// (`.defaults-` or `.winspace-`), so a leftover file shows who made it.
///
/// # Errors
///
/// The I/O error of creating, writing, syncing or renaming the file.
pub(crate) fn write_private_file(path: &Path, prefix: &str, contents: &[u8]) -> io::Result<()> {
    let temporary = temporary_path(path, prefix)?;
    let written = write_and_rename(&temporary, path, contents);
    if written.is_err() {
        // The temporary file may not exist if creating it failed.
        let _ = fs::remove_file(&temporary);
    }
    written
}

/// A new name beside `path` that starts with `prefix`.
fn temporary_path(path: &Path, prefix: &str) -> io::Result<PathBuf> {
    let directory = path
        .parent()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "a file path has a parent"))?;
    let name = format!("{prefix}{}", random_hex(NAME_BYTES)?);
    Ok(directory.join(name))
}

/// Writes `contents` to a new private file at `temporary`, syncs it and
/// renames it to `target`.
fn write_and_rename(temporary: &Path, target: &Path, contents: &[u8]) -> io::Result<()> {
    let mut file = create_private(temporary)?;
    file.write_all(contents)?;
    file.sync_all()?;
    fs::rename(temporary, target)
}

/// Creates `path` exclusively with mode 0600, not following a symlink.
///
/// The mode is set again after creating because the umask can only
/// narrow the creation mode; Python's `os.fchmod(fd, 0o600)` does the
/// same.
fn create_private(path: &Path) -> io::Result<File> {
    let file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(FILE_MODE)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)?;
    file.set_permissions(Permissions::from_mode(FILE_MODE))?;
    Ok(file)
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::MetadataExt;

    use super::*;

    #[test]
    fn replacing_leaves_a_private_file_and_no_temporary_file() {
        let folder = tempfile::tempdir().expect("temporary folder");
        let target = folder.path().join("previous-defaults.json");
        fs::write(&target, "old").expect("fixture");

        write_private_file(&target, ".defaults-", b"{}").expect("write");

        assert_eq!(fs::read(&target).expect("read"), b"{}");
        assert_eq!(fs::metadata(&target).expect("stat").mode() & 0o777, 0o600);
        let names: Vec<_> = fs::read_dir(folder.path())
            .expect("list")
            .map(|entry| entry.expect("entry").file_name())
            .collect();
        assert_eq!(names, ["previous-defaults.json"]);
    }

    #[test]
    fn a_failed_write_removes_its_temporary_file() {
        let folder = tempfile::tempdir().expect("temporary folder");
        // Renaming a file over a folder fails after the temporary file was
        // written.
        let target = folder.path().join("occupied");
        fs::create_dir(&target).expect("fixture");
        fs::write(target.join("inside"), "x").expect("fixture");

        let written = write_private_file(&target, ".winspace-", b"new");

        assert!(written.is_err());
        let names: Vec<_> = fs::read_dir(folder.path())
            .expect("list")
            .map(|entry| entry.expect("entry").file_name())
            .collect();
        assert_eq!(names, ["occupied"]);
    }
}
