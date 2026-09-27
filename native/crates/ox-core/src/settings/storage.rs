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

use std::fs::{self, DirBuilder, File, Metadata, OpenOptions, Permissions};
use std::io::{self, Read, Write};
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

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
    /// `private_file` in private_storage.py does.
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
    /// (private_storage.py:private_file): a second link could expose the
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

/// Adds the affected path to an I/O error, as Python's `OSError` does.
trait WithPath<T> {
    fn with_path(self, path: &Path) -> Result<T, SettingsError>;
}

impl<T> WithPath<T> for io::Result<T> {
    fn with_path(self, path: &Path) -> Result<T, SettingsError> {
        self.map_err(|error| SettingsError::io(path, error))
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

/// Seconds since the Unix epoch; 0 if the clock is set before it.
fn unix_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs())
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
