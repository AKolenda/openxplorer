// SPDX-License-Identifier: AGPL-3.0-only
//! Private application state: owned 0700 directories and 0600 files that
//! are never symlinks, hard links, FIFOs or devices.
//!
//! Ports `desktop/private_storage.py`; the lock and the atomic replace
//! built on it are in the `save` module. This is defence in depth for misplaced or tampered XDG state, not
//! isolation from another process running as the same user. Every open
//! uses `O_NOFOLLOW` (a symlinked leaf fails with `ELOOP`) and
//! `O_NONBLOCK` (a FIFO never blocks), and the checks run on the opened
//! descriptor before its mode is changed.

use std::fs::{self, DirBuilder, File, Metadata, OpenOptions, Permissions};
use std::io::{self, Read};
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::Path;

use super::error::WithPath;
use super::SettingsError;

/// Largest settings file read, in bytes.
pub const SETTINGS_SIZE_LIMIT: u64 = 4 * 1024 * 1024;

/// Mode of private directories.
const DIRECTORY_MODE: u32 = 0o700;

/// Mode of private files.
pub(super) const FILE_MODE: u32 = 0o600;

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
///
/// # Errors
///
/// [`SettingsError::Io`] if the directory cannot be created or opened,
/// including `ELOOP` for a symlink; [`SettingsError::Invalid`] if it is
/// owned by another user.
pub fn private_directory(path: &Path) -> Result<(), SettingsError> {
    create_directory(path).with_path(path)?;
    let directory = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW)
        .open(path)
        .with_path(path)?;
    let info = directory.metadata().with_path(path)?;
    if !info.is_dir() || info.uid() != effective_uid() {
        return Err(SettingsError::invalid(
            "Application state directory must be owned by this user.",
        ));
    }
    directory
        .set_permissions(Permissions::from_mode(DIRECTORY_MODE))
        .with_path(path)
}

/// Opens a private file. It must be a regular file owned by this user with
/// exactly one link; its mode becomes 0600. The checks run before the mode
/// changes, so a rejected file is never modified.
///
/// # Errors
///
/// [`SettingsError::Io`] if the file cannot be opened, including `ELOOP`
/// for a symlink and `ENOENT` for a missing file without
/// [`create`](PrivateFileOptions::create); [`SettingsError::Invalid`] for a
/// hard link, FIFO, device or a file owned by another user.
pub fn private_file(path: &Path, options: PrivateFileOptions) -> Result<File, SettingsError> {
    let file = open_without_following(path, options).with_path(path)?;
    let opened = OpenedFile::inspect(&file.metadata().with_path(path)?);
    match opened.verdict(options) {
        Verdict::Refuse => Err(SettingsError::invalid(
            "Application state must be an owned regular file, not a link or device.",
        )),
        Verdict::AcceptUnlinked => Ok(file),
        Verdict::AcceptAndMakePrivate => {
            file.set_permissions(Permissions::from_mode(FILE_MODE))
                .with_path(path)?;
            Ok(file)
        }
    }
}

/// Reads a private UTF-8 text file of at most `limit` bytes.
///
/// # Errors
///
/// Everything [`private_file`] refuses, plus [`SettingsError::Invalid`] for
/// a file over `limit` bytes or one that is not UTF-8.
pub fn private_text(path: &Path, limit: u64) -> Result<String, SettingsError> {
    let file = private_file(path, PrivateFileOptions::default())?;
    read_limited_text(file, path, limit)
}

/// Reads an opened file as UTF-8 text of at most `limit` bytes. A file that
/// grows past the limit while it is read is refused too.
///
/// # Errors
///
/// [`SettingsError::Io`] if reading fails; [`SettingsError::Invalid`] if the
/// contents are too large or not UTF-8.
pub(crate) fn read_limited_text(file: File, path: &Path, limit: u64) -> Result<String, SettingsError> {
    // The message names 4 MiB whatever the limit, as private_storage.py does.
    let too_large = || SettingsError::invalid("Settings file exceeds the 4 MiB safety limit.");
    if file.metadata().with_path(path)?.len() > limit {
        return Err(too_large());
    }
    let mut bytes = Vec::new();
    file.take(limit.saturating_add(1))
        .read_to_end(&mut bytes)
        .with_path(path)?;
    if bytes.len() as u64 > limit {
        return Err(too_large());
    }
    String::from_utf8(bytes).map_err(|_| SettingsError::invalid("The settings file is not valid UTF-8 text."))
}

/// Checks a SQLite database and any `-wal`, `-shm` or `-journal` sidecar
/// next to it. The database itself must exist; sidecars may be missing,
/// or unlinked by another connection while being checked.
///
/// # Errors
///
/// Everything [`private_file`] refuses for the database or a sidecar
/// that still exists.
pub fn validate_sqlite_files(path: &Path) -> Result<(), SettingsError> {
    let database = PrivateFileOptions {
        writable: true,
        ..PrivateFileOptions::default()
    };
    private_file(path, database)?;
    let sidecar_options = PrivateFileOptions {
        allow_unlinked: true,
        ..database
    };
    for suffix in ["-wal", "-shm", "-journal"] {
        let mut sidecar = path.as_os_str().to_owned();
        sidecar.push(suffix);
        match private_file(Path::new(&sidecar), sidecar_options) {
            Err(error) if !error.is_not_found() => return Err(error),
            _ => {}
        }
    }
    Ok(())
}

/// What `fstat` reported about an opened file, reduced to what
/// [`private_file`] decides on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct OpenedFile {
    is_regular: bool,
    is_owned: bool,
    links: u64,
}

/// What [`private_file`] does with an opened file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Verdict {
    /// Not an owned regular file with one link: close it untouched.
    Refuse,
    /// An owned regular file with one link: set its mode to 0600.
    AcceptAndMakePrivate,
    /// An owned regular file without a directory entry, allowed by
    /// [`PrivateFileOptions::allow_unlinked`]. Its mode is left alone, as
    /// `private_file` in `private_storage.py` does.
    AcceptUnlinked,
}

impl OpenedFile {
    fn inspect(info: &Metadata) -> Self {
        Self {
            is_regular: info.file_type().is_file(),
            is_owned: info.uid() == effective_uid(),
            links: info.nlink(),
        }
    }

    /// Safety rule "private state is an owned regular file with one link"
    /// (`private_file` in `private_storage.py`): a second link could expose the
    /// contents elsewhere, and a FIFO or device is never state.
    fn verdict(self, options: PrivateFileOptions) -> Verdict {
        if !self.is_regular || !self.is_owned {
            return Verdict::Refuse;
        }
        match self.links {
            1 => Verdict::AcceptAndMakePrivate,
            0 if options.allow_unlinked => Verdict::AcceptUnlinked,
            _ => Verdict::Refuse,
        }
    }
}

/// Opens `path` without following a symlinked leaf and without blocking on
/// a FIFO, creating it with mode 0600 if requested.
fn open_without_following(path: &Path, options: PrivateFileOptions) -> io::Result<File> {
    let mut flags = libc::O_NOFOLLOW | libc::O_NONBLOCK;
    if options.create {
        flags |= libc::O_CREAT;
    }
    OpenOptions::new()
        .read(true)
        .write(options.writable)
        .mode(FILE_MODE)
        .custom_flags(flags)
        .open(path)
}

/// `mkdir -p` where only the final directory gets mode 0700, like Python's
/// `mkdir(parents=True, exist_ok=True, mode=0o700)`.
fn create_directory(path: &Path) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    match DirBuilder::new().mode(DIRECTORY_MODE).create(path) {
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists && path.is_dir() => Ok(()),
        created => created,
    }
}

/// This process's effective user ID, which must own every private file.
///
/// `GCredentials` records `geteuid()` on Linux. The standard library has
/// no safe `geteuid`.
fn effective_uid() -> u32 {
    gio::Credentials::new()
        .unix_user()
        .expect("GCredentials holds the effective user ID on Linux")
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

    fn owned_regular_file(links: u64) -> OpenedFile {
        OpenedFile {
            is_regular: true,
            is_owned: true,
            links,
        }
    }

    #[test]
    fn effective_uid_owns_new_files() {
        let own_file = tempfile::NamedTempFile::new().unwrap();
        let owner = own_file.as_file().metadata().unwrap().uid();
        assert_eq!(effective_uid(), owner);
    }

    /// Ported from `desktop/tests/test_terminal_security.py::PrivateStorageTests::test_permissions`
    /// parity: SAFE-009
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

    /// Ported from `desktop/tests/test_terminal_security.py::PrivateStorageTests::test_directory_symlink_not_chmodded`
    /// parity: SAFE-009
    #[test]
    fn directory_symlink_not_chmodded() {
        let root = tempfile::tempdir().unwrap();
        let real = root.path().join("real");
        DirBuilder::new().mode(0o755).create(&real).unwrap();
        fs::set_permissions(&real, Permissions::from_mode(0o755)).unwrap();
        let link = root.path().join("link");
        symlink(&real, &link).unwrap();
        assert!(matches!(private_directory(&link), Err(SettingsError::Io { .. })));
        assert_eq!(mode(&real), 0o755);
    }

    /// Ported from `desktop/tests/test_terminal_security.py::PrivateStorageTests::test_file_symlink_leaves_target_unchanged`
    /// parity: SAFE-009
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
            Err(SettingsError::Io { .. })
        ));
        assert_eq!(fs::read_to_string(&real).unwrap(), "private");
        assert_eq!(mode(&real), 0o644);
    }

    /// Ported from `desktop/tests/test_terminal_security.py::PrivateStorageTests::test_hardlink_rejected`
    /// parity: SAFE-009
    #[test]
    fn hardlink_rejected() {
        let root = tempfile::tempdir().unwrap();
        let real = root.path().join("real");
        fs::write(&real, "x").unwrap();
        fs::hard_link(&real, root.path().join("link")).unwrap();
        let result = private_file(&root.path().join("link"), PrivateFileOptions::default());
        assert!(matches!(result, Err(SettingsError::Invalid(_))));
    }

    /// Ported from `desktop/tests/test_terminal_security.py::PrivateStorageTests::test_fifo_does_not_block`
    /// parity: SAFE-009
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

    /// Ported from `desktop/tests/test_terminal_security.py::PrivateStorageTests::test_settings_read_bound`
    /// parity: SAFE-009, SET-013
    #[test]
    fn settings_read_bound() {
        let root = tempfile::tempdir().unwrap();
        let file = root.path().join("large");
        fs::write(&file, [b'x'; 33]).unwrap();
        assert!(matches!(private_text(&file, 32), Err(SettingsError::Invalid(_))));
        assert_eq!(private_text(&file, 33).unwrap().len(), 33);
    }

    /// Ported from `desktop/tests/test_terminal_security.py::PrivateStorageTests::test_database_symlink_refused`
    /// parity: SAFE-009
    #[test]
    fn database_symlink_refused() {
        let root = tempfile::tempdir().unwrap();
        let target = root.path().join("target");
        fs::write(&target, b"unchanged").unwrap();
        symlink(&target, root.path().join("search.sqlite3")).unwrap();
        let result = validate_sqlite_files(&root.path().join("search.sqlite3"));
        assert!(matches!(result, Err(SettingsError::Io { .. })));
        assert_eq!(fs::read(&target).unwrap(), b"unchanged");
    }

    /// Ported from `desktop/tests/test_terminal_security.py::PrivateStorageTests::test_database_sidecar_symlink_refused`
    /// parity: SAFE-009
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
            Err(SettingsError::Io { .. })
        ));
        assert_eq!(fs::read(&target).unwrap(), b"unchanged");
        fs::remove_file(root.path().join("search.sqlite3-wal")).unwrap();
        validate_sqlite_files(&database).unwrap();
    }

    /// Ported from `desktop/tests/test_terminal_security.py::PrivateStorageTests::test_sqlite_sidecar_unlinked_during_check_is_allowed`
    ///
    /// Python fakes `fstat` to report no links; here the decision is a pure
    /// function of what `fstat` reported, so the same facts are passed in.
    /// parity: SAFE-009
    #[test]
    fn sqlite_sidecar_unlinked_during_check_is_allowed() {
        let unlinked = owned_regular_file(0);
        let sidecar = PrivateFileOptions {
            writable: true,
            allow_unlinked: true,
            ..PrivateFileOptions::default()
        };
        assert_eq!(unlinked.verdict(sidecar), Verdict::AcceptUnlinked);
        assert_eq!(unlinked.verdict(writable()), Verdict::Refuse);
    }

    #[test]
    fn only_owned_regular_files_with_one_link_are_accepted() {
        let options = PrivateFileOptions {
            allow_unlinked: true,
            ..PrivateFileOptions::default()
        };
        let foreign = OpenedFile {
            is_owned: false,
            ..owned_regular_file(1)
        };
        let fifo = OpenedFile {
            is_regular: false,
            ..owned_regular_file(1)
        };
        assert_eq!(
            owned_regular_file(1).verdict(options),
            Verdict::AcceptAndMakePrivate
        );
        assert_eq!(owned_regular_file(2).verdict(options), Verdict::Refuse);
        assert_eq!(foreign.verdict(options), Verdict::Refuse);
        assert_eq!(fifo.verdict(options), Verdict::Refuse);
    }

    #[test]
    fn missing_parents_are_created_and_only_the_leaf_is_private() {
        let root = tempfile::tempdir().unwrap();
        let leaf = root.path().join("a/b/winspace");
        private_directory(&leaf).unwrap();
        assert_eq!(mode(&leaf), 0o700);
        private_directory(&leaf).unwrap();
    }
}
