// SPDX-License-Identifier: AGPL-3.0-only
//! Private application state: owned 0700 directories and 0600 files that
//! are never symlinks, hard links, FIFOs or devices.
//!
//! Ports `desktop/private_storage.py`, plus the lock and atomic replace
//! that `Settings.save` and `settings_mutation` in `desktop/core.py` use.
//! This is defence in depth for misplaced or tampered XDG state, not
//! isolation from another process running as the same user. Every open
//! uses `O_NOFOLLOW` (a symlinked leaf fails with `ELOOP`) and
//! `O_NONBLOCK` (a FIFO never blocks), and the checks run on the opened
//! descriptor before its mode is changed.

use std::fs::{self, DirBuilder, File, OpenOptions, Permissions};
use std::hash::{BuildHasher, Hasher};
use std::io::{self, Read, Write};
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use super::SettingsError;

/// Largest settings file read, in bytes.
pub const SETTINGS_SIZE_LIMIT: u64 = 4 * 1024 * 1024;

/// Mode of private directories.
const DIRECTORY_MODE: u32 = 0o700;

/// Mode of private files.
const FILE_MODE: u32 = 0o600;

/// How a private file is opened.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PrivateFileOptions {
    /// Create the file (mode 0600) if it is missing.
    pub create: bool,
    /// Open for reading and writing instead of reading only.
    pub writable: bool,
    /// Accept a file whose last directory entry was removed after opening,
    /// as SQLite does with its sidecar files. Its mode is left unchanged.
    pub allow_unlinked: bool,
}

/// Creates `path` (and missing parents) and makes it a private directory:
/// it must be a real directory owned by this user, not a symlink, and its
/// mode becomes 0700. Parents keep their default mode, as in Python.
pub fn private_directory(path: &Path) -> Result<(), SettingsError> {
    create_directory(path)?;
    let directory = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW)
        .open(path)?;
    let info = directory.metadata()?;
    if !info.is_dir() || info.uid() != effective_uid()? {
        return Err(SettingsError::invalid(
            "Application state directory must be owned by this user.",
        ));
    }
    directory.set_permissions(Permissions::from_mode(DIRECTORY_MODE))?;
    Ok(())
}

/// Opens a private file. It must be a regular file owned by this user with
/// exactly one link; its mode becomes 0600. The checks run before the mode
/// changes, so a rejected file is never modified.
pub fn private_file(path: &Path, options: PrivateFileOptions) -> Result<File, SettingsError> {
    let mut flags = libc::O_NOFOLLOW | libc::O_NONBLOCK;
    if options.create {
        flags |= libc::O_CREAT;
    }
    let file = OpenOptions::new()
        .read(true)
        .write(options.writable)
        .mode(FILE_MODE)
        .custom_flags(flags)
        .open(path)?;
    let info = file.metadata()?;
    let unlinked = info.nlink() == 0;
    let acceptable = info.file_type().is_file()
        && info.uid() == effective_uid()?
        && info.nlink() <= 1
        && (!unlinked || options.allow_unlinked);
    if !acceptable {
        return Err(SettingsError::invalid(
            "Application state must be an owned regular file, not a link or device.",
        ));
    }
    if !unlinked {
        file.set_permissions(Permissions::from_mode(FILE_MODE))?;
    }
    Ok(file)
}

/// Reads a private UTF-8 text file of at most `limit` bytes.
pub fn private_text(path: &Path, limit: u64) -> Result<String, SettingsError> {
    let file = private_file(path, PrivateFileOptions::default())?;
    let too_large = || SettingsError::invalid("Settings file exceeds the 4 MiB safety limit.");
    if file.metadata()?.len() > limit {
        return Err(too_large());
    }
    let mut bytes = Vec::new();
    file.take(limit.saturating_add(1)).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > limit {
        return Err(too_large());
    }
    String::from_utf8(bytes).map_err(|_| SettingsError::invalid("The settings file is not valid UTF-8 text."))
}

/// Checks a SQLite database and any `-wal`, `-shm` or `-journal` sidecar
/// next to it. The database itself must exist; sidecars may be missing,
/// or unlinked by another connection while being checked.
pub fn validate_sqlite_files(path: &Path) -> Result<(), SettingsError> {
    let database = PrivateFileOptions {
        writable: true,
        ..PrivateFileOptions::default()
    };
    private_file(path, database)?;
    for suffix in ["-wal", "-shm", "-journal"] {
        let mut sidecar = path.as_os_str().to_owned();
        sidecar.push(suffix);
        let options = PrivateFileOptions {
            allow_unlinked: true,
            ..database
        };
        match private_file(Path::new(&sidecar), options) {
            Ok(_) => {}
            Err(SettingsError::Io(error)) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

/// An exclusive `flock` on `settings.lock`, released when dropped.
///
/// `std::fs::File::lock` is `flock(fd, LOCK_EX)` on Linux, the same lock
/// Python's `fcntl.flock` takes, so this excludes the Python app too; the
/// interop tests check it in both directions.
#[derive(Debug)]
pub(crate) struct SettingsLock {
    _file: File,
}

impl SettingsLock {
    /// Name of the lock file inside the settings directory.
    pub const FILE_NAME: &'static str = "settings.lock";

    /// Makes `directory` private and blocks until the lock is held.
    pub fn acquire(directory: &Path) -> Result<Self, SettingsError> {
        private_directory(directory)?;
        let options = PrivateFileOptions {
            create: true,
            writable: true,
            allow_unlinked: false,
        };
        let file = private_file(&directory.join(Self::FILE_NAME), options)?;
        lock_exclusive(&file)?;
        Ok(Self { _file: file })
    }
}

/// Blocks until `file` holds an exclusive `flock`.
fn lock_exclusive(file: &File) -> io::Result<()> {
    file.lock()
}

/// Atomically replaces `target` with `contents`: a private temporary file
/// in the same directory is written, flushed to disk and renamed over the
/// target, so readers see either the old or the new file, never a mix.
/// An existing target must itself be a private file; a symlink is refused
/// rather than replaced.
pub(crate) fn replace_private_file(
    target: &Path,
    prefix: &str,
    contents: &[u8],
) -> Result<(), SettingsError> {
    let directory = target.parent().unwrap_or(Path::new("."));
    private_directory(directory)?;
    match private_file(target, PrivateFileOptions::default()) {
        Ok(_) => {}
        Err(SettingsError::Io(error)) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    let (mut file, temporary) = create_temporary(directory, prefix)?;
    let written = write_and_rename(&mut file, &temporary, target, contents);
    if written.is_err() {
        // The rename did not happen; do not leave the partial copy behind.
        let _ = fs::remove_file(&temporary);
    }
    written?;
    // Make the rename itself durable. Failing here does not undo it.
    if let Ok(parent) = File::open(directory) {
        let _ = parent.sync_all();
    }
    Ok(())
}

/// Writes, flushes and renames the temporary file over `target`.
fn write_and_rename(file: &mut File, temporary: &Path, target: &Path, contents: &[u8]) -> io::Result<()> {
    file.set_permissions(Permissions::from_mode(FILE_MODE))?;
    file.write_all(contents)?;
    file.flush()?;
    file.sync_all()?;
    fs::rename(temporary, target)
}

/// Creates a new private file named `<prefix><random>` in `directory`,
/// like Python's `tempfile.mkstemp`.
fn create_temporary(directory: &Path, prefix: &str) -> io::Result<(File, PathBuf)> {
    const ATTEMPTS: u32 = 100;
    for _ in 0..ATTEMPTS {
        let path = directory.join(format!("{prefix}{}", random_suffix()));
        let created = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(FILE_MODE)
            .custom_flags(libc::O_NOFOLLOW)
            .open(&path);
        match created {
            Ok(file) => return Ok((file, path)),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        }
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "No usable temporary file name was found.",
    ))
}

/// Eight random characters from `[a-z0-9_]`, as `mkstemp` uses.
fn random_suffix() -> String {
    const ALPHABET: &[u8] = b"abcdefghijklmnopqrstuvwxyz0123456789_";
    let mut hasher = std::collections::hash_map::RandomState::new().build_hasher();
    hasher.write_u32(std::process::id());
    let mut value = hasher.finish();
    let alphabet_len = ALPHABET.len() as u64;
    (0..8)
        .map(|_| {
            let index = (value % alphabet_len) as usize;
            value /= alphabet_len;
            char::from(ALPHABET[index])
        })
        .collect()
}

/// `mkdir -p` where only the final directory gets mode 0700.
fn create_directory(path: &Path) -> io::Result<()> {
    let created = DirBuilder::new().mode(DIRECTORY_MODE).create(path);
    match created {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists && path.is_dir() => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent)?;
            }
            match DirBuilder::new().mode(DIRECTORY_MODE).create(path) {
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists && path.is_dir() => Ok(()),
                other => other,
            }
        }
        Err(error) => Err(error),
    }
}

/// This process's effective user ID, read once from `/proc/self/status`
/// (the standard library has no safe `geteuid`).
fn effective_uid() -> io::Result<u32> {
    static EFFECTIVE_UID: OnceLock<Option<u32>> = OnceLock::new();
    let uid = EFFECTIVE_UID.get_or_init(|| {
        let status = fs::read_to_string("/proc/self/status").ok()?;
        parse_effective_uid(&status)
    });
    uid.ok_or_else(|| io::Error::other("The effective user ID could not be determined."))
}

/// The second field of the `Uid:` line (real, effective, saved, file system).
fn parse_effective_uid(status: &str) -> Option<u32> {
    let line = status.lines().find_map(|line| line.strip_prefix("Uid:"))?;
    line.split_whitespace().nth(1)?.parse().ok()
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::symlink;

    use super::*;

    fn mode(path: &Path) -> u32 {
        fs::metadata(path).unwrap().mode() & 0o777
    }

    fn writable() -> PrivateFileOptions {
        PrivateFileOptions {
            writable: true,
            ..PrivateFileOptions::default()
        }
    }

    #[test]
    fn effective_uid_parses_the_status_line() {
        let status = "Name:\tx\nUid:\t1000\t1001\t1000\t1000\nGid:\t5\t5\t5\t5\n";
        assert_eq!(parse_effective_uid(status), Some(1001));
        assert_eq!(parse_effective_uid("Name:\tx\n"), None);
        let own_file = tempfile::NamedTempFile::new().unwrap();
        let owner = own_file.as_file().metadata().unwrap().uid();
        assert_eq!(effective_uid().unwrap(), owner);
    }

    /// Ported from desktop/tests/test_terminal_security.py::PrivateStorageTests::test_permissions
    #[test]
    fn permissions() {
        let root = tempfile::tempdir().unwrap();
        let directory = root.path().join("private");
        private_directory(&directory).unwrap();
        let options = PrivateFileOptions {
            create: true,
            writable: true,
            allow_unlinked: false,
        };
        drop(private_file(&directory.join("state"), options).unwrap());
        assert_eq!(mode(&directory), 0o700);
        assert_eq!(mode(&directory.join("state")), 0o600);
    }

    /// Ported from desktop/tests/test_terminal_security.py::PrivateStorageTests::test_directory_symlink_not_chmodded
    #[test]
    fn directory_symlink_not_chmodded() {
        let root = tempfile::tempdir().unwrap();
        let real = root.path().join("real");
        DirBuilder::new().mode(0o755).create(&real).unwrap();
        fs::set_permissions(&real, Permissions::from_mode(0o755)).unwrap();
        let link = root.path().join("link");
        symlink(&real, &link).unwrap();
        assert!(matches!(private_directory(&link), Err(SettingsError::Io(_))));
        assert_eq!(mode(&real), 0o755);
    }

    /// Ported from desktop/tests/test_terminal_security.py::PrivateStorageTests::test_file_symlink_leaves_target_unchanged
    #[test]
    fn file_symlink_leaves_target_unchanged() {
        let root = tempfile::tempdir().unwrap();
        let real = root.path().join("real");
        fs::write(&real, "private").unwrap();
        fs::set_permissions(&real, Permissions::from_mode(0o644)).unwrap();
        let link = root.path().join("link");
        symlink(&real, &link).unwrap();
        assert!(matches!(
            private_file(&link, writable()),
            Err(SettingsError::Io(_))
        ));
        assert_eq!(fs::read_to_string(&real).unwrap(), "private");
        assert_eq!(mode(&real), 0o644);
    }

    /// Ported from desktop/tests/test_terminal_security.py::PrivateStorageTests::test_hardlink_rejected
    #[test]
    fn hardlink_rejected() {
        let root = tempfile::tempdir().unwrap();
        let real = root.path().join("real");
        fs::write(&real, "x").unwrap();
        fs::hard_link(&real, root.path().join("link")).unwrap();
        let result = private_file(&root.path().join("link"), PrivateFileOptions::default());
        assert!(matches!(result, Err(SettingsError::Invalid(_))));
    }

    /// Ported from desktop/tests/test_terminal_security.py::PrivateStorageTests::test_fifo_does_not_block
    #[test]
    fn fifo_does_not_block() {
        let root = tempfile::tempdir().unwrap();
        let fifo = root.path().join("pipe");
        let made = std::process::Command::new("mkfifo")
            .arg(&fifo)
            .status()
            .expect("mkfifo (GNU coreutils) is required for the private-storage FIFO safety test");
        assert!(made.success(), "mkfifo failed to create the FIFO fixture: {made}");
        let result = private_file(&fifo, PrivateFileOptions::default());
        assert!(matches!(result, Err(SettingsError::Invalid(_))));
    }

    /// Ported from desktop/tests/test_terminal_security.py::PrivateStorageTests::test_settings_read_bound
    #[test]
    fn settings_read_bound() {
        let root = tempfile::tempdir().unwrap();
        let file = root.path().join("large");
        fs::write(&file, [b'x'; 33]).unwrap();
        assert!(matches!(private_text(&file, 32), Err(SettingsError::Invalid(_))));
        assert_eq!(private_text(&file, 33).unwrap().len(), 33);
    }

    /// Ported from desktop/tests/test_terminal_security.py::PrivateStorageTests::test_database_symlink_refused
    #[test]
    fn database_symlink_refused() {
        let root = tempfile::tempdir().unwrap();
        let target = root.path().join("target");
        fs::write(&target, b"unchanged").unwrap();
        symlink(&target, root.path().join("search.sqlite3")).unwrap();
        let result = validate_sqlite_files(&root.path().join("search.sqlite3"));
        assert!(matches!(result, Err(SettingsError::Io(_))));
        assert_eq!(fs::read(&target).unwrap(), b"unchanged");
    }

    /// Ported from desktop/tests/test_terminal_security.py::PrivateStorageTests::test_database_sidecar_symlink_refused
    #[test]
    fn database_sidecar_symlink_refused() {
        let root = tempfile::tempdir().unwrap();
        let database = root.path().join("search.sqlite3");
        fs::write(&database, b"").unwrap();
        let target = root.path().join("target");
        fs::write(&target, b"unchanged").unwrap();
        symlink(&target, root.path().join("search.sqlite3-wal")).unwrap();
        assert!(matches!(
            validate_sqlite_files(&database),
            Err(SettingsError::Io(_))
        ));
        assert_eq!(fs::read(&target).unwrap(), b"unchanged");
        fs::remove_file(root.path().join("search.sqlite3-wal")).unwrap();
        validate_sqlite_files(&database).unwrap();
    }

    #[test]
    fn missing_parents_are_created_and_only_the_leaf_is_private() {
        let root = tempfile::tempdir().unwrap();
        let leaf = root.path().join("a/b/winspace");
        private_directory(&leaf).unwrap();
        assert_eq!(mode(&leaf), 0o700);
        private_directory(&leaf).unwrap();
    }

    #[test]
    fn replace_writes_a_private_file_and_refuses_symlinks() {
        let root = tempfile::tempdir().unwrap();
        let target = root.path().join("settings.json");
        replace_private_file(&target, ".settings-", b"one").unwrap();
        replace_private_file(&target, ".settings-", b"two").unwrap();
        assert_eq!(fs::read(&target).unwrap(), b"two");
        assert_eq!(mode(&target), 0o600);
        let leftovers = fs::read_dir(root.path()).unwrap().count();
        assert_eq!(leftovers, 1, "no temporary files remain");

        let other = root.path().join("other");
        fs::write(&other, "{}").unwrap();
        let link = root.path().join("linked.json");
        symlink(&other, &link).unwrap();
        assert!(replace_private_file(&link, ".settings-", b"x").is_err());
        assert!(link.is_symlink());
        assert_eq!(fs::read_to_string(&other).unwrap(), "{}");
    }

    #[test]
    fn random_suffixes_differ() {
        let first = random_suffix();
        assert_eq!(first.len(), 8);
        assert!((0..10).any(|_| random_suffix() != first));
    }
}
