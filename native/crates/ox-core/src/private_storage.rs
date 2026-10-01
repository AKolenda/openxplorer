// SPDX-License-Identifier: AGPL-3.0-only
//! Private application state: owned 0700 directories and 0600 files that
//! are never symlinks, hard links, FIFOs or devices.
//!
//! Ports `v2.0.0:desktop/private_storage.py`, which several Python services use.
//! The settings keep `settings.json` here (their lock is in
//! `settings::save`) and the previous-versions service its
//! `snapshot-sources.json`; both save through the atomic replace of
//! `replace` ([`replace_file_atomically`], [`replace_file_with`]). The
//! search index checks its SQLite files with [`validate_sqlite_files`] on
//! every connection. This is defence in depth against misplaced or
//! tampered XDG state, not isolation from another process running as the
//! same user.
//!
//! Every open uses `O_NOFOLLOW` (a symlinked leaf fails with `ELOOP`) and
//! `O_NONBLOCK` (a FIFO never blocks), and the checks run on the opened
//! descriptor before its mode is changed, so a refused file is never
//! modified.

mod replace;

pub(crate) use replace::{parent_directory, replace_file_atomically, replace_file_with, reserve_unique_name};

use std::fs::{self, DirBuilder, File, Metadata, OpenOptions, Permissions};
use std::io::{self, Read};
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

use rustix::fs::OFlags;

/// Mode of private directories.
const DIRECTORY_MODE: u32 = 0o700;

/// Mode of private files.
pub(crate) const FILE_MODE: u32 = 0o600;

/// The sidecar files SQLite keeps next to a database.
const SQLITE_SIDECAR_SUFFIXES: [&str; 3] = ["-wal", "-shm", "-journal"];

/// Why private storage refused a file or directory, or could not use it.
///
/// Python raises `ValueError` for a refusal and `OSError` for everything
/// else; both name the path, so a warning can say which file it was.
#[derive(Debug, thiserror::Error)]
pub enum StorageError {
    /// `path` broke a private-storage rule.
    #[error("{reason} ({})", path.display())]
    Refused {
        /// The file or directory that was refused.
        path: PathBuf,
        /// Which rule it broke.
        reason: StorageRefusal,
    },
    /// The file system refused an operation on `path`, for example `ELOOP`
    /// when the path is a symlink.
    #[error("{error}: {}", path.display())]
    Io {
        /// The file or directory the operation was on.
        path: PathBuf,
        /// What the operating system reported.
        error: io::Error,
    },
}

/// The private-storage rule a file or directory broke, in the words of
/// `v2.0.0:desktop/private_storage.py`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum StorageRefusal {
    /// The application's own directory belongs to another user.
    #[error("Application state directory must be owned by this user.")]
    ForeignDirectory,
    /// A hard link, FIFO, device, or a file that belongs to another user.
    #[error("Application state must be an owned regular file, not a link or device.")]
    NotPrivateFile,
    /// Larger than the read limit. The message names 4 MiB whatever the
    /// limit, as `private_text` in `private_storage.py` does.
    #[error("Settings file exceeds the 4 MiB safety limit.")]
    TooLarge,
    /// Not UTF-8, which Python's `decode('utf-8')` refuses.
    #[error("The settings file is not valid UTF-8 text.")]
    NotText,
}

impl StorageError {
    /// A refusal of `path`.
    pub(crate) fn refused(path: &Path, reason: StorageRefusal) -> Self {
        Self::Refused {
            path: path.to_path_buf(),
            reason,
        }
    }

    /// A file-system error on `path`.
    pub(crate) fn io(path: &Path, error: io::Error) -> Self {
        Self::Io {
            path: path.to_path_buf(),
            error,
        }
    }

    /// Whether this is a missing file or directory, which Python reports as
    /// `FileNotFoundError` and [`private_file_if_present`] treats as
    /// "nothing there yet".
    pub(crate) fn is_not_found(&self) -> bool {
        matches!(self, Self::Io { error, .. } if error.kind() == io::ErrorKind::NotFound)
    }
}

/// Adds the affected path to an I/O error, as Python's `OSError` does.
pub(crate) trait WithPath<T> {
    /// This result, with an error turned into [`StorageError::Io`] on
    /// `path`.
    fn with_path(self, path: &Path) -> Result<T, StorageError>;
}

impl<T> WithPath<T> for io::Result<T> {
    fn with_path(self, path: &Path) -> Result<T, StorageError> {
        self.map_err(|error| StorageError::io(path, error))
    }
}

/// How a private file is opened.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct PrivateFileOptions {
    /// Create the file (mode 0600) if it is missing.
    pub(crate) create: bool,
    /// Open for reading and writing instead of reading only.
    pub(crate) writable: bool,
    /// Accept a file whose last directory entry was removed after opening,
    /// as SQLite does with its sidecar files. Its mode is left unchanged.
    pub(crate) allow_unlinked: bool,
}

/// Creates `path` (and missing parents) and makes it a private directory:
/// it must be a real directory owned by this user, not a symlink, and its
/// mode becomes 0700. Parents keep their default mode, as in Python.
///
/// Safety rule "private state lives in an owned, real directory"
/// (`private_directory` in `private_storage.py`): the directory is opened
/// with `O_DIRECTORY | O_NOFOLLOW`, so a symlink fails with `ELOOP` before
/// anything changes its target's mode, and one owned by another user is
/// refused.
///
/// # Errors
///
/// [`StorageError::Io`] if the directory cannot be created or opened,
/// including `ELOOP` for a symlink; [`StorageError::Refused`] with
/// [`StorageRefusal::ForeignDirectory`] if it is owned by another user.
pub(crate) fn private_directory(path: &Path) -> Result<(), StorageError> {
    create_directory(path).with_path(path)?;
    let directory = OpenOptions::new()
        .read(true)
        .kernel_flags(OFlags::DIRECTORY | OFlags::NOFOLLOW)
        .open(path)
        .with_path(path)?;
    let metadata = directory.metadata().with_path(path)?;
    if !metadata.is_dir() || metadata.uid() != effective_uid() {
        return Err(StorageError::refused(path, StorageRefusal::ForeignDirectory));
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
/// [`StorageError::Io`] if the file cannot be opened, including `ELOOP`
/// for a symlink and `ENOENT` for a missing file without
/// [`create`](PrivateFileOptions::create); [`StorageError::Refused`] with
/// [`StorageRefusal::NotPrivateFile`] for a hard link, FIFO, device or a
/// file owned by another user.
pub(crate) fn private_file(path: &Path, options: PrivateFileOptions) -> Result<File, StorageError> {
    let file = open_without_following(path, options).with_path(path)?;
    let opened = OpenedFile::inspect(&file.metadata().with_path(path)?);
    match opened.verdict(options) {
        Verdict::Refuse => Err(StorageError::refused(path, StorageRefusal::NotPrivateFile)),
        Verdict::AcceptUnlinked => Ok(file),
        Verdict::AcceptAndMakePrivate => {
            file.set_permissions(Permissions::from_mode(FILE_MODE))
                .with_path(path)?;
            Ok(file)
        }
    }
}

/// [`private_file`] for a file that may be missing: `Ok(None)` if there is
/// nothing at `path`, as Python's `except FileNotFoundError: pass`.
///
/// # Errors
///
/// Everything [`private_file`] refuses except a missing file.
pub(crate) fn private_file_if_present(
    path: &Path,
    options: PrivateFileOptions,
) -> Result<Option<File>, StorageError> {
    match private_file(path, options) {
        Ok(file) => Ok(Some(file)),
        Err(error) if error.is_not_found() => Ok(None),
        Err(error) => Err(error),
    }
}

/// Reads an opened file as UTF-8 text of at most `limit` bytes. Together
/// with [`private_file`] this is `private_text` in `private_storage.py`;
/// the settings reader calls them one after the other so that it can tell
/// a refused file from unusable contents.
///
/// Safety rule "settings reads are bounded" (`private_text` in
/// `private_storage.py`): a file over the limit is refused before it is
/// read, and so is one that grows past the limit while it is read.
///
/// # Errors
///
/// [`StorageError::Io`] if reading fails; [`StorageError::Refused`] with
/// [`StorageRefusal::TooLarge`] or [`StorageRefusal::NotText`] if the
/// contents are too large or not UTF-8.
pub(crate) fn read_limited_text(file: File, path: &Path, limit: u64) -> Result<String, StorageError> {
    if file.metadata().with_path(path)?.len() > limit {
        return Err(StorageError::refused(path, StorageRefusal::TooLarge));
    }
    let mut bytes = Vec::new();
    file.take(limit.saturating_add(1))
        .read_to_end(&mut bytes)
        .with_path(path)?;
    if bytes.len() as u64 > limit {
        return Err(StorageError::refused(path, StorageRefusal::TooLarge));
    }
    String::from_utf8(bytes).map_err(|_| StorageError::refused(path, StorageRefusal::NotText))
}

/// Checks a SQLite database and any `-wal`, `-shm` or `-journal` sidecar
/// next to it. The database itself must exist; sidecars may be missing,
/// or unlinked by another connection while being checked.
///
/// # Errors
///
/// Everything [`private_file`] refuses for the database or a sidecar
/// that still exists.
pub(crate) fn validate_sqlite_files(path: &Path) -> Result<(), StorageError> {
    let database = PrivateFileOptions {
        writable: true,
        ..PrivateFileOptions::default()
    };
    private_file(path, database)?;
    let sidecar = PrivateFileOptions {
        allow_unlinked: true,
        ..database
    };
    for suffix in SQLITE_SIDECAR_SUFFIXES {
        let mut sidecar_path = path.as_os_str().to_owned();
        sidecar_path.push(suffix);
        private_file_if_present(Path::new(&sidecar_path), sidecar)?;
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
    /// The facts of an opened file's `metadata` that decide its verdict.
    fn inspect(metadata: &Metadata) -> Self {
        Self {
            is_regular: metadata.file_type().is_file(),
            is_owned: metadata.uid() == effective_uid(),
            links: metadata.nlink(),
        }
    }

    /// Safety rule "private state is an owned regular file with one link"
    /// (`private_file` in `private_storage.py`): a second link could expose
    /// the contents elsewhere, and a FIFO or device is never state.
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

/// Opens `path`, creating it with mode 0600 if requested.
///
/// Safety rule "never follow a link, never block on a FIFO": `O_NOFOLLOW`
/// fails on a symlinked leaf instead of opening its target, and
/// `O_NONBLOCK` returns at once for a FIFO, which the checks then refuse.
fn open_without_following(path: &Path, options: PrivateFileOptions) -> io::Result<File> {
    let mut flags = OFlags::NOFOLLOW | OFlags::NONBLOCK;
    if options.create {
        flags |= OFlags::CREATE;
    }
    OpenOptions::new()
        .read(true)
        .write(options.writable)
        .mode(FILE_MODE)
        .kernel_flags(flags)
        .open(path)
}

/// Kernel open flags for std's [`OpenOptions`], as rustix's typed [`OFlags`]
/// rather than raw integers. The crate does all descriptor-level I/O through
/// rustix: opening a path with these flags, and the descriptor-relative
/// calls std lacks (`openat`, `renameat2` with `RENAME_NOREPLACE`).
pub(crate) trait KernelOpenFlags {
    /// Adds `flags`, such as `O_NOFOLLOW` or `O_NONBLOCK`, to the open call.
    fn kernel_flags(&mut self, flags: OFlags) -> &mut Self;
}

impl KernelOpenFlags for OpenOptions {
    fn kernel_flags(&mut self, flags: OFlags) -> &mut Self {
        self.custom_flags(flags.bits().cast_signed())
    }
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
    use crate::test_support::{make_fifo, permission_bits};

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

    /// Python's `private_text`: [`private_file`], then
    /// [`read_limited_text`].
    fn private_text(path: &Path, limit: u64) -> Result<String, StorageError> {
        let file = private_file(path, PrivateFileOptions::default())?;
        read_limited_text(file, path, limit)
    }

    /// parity: SAFE-009
    #[test]
    fn effective_uid_owns_new_files() {
        let own_file = tempfile::NamedTempFile::new().unwrap();
        let owner = own_file.as_file().metadata().unwrap().uid();
        assert_eq!(effective_uid(), owner);
    }

    /// Ported from `v2.0.0:desktop/tests/test_terminal_security.py::PrivateStorageTests::test_permissions`
    /// parity: SAFE-009
    #[test]
    fn private_storage_gets_modes_0700_and_0600() {
        let root = tempfile::tempdir().unwrap();
        let directory = root.path().join("private");
        private_directory(&directory).unwrap();
        let options = PrivateFileOptions {
            create: true,
            writable: true,
            allow_unlinked: false,
        };
        drop(private_file(&directory.join("state"), options).unwrap());
        assert_eq!(permission_bits(&directory), 0o700);
        assert_eq!(permission_bits(&directory.join("state")), 0o600);
    }

    /// Ported from `v2.0.0:desktop/tests/test_terminal_security.py::PrivateStorageTests::test_directory_symlink_not_chmodded`
    /// parity: SAFE-009
    #[test]
    fn directory_symlink_not_chmodded() {
        let root = tempfile::tempdir().unwrap();
        let real = root.path().join("real");
        DirBuilder::new().mode(0o755).create(&real).unwrap();
        fs::set_permissions(&real, Permissions::from_mode(0o755)).unwrap();
        let link = root.path().join("link");
        symlink(&real, &link).unwrap();
        assert!(matches!(private_directory(&link), Err(StorageError::Io { .. })));
        assert_eq!(permission_bits(&real), 0o755);
    }

    /// Ported from `v2.0.0:desktop/tests/test_terminal_security.py::PrivateStorageTests::test_file_symlink_leaves_target_unchanged`
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
            Err(StorageError::Io { .. })
        ));
        assert_eq!(fs::read_to_string(&real).unwrap(), "private");
        assert_eq!(permission_bits(&real), 0o644);
    }

    /// Ported from `v2.0.0:desktop/tests/test_terminal_security.py::PrivateStorageTests::test_hardlink_rejected`
    /// parity: SAFE-009
    #[test]
    fn a_hard_linked_file_is_refused() {
        let root = tempfile::tempdir().unwrap();
        let real = root.path().join("real");
        fs::write(&real, "x").unwrap();
        fs::hard_link(&real, root.path().join("link")).unwrap();
        let result = private_file(&root.path().join("link"), PrivateFileOptions::default());
        assert!(matches!(
            result,
            Err(StorageError::Refused {
                reason: StorageRefusal::NotPrivateFile,
                ..
            })
        ));
    }

    /// Ported from `v2.0.0:desktop/tests/test_terminal_security.py::PrivateStorageTests::test_fifo_does_not_block`
    /// parity: SAFE-009
    #[test]
    fn fifo_does_not_block() {
        let root = tempfile::tempdir().unwrap();
        let fifo = root.path().join("pipe");
        make_fifo(&fifo);
        let result = private_file(&fifo, PrivateFileOptions::default());
        assert!(matches!(
            result,
            Err(StorageError::Refused {
                reason: StorageRefusal::NotPrivateFile,
                ..
            })
        ));
    }

    /// Ported from `v2.0.0:desktop/tests/test_terminal_security.py::PrivateStorageTests::test_settings_read_bound`
    /// parity: SAFE-009, SET-013
    #[test]
    fn reads_beyond_the_size_limit_are_refused() {
        let root = tempfile::tempdir().unwrap();
        let file = root.path().join("large");
        fs::write(&file, [b'x'; 33]).unwrap();
        assert!(matches!(
            private_text(&file, 32),
            Err(StorageError::Refused {
                reason: StorageRefusal::TooLarge,
                ..
            })
        ));
        assert_eq!(private_text(&file, 33).unwrap().len(), 33);
    }

    /// Ported from `v2.0.0:desktop/tests/test_terminal_security.py::PrivateStorageTests::test_database_symlink_refused`
    /// parity: SAFE-009
    #[test]
    fn a_symlinked_database_is_refused_and_its_target_unchanged() {
        let root = tempfile::tempdir().unwrap();
        let target = root.path().join("target");
        fs::write(&target, b"unchanged").unwrap();
        symlink(&target, root.path().join("search.sqlite3")).unwrap();
        let result = validate_sqlite_files(&root.path().join("search.sqlite3"));
        assert!(matches!(result, Err(StorageError::Io { .. })));
        assert_eq!(fs::read(&target).unwrap(), b"unchanged");
    }

    /// Ported from `v2.0.0:desktop/tests/test_terminal_security.py::PrivateStorageTests::test_database_sidecar_symlink_refused`
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
            Err(StorageError::Io { .. })
        ));
        assert_eq!(fs::read(&target).unwrap(), b"unchanged");
        fs::remove_file(root.path().join("search.sqlite3-wal")).unwrap();
        validate_sqlite_files(&database).unwrap();
    }

    /// Ported from `v2.0.0:desktop/tests/test_terminal_security.py::PrivateStorageTests::test_sqlite_sidecar_unlinked_during_check_is_allowed`
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

    /// parity: SAFE-009
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

    /// parity: SAFE-009
    #[test]
    fn missing_parents_are_created_and_only_the_leaf_is_private() {
        let root = tempfile::tempdir().unwrap();
        let leaf = root.path().join("a/b/winspace");
        let reference = root.path().join("reference");
        fs::create_dir(&reference).unwrap();

        private_directory(&leaf).unwrap();

        assert_eq!(permission_bits(&leaf), 0o700);
        let default_mode = permission_bits(&reference);
        for parent in ["a", "a/b"] {
            let parent_mode = permission_bits(&root.path().join(parent));
            assert_eq!(parent_mode, default_mode, "{parent} keeps the default mode");
        }
        private_directory(&leaf).expect("an existing private directory is accepted again");
    }

    /// parity: SAFE-009
    #[test]
    fn a_missing_file_is_absent_but_a_symlink_is_still_refused() {
        let root = tempfile::tempdir().unwrap();
        let missing = root.path().join("missing");
        let found = private_file_if_present(&missing, PrivateFileOptions::default()).unwrap();
        assert!(found.is_none());

        let link = root.path().join("link");
        symlink(&missing, &link).unwrap();
        let result = private_file_if_present(&link, PrivateFileOptions::default());
        assert!(matches!(result, Err(StorageError::Io { .. })));
    }
}
