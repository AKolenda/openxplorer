// SPDX-License-Identifier: AGPL-3.0-only
//! "Install updates without asking for a password": the setting that lets
//! an administrator at the machine install `OpenXplorer` updates without
//! typing a password. New in the native app.
//!
//! The setting is on by default. The stable Debian package installs the
//! update helper and its polkit action, whose defaults ask for an
//! administrator (`auth_admin_keep`), and the polkit rule that skips the
//! password as a template, [`RULE_TEMPLATE`], outside the folders polkit
//! reads. Its `postinst` copies that template to [`PASSWORDLESS_RULE`] in
//! `/etc/polkit-1/rules.d` and records the setting as on, on a fresh
//! installation and on every upgrade, unless the record says the user
//! turned it off (`native/packaging/debian/install-update-rule`). Polkit
//! rules cannot read the app's settings, so turning the setting off
//! deletes the copy, and turning it back on copies the template again.
//! Both run as root in the update helper (`openxplorer-install-update
//! --disable-passwordless` or `--enable-passwordless`), started through
//! `pkexec`, so changing the setting asks for an administrator's password
//! once. The package's `postrm` deletes the copy and the record when the
//! package is removed or purged; an upgrade keeps them.
//!
//! The rule's text is always the installed template's, never the
//! caller's: the helper takes no path and no text. It writes only into
//! existing root-owned folders, never through a symbolic link, and
//! replaces the file atomically with mode 0644, owned by root.
//!
//! Ubuntu and Debian keep `/etc/polkit-1/rules.d` unreadable to users, so
//! the helper also writes [`PASSWORDLESS_RECORD`], which anyone may read:
//! `on` or `off`. The record also keeps a user's `off` across upgrades.
//! The app shows the setting on when the rule file is there; only when it
//! cannot look into the rules folder does it go by the record. The helper
//! records `on` before writing the rule and removes the rule before
//! recording `off`, so an interrupted change can never leave the rule
//! active while the switch shows off; after any error it makes the record
//! match the rule as it is. Every change holds the lock installations hold
//! ([`super::STAGING_FOLDER`]), so changes never interleave.

use std::ffi::OsString;
use std::io;
use std::os::fd::OwnedFd;
use std::path::{Path, PathBuf};

use rustix::fs::{AtFlags, FileType, Mode, OFlags};

use super::Installation;

/// The rule template the stable Debian package installs: the rule text,
/// with the action ID filled in, where polkit does not read it.
pub const RULE_TEMPLATE: &str = "/usr/share/openxplorer/install-update.rules";

/// Where turning the setting on puts the rule, and where polkit reads it.
pub const PASSWORDLESS_RULE: &str = "/etc/polkit-1/rules.d/60-io.winspace.Development.install-update.rules";

/// The record of whether the setting is on, which any user may read.
pub const PASSWORDLESS_RECORD: &str = "/var/lib/openxplorer/install-updates-without-password";

/// The helper's argument that turns the setting on.
pub const ENABLE_PASSWORDLESS: &str = "--enable-passwordless";

/// The helper's argument that turns the setting off.
pub const DISABLE_PASSWORDLESS: &str = "--disable-passwordless";

/// The program that starts the helper as root.
const PKEXEC: &str = "/usr/bin/pkexec";

/// The mode of the rule and the record: read by everyone, written by root.
const FILE_MODE: u32 = 0o644;

/// The mode of a folder the helper makes for the record.
const FOLDER_MODE: u32 = 0o755;

/// The largest template the helper copies.
const MAX_TEMPLATE_BYTES: u64 = 64 * 1024;

/// What the record says while the setting is on.
const RECORD_ON: &str = "on\n";

/// What the record says once the user turned the setting off: the
/// package's `postinst` then leaves it off on an upgrade.
const RECORD_OFF: &str = "off\n";

/// Why the setting could not be changed. `Display` is the helper's
/// message on standard error.
#[derive(Debug, thiserror::Error)]
pub enum PasswordlessError {
    /// A file or folder could not be read or written.
    #[error("{}: {error}", path.display())]
    Io {
        /// The file or folder.
        path: PathBuf,
        /// What went wrong.
        error: io::Error,
    },
    /// The template is missing its rule, or is not a regular file owned
    /// by root that only root may change.
    #[error("{} is not the rule the package installed.", path.display())]
    UntrustedTemplate {
        /// The template.
        path: PathBuf,
    },
    /// Something other than a regular file, such as a symbolic link, is
    /// where the rule or the record goes.
    #[error("{} is not a regular file; nothing was changed.", path.display())]
    NotARegularFile {
        /// The rule or the record.
        path: PathBuf,
    },
    /// Another update helper is installing an update or changing the
    /// setting; nothing was changed.
    #[error(
        "Another OpenXplorer update helper is running. Nothing was changed; try again when it has finished."
    )]
    Busy,
    /// The lock could not be taken.
    #[error("{}: {reason}", path.display())]
    Lock {
        /// The lock's folder.
        path: PathBuf,
        /// Why.
        reason: String,
    },
}

impl PasswordlessError {
    fn io(path: &Path, error: impl Into<io::Error>) -> Self {
        Self::Io {
            path: path.to_owned(),
            error: error.into(),
        }
    }
}

/// The files the setting reads and writes, and who must own them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuleFiles {
    /// The package's template, [`RULE_TEMPLATE`].
    pub template: PathBuf,
    /// The active rule, [`PASSWORDLESS_RULE`].
    pub rule: PathBuf,
    /// The record, [`PASSWORDLESS_RECORD`].
    pub record: PathBuf,
    /// The folder whose lock every change of the setting and every
    /// installation holds, [`super::STAGING_FOLDER`], so they run one at a
    /// time.
    pub lock: PathBuf,
    /// The user and group that must own the template, and that own what
    /// the helper writes: root's, except in tests.
    pub owner: (u32, u32),
}

impl RuleFiles {
    /// The files of this computer, owned by root.
    pub fn system() -> Self {
        Self::under(Path::new("/"), (0, 0))
    }

    /// The same files under `filesystem_root`, owned by `owner`, for
    /// tests that build a fake system in a temporary folder.
    pub fn under(filesystem_root: &Path, owner: (u32, u32)) -> Self {
        let inside = |path: &str| filesystem_root.join(path.trim_start_matches('/'));
        Self {
            template: inside(RULE_TEMPLATE),
            rule: inside(PASSWORDLESS_RULE),
            record: inside(PASSWORDLESS_RECORD),
            lock: inside(super::STAGING_FOLDER),
            owner,
        }
    }

    /// Whether the setting is on: the rule file is there. Where the rules
    /// folder cannot be read, as on Ubuntu and Debian for anyone but
    /// root, the record says.
    pub fn is_on(&self) -> bool {
        match std::fs::symlink_metadata(&self.rule) {
            Ok(metadata) => metadata.is_file(),
            Err(error) if error.kind() == io::ErrorKind::PermissionDenied => {
                std::fs::symlink_metadata(&self.record).is_ok_and(|metadata| metadata.is_file())
                    && std::fs::read(&self.record).is_ok_and(|text| text == RECORD_ON.as_bytes())
            }
            Err(_) => false,
        }
    }
}

/// The setting as the app sees it: where it is, and how to change it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PasswordlessUpdates {
    /// The program that starts the helper as root, [`PKEXEC`].
    launcher: PathBuf,
    /// The installed update helper.
    helper: PathBuf,
    /// The files the helper reads and writes.
    files: RuleFiles,
}

impl PasswordlessUpdates {
    /// The setting of `installation` in `filesystem_root`, if it has one:
    /// only the stable Debian package with its update helper and the rule
    /// template does. `filesystem_root` is `/` except in tests.
    pub fn detect(installation: Installation, filesystem_root: &Path) -> Option<Self> {
        let helper = installation.install_helper(filesystem_root)?;
        let files = RuleFiles::under(filesystem_root, (0, 0));
        std::fs::symlink_metadata(&files.template)
            .is_ok_and(|metadata| metadata.is_file())
            .then(|| Self {
                launcher: PathBuf::from(PKEXEC),
                helper,
                files,
            })
    }

    /// The same setting, started through `launcher` instead of `pkexec`,
    /// for tests that must not ask for a password.
    #[must_use]
    pub fn with_launcher(mut self, launcher: PathBuf) -> Self {
        self.launcher = launcher;
        self
    }

    /// Whether the setting is on now ([`RuleFiles::is_on`]).
    pub fn is_on(&self) -> bool {
        self.files.is_on()
    }

    /// The files the setting reads.
    pub fn files(&self) -> &RuleFiles {
        &self.files
    }

    /// The command that turns the setting `on` or off:
    /// `pkexec <helper> --enable-passwordless` or `--disable-passwordless`.
    pub fn command(&self, on: bool) -> [OsString; 3] {
        let argument = if on {
            ENABLE_PASSWORDLESS
        } else {
            DISABLE_PASSWORDLESS
        };
        [
            self.launcher.clone().into_os_string(),
            self.helper.clone().into_os_string(),
            OsString::from(argument),
        ]
    }
}

/// Turns the setting on: records `on`, then copies the template to the
/// rule, holding the lock installations hold. Doing it again changes
/// nothing.
///
/// # Errors
///
/// [`PasswordlessError::UntrustedTemplate`] for a template that is not
/// root's, [`PasswordlessError::NotARegularFile`] where a link or anything
/// else but a file is in the way, and the file system's errors. Nothing is
/// left half-done: after any error the record is made to match the rule
/// as it is, and [`PasswordlessError::Busy`] while another helper runs.
pub fn enable_passwordless(files: &RuleFiles) -> Result<(), PasswordlessError> {
    enable_with_checkpoint(files, &mut |_| Fault::None)
}

/// A point in a change of the setting where a test can make something go
/// wrong.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Step {
    /// Turning it on recorded `on`.
    RecordWritten,
    /// Turning it on renamed the rule into place.
    RuleWritten,
    /// Turning it off removed the rule.
    RuleRemoved,
}

/// What goes wrong at a [`Step`], in a test.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(not(test), expect(dead_code, reason = "only tests make something go wrong"))]
enum Fault {
    /// Nothing.
    None,
    /// The helper stops there, as when it is killed or the computer loses
    /// power: nothing after it runs.
    Stop,
    /// The step reports an error although its change was made, as when
    /// syncing the folder fails after a rename.
    Fail,
}

/// The error a [`Fault::Fail`] at `step` reports for `path`.
fn simulated_failure(path: &Path) -> PasswordlessError {
    PasswordlessError::io(path, io::Error::other("simulated failure"))
}

/// Takes the lock installations hold too, without waiting.
fn lock(files: &RuleFiles) -> Result<std::fs::File, PasswordlessError> {
    super::install_helper::lock_staging_folder(&files.lock).map_err(|error| match error {
        super::UpdateError::TaskRunning => PasswordlessError::Busy,
        error => PasswordlessError::Lock {
            path: files.lock.clone(),
            reason: error.to_string(),
        },
    })
}

/// After a change failed part-way, makes the record match the rule as it
/// is now: `on` whenever the rule is there, so the switch never shows OFF
/// while the rule is active, and `off` when it is not.
fn match_record_to_rule(files: &RuleFiles) {
    let Ok(rule_target) = Target::open(&files.rule, false) else {
        return;
    };
    let text = match rule_target.exists() {
        Ok(true) => RECORD_ON,
        Ok(false) => RECORD_OFF,
        // Something else is in the rule's place; it is not this setting's.
        Err(_) => return,
    };
    if let Ok(record_target) = Target::open(&files.record, true) {
        if record_target.exists().is_ok() {
            let _ = record_target.replace(text.as_bytes(), files.owner);
        }
    }
}

/// [`enable_passwordless`], which asks `checkpoint` after each step
/// whether something goes wrong there, as a test simulates.
fn enable_with_checkpoint(
    files: &RuleFiles,
    checkpoint: &mut dyn FnMut(Step) -> Fault,
) -> Result<(), PasswordlessError> {
    let _only_change = lock(files)?;
    let changed = enable_steps(files, checkpoint);
    if changed.is_err() {
        match_record_to_rule(files);
    }
    changed
}

/// The steps of [`enable_with_checkpoint`], under its lock.
fn enable_steps(
    files: &RuleFiles,
    checkpoint: &mut dyn FnMut(Step) -> Fault,
) -> Result<(), PasswordlessError> {
    let rule = read_template(files)?;
    let rule_target = Target::open(&files.rule, true)?;
    let record_target = Target::open(&files.record, true)?;
    rule_target.refuse_anything_but_a_file()?;
    record_target.refuse_anything_but_a_file()?;
    // The record first, synced, then the rule: an interruption between
    // them can only leave the switch showing ON with no rule, never an
    // active rule the app shows as OFF where it reads only the record.
    record_target.replace(RECORD_ON.as_bytes(), files.owner)?;
    match checkpoint(Step::RecordWritten) {
        Fault::Stop => return Ok(()),
        Fault::Fail => return Err(simulated_failure(&files.record)),
        Fault::None => {}
    }
    let written = rule_target.replace(rule.as_bytes(), files.owner).and_then(|()| {
        match checkpoint(Step::RuleWritten) {
            Fault::Fail => Err(simulated_failure(&files.rule)),
            Fault::Stop | Fault::None => Ok(()),
        }
    });
    // On an error the caller makes the record match the rule as it is: a
    // rename can have gone through although the step failed.
    written
}

/// Turns the setting off: deletes the rule, then records `off`, so an
/// upgrade keeps it off, holding the lock installations hold. Doing it
/// again changes nothing.
///
/// # Errors
///
/// [`PasswordlessError::NotARegularFile`] where a link or anything else
/// but a file is in the way, [`PasswordlessError::Busy`] while another
/// helper runs, and the file system's errors; after any error the record
/// is made to match the rule as it is.
pub fn disable_passwordless(files: &RuleFiles) -> Result<(), PasswordlessError> {
    disable_with_checkpoint(files, &mut |_| Fault::None)
}

/// [`disable_passwordless`], which asks `checkpoint` after each step
/// whether something goes wrong there, as a test simulates.
fn disable_with_checkpoint(
    files: &RuleFiles,
    checkpoint: &mut dyn FnMut(Step) -> Fault,
) -> Result<(), PasswordlessError> {
    let _only_change = lock(files)?;
    let changed = disable_steps(files, checkpoint);
    if changed.is_err() {
        match_record_to_rule(files);
    }
    changed
}

/// The steps of [`disable_with_checkpoint`], under its lock.
fn disable_steps(
    files: &RuleFiles,
    checkpoint: &mut dyn FnMut(Step) -> Fault,
) -> Result<(), PasswordlessError> {
    let rule_target = Target::open(&files.rule, false)?;
    let record_target = Target::open(&files.record, true)?;
    for target in [&rule_target, &record_target] {
        target.refuse_anything_but_a_file()?;
    }
    // The rule first, then the record, for the same reason.
    rule_target.remove()?;
    match checkpoint(Step::RuleRemoved) {
        Fault::Stop => return Ok(()),
        Fault::Fail => return Err(simulated_failure(&files.rule)),
        Fault::None => {}
    }
    record_target.replace(RECORD_OFF.as_bytes(), files.owner)
}

/// Reads the template, which must be a regular file of `files.owner`
/// that nobody else may change, holding one polkit rule.
fn read_template(files: &RuleFiles) -> Result<String, PasswordlessError> {
    use std::io::Read;
    let path = &files.template;
    let untrusted = || PasswordlessError::UntrustedTemplate { path: path.clone() };
    let file = rustix::fs::open(
        path,
        OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC | OFlags::NOCTTY,
        Mode::empty(),
    )
    .map_err(|error| match error {
        rustix::io::Errno::LOOP => untrusted(),
        error => PasswordlessError::io(path, error),
    })?;
    let stat = rustix::fs::fstat(&file).map_err(|error| PasswordlessError::io(path, error))?;
    let is_owners = stat.st_uid == files.owner.0 && stat.st_mode & 0o022 == 0;
    let is_file = FileType::from_raw_mode(stat.st_mode) == FileType::RegularFile;
    if !is_file || !is_owners || u64::try_from(stat.st_size).unwrap_or(u64::MAX) > MAX_TEMPLATE_BYTES {
        return Err(untrusted());
    }
    let mut text = String::new();
    std::fs::File::from(file)
        .take(MAX_TEMPLATE_BYTES)
        .read_to_string(&mut text)
        .map_err(|_| untrusted())?;
    if !text.contains("polkit.addRule(") || text.contains("@ACTION_ID@") {
        return Err(untrusted());
    }
    Ok(text)
}

/// A file the helper writes or deletes, by its folder, opened without
/// following a link, and its name in it.
struct Target {
    /// The whole path, for messages.
    path: PathBuf,
    /// The open folder, or `None` when it does not exist and is not
    /// made.
    folder: Option<OwnedFd>,
    /// The folder's path.
    folder_path: PathBuf,
    /// The file's name in the folder.
    name: OsString,
}

impl Target {
    /// Opens the folder of `path`, making it (only its last part) when
    /// `make` is set and it is missing.
    fn open(path: &Path, make: bool) -> Result<Self, PasswordlessError> {
        let (Some(folder_path), Some(name)) = (path.parent(), path.file_name()) else {
            return Err(PasswordlessError::NotARegularFile {
                path: path.to_owned(),
            });
        };
        let folder = match open_folder(folder_path) {
            Ok(folder) => Some(folder),
            Err(rustix::io::Errno::NOENT) if make => {
                make_folder(folder_path)?;
                Some(open_folder(folder_path).map_err(|error| PasswordlessError::io(folder_path, error))?)
            }
            Err(rustix::io::Errno::NOENT) => None,
            Err(rustix::io::Errno::NOTDIR | rustix::io::Errno::LOOP) => {
                return Err(PasswordlessError::NotARegularFile {
                    path: folder_path.to_owned(),
                })
            }
            Err(error) => return Err(PasswordlessError::io(folder_path, error)),
        };
        Ok(Self {
            path: path.to_owned(),
            folder,
            folder_path: folder_path.to_owned(),
            name: name.to_owned(),
        })
    }

    /// Whether a file is there, refusing anything else, a link included.
    fn exists(&self) -> Result<bool, PasswordlessError> {
        let Some(folder) = &self.folder else {
            return Ok(false);
        };
        match rustix::fs::statat(folder, self.name.as_os_str(), AtFlags::SYMLINK_NOFOLLOW) {
            Ok(stat) if FileType::from_raw_mode(stat.st_mode) == FileType::RegularFile => Ok(true),
            Ok(_) => Err(PasswordlessError::NotARegularFile {
                path: self.path.clone(),
            }),
            Err(rustix::io::Errno::NOENT) => Ok(false),
            Err(error) => Err(PasswordlessError::io(&self.path, error)),
        }
    }

    fn refuse_anything_but_a_file(&self) -> Result<(), PasswordlessError> {
        self.exists().map(|_| ())
    }

    /// Replaces the file with `contents`, owned by `owner` with mode
    /// 0644: written to a new file beside it, synced and renamed over it.
    fn replace(&self, contents: &[u8], owner: (u32, u32)) -> Result<(), PasswordlessError> {
        use std::io::Write;
        let Some(folder) = &self.folder else {
            return Err(PasswordlessError::io(
                &self.folder_path,
                io::Error::from(io::ErrorKind::NotFound),
            ));
        };
        let mut temporary = OsString::from(".");
        temporary.push(&self.name);
        temporary.push(format!(".{}.new", std::process::id()));
        let failed = |error: rustix::io::Errno| PasswordlessError::io(&self.path, error);
        // A file a killed helper left behind.
        let _ = rustix::fs::unlinkat(folder, temporary.as_os_str(), AtFlags::empty());
        let file = rustix::fs::openat(
            folder,
            temporary.as_os_str(),
            OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::from_raw_mode(0o600),
        )
        .map_err(failed)?;
        let written = (|| {
            rustix::fs::fchown(
                &file,
                Some(rustix::fs::Uid::from_raw(owner.0)),
                Some(rustix::fs::Gid::from_raw(owner.1)),
            )?;
            rustix::fs::fchmod(&file, Mode::from_raw_mode(FILE_MODE))?;
            let mut file = std::fs::File::from(file);
            file.write_all(contents)
                .map_err(|error| rustix::io::Errno::from_io_error(&error).unwrap_or(rustix::io::Errno::IO))?;
            rustix::fs::fsync(&file)?;
            rustix::fs::renameat(folder, temporary.as_os_str(), folder, self.name.as_os_str())?;
            rustix::fs::fsync(folder)
        })();
        if let Err(error) = written {
            let _ = rustix::fs::unlinkat(folder, temporary.as_os_str(), AtFlags::empty());
            return Err(failed(error));
        }
        Ok(())
    }

    /// Deletes the file, if it is there.
    fn remove(&self) -> Result<(), PasswordlessError> {
        let Some(folder) = &self.folder else {
            return Ok(());
        };
        match rustix::fs::unlinkat(folder, self.name.as_os_str(), AtFlags::empty()) {
            Ok(()) | Err(rustix::io::Errno::NOENT) => {
                rustix::fs::fsync(folder).map_err(|error| PasswordlessError::io(&self.folder_path, error))
            }
            Err(error) => Err(PasswordlessError::io(&self.path, error)),
        }
    }
}

/// Opens `path` as a folder, refusing a link.
fn open_folder(path: &Path) -> rustix::io::Result<OwnedFd> {
    rustix::fs::open(
        path,
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    )
}

/// Makes the last part of `path` a folder with mode 0755 inside its
/// existing parent, opened without following a link, and syncs the parent
/// so the new folder outlasts a power loss along with what goes in it.
fn make_folder(path: &Path) -> Result<(), PasswordlessError> {
    let (Some(parent), Some(name)) = (path.parent(), path.file_name()) else {
        return Err(PasswordlessError::NotARegularFile {
            path: path.to_owned(),
        });
    };
    let parent_folder = open_folder(parent).map_err(|error| PasswordlessError::io(parent, error))?;
    match rustix::fs::mkdirat(&parent_folder, name, Mode::from_raw_mode(FOLDER_MODE)) {
        Ok(()) => rustix::fs::fsync(&parent_folder).map_err(|error| PasswordlessError::io(parent, error)),
        Err(rustix::io::Errno::EXIST) => Ok(()),
        Err(error) => Err(PasswordlessError::io(path, error)),
    }
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};

    use super::*;

    const RULE: &str = "// fixture\npolkit.addRule(function (action, subject) {\n    return \
                        polkit.Result.NOT_HANDLED;\n});\n";

    /// A fake system with the package's template, owned by the test
    /// user as the real one is owned by root.
    struct FakeSystem {
        _root: tempfile::TempDir,
        files: RuleFiles,
    }

    impl FakeSystem {
        fn new() -> Self {
            let root = tempfile::tempdir().unwrap();
            let owner = (
                rustix::process::geteuid().as_raw(),
                rustix::process::getegid().as_raw(),
            );
            let files = RuleFiles::under(root.path(), owner);
            for folder in [
                "usr/share/openxplorer",
                "etc/polkit-1/rules.d",
                "var/lib",
                "var/cache",
            ] {
                std::fs::create_dir_all(root.path().join(folder)).unwrap();
            }
            std::fs::write(&files.template, RULE).unwrap();
            std::fs::set_permissions(&files.template, std::fs::Permissions::from_mode(0o644)).unwrap();
            Self { _root: root, files }
        }

        fn rules_folder(&self) -> &Path {
            self.files.rule.parent().unwrap()
        }
    }

    fn mode(path: &Path) -> u32 {
        std::fs::symlink_metadata(path).unwrap().permissions().mode() & 0o7777
    }

    fn record(files: &RuleFiles) -> String {
        std::fs::read_to_string(&files.record).unwrap()
    }

    /// Without the rule the setting is off; the package's postinst is what
    /// turns it on by default (tested in native/tools/test_build_deb.py).
    #[test]
    fn without_the_rule_the_setting_is_off() {
        let system = FakeSystem::new();

        assert!(!system.files.is_on());
        assert!(!system.files.rule.exists());
    }

    /// Turning it on writes exactly the template, mode 0644, owned by the
    /// owner, and the record; the state follows the file.
    /// parity: UPD-003
    #[test]
    fn turning_it_on_writes_exactly_the_template() {
        let system = FakeSystem::new();

        enable_passwordless(&system.files).unwrap();

        assert_eq!(std::fs::read_to_string(&system.files.rule).unwrap(), RULE);
        assert_eq!(mode(&system.files.rule), 0o644);
        assert_eq!(mode(&system.files.record), 0o644);
        let metadata = std::fs::metadata(&system.files.rule).unwrap();
        assert_eq!((metadata.uid(), metadata.gid()), system.files.owner);
        assert!(system.files.is_on());
        let leftovers: Vec<_> = std::fs::read_dir(system.rules_folder())
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        assert_eq!(
            leftovers,
            [OsString::from("60-io.winspace.Development.install-update.rules")]
        );
    }

    #[test]
    fn turning_it_on_twice_is_fine() {
        let system = FakeSystem::new();

        enable_passwordless(&system.files).unwrap();
        enable_passwordless(&system.files).unwrap();

        assert_eq!(std::fs::read_to_string(&system.files.rule).unwrap(), RULE);
        assert!(system.files.is_on());
    }

    /// Turning it off deletes the rule and records `off`, which the
    /// package's postinst keeps on an upgrade; turning it off again is
    /// fine.
    /// parity: UPD-003
    #[test]
    fn turning_it_off_removes_the_rule() {
        let system = FakeSystem::new();
        enable_passwordless(&system.files).unwrap();

        disable_passwordless(&system.files).unwrap();
        disable_passwordless(&system.files).unwrap();

        assert!(!system.files.rule.exists());
        assert_eq!(record(&system.files), RECORD_OFF);
        assert_eq!(mode(&system.files.record), 0o644);
        assert!(system.rules_folder().is_dir());
        assert!(!system.files.is_on());
    }

    /// Turning it off on a system that never had a record records `off`
    /// all the same.
    #[test]
    fn turning_it_off_without_a_record_records_off() {
        let system = FakeSystem::new();

        disable_passwordless(&system.files).unwrap();

        assert_eq!(record(&system.files), RECORD_OFF);
    }

    /// Whether the switch can show OFF while the rule is active: the rule
    /// is there, and the app, which on Ubuntu reads only the record, does
    /// not find `on` in it.
    fn shows_off_while_active(files: &RuleFiles) -> bool {
        files.rule.exists() && std::fs::read_to_string(&files.record).ok().as_deref() != Some(RECORD_ON)
    }

    /// A helper killed, or a computer that loses power, between the two
    /// writes of turning the setting on never leaves the rule active
    /// with the switch showing OFF: the record is written first.
    /// parity: UPD-003
    #[test]
    fn an_interrupted_enable_never_hides_an_active_rule() {
        let system = FakeSystem::new();

        enable_with_checkpoint(&system.files, &mut |_| Fault::Stop).unwrap();

        assert!(!shows_off_while_active(&system.files));
        assert_eq!(record(&system.files), RECORD_ON, "the record is written first");
        assert!(!system.files.rule.exists());
        enable_passwordless(&system.files).unwrap();
        assert!(system.files.rule.exists());
        assert_eq!(record(&system.files), RECORD_ON);
    }

    /// The same for turning it off: the rule goes first, so an
    /// interruption can only leave the switch showing ON with the rule
    /// already gone, and turning it off again finishes.
    /// parity: UPD-003
    #[test]
    fn an_interrupted_disable_never_hides_an_active_rule() {
        let system = FakeSystem::new();
        enable_passwordless(&system.files).unwrap();

        disable_with_checkpoint(&system.files, &mut |_| Fault::Stop).unwrap();

        assert!(!shows_off_while_active(&system.files));
        assert!(!system.files.rule.exists(), "the rule is removed first");
        assert_eq!(record(&system.files), RECORD_ON);
        disable_passwordless(&system.files).unwrap();
        assert!(!system.files.rule.exists());
        assert_eq!(record(&system.files), RECORD_OFF);
    }

    /// A rule whose rename went through although the step reported an
    /// error, as when syncing the folder fails afterwards, is live: its
    /// record stays, so the switch does not show OFF.
    /// parity: UPD-003
    #[test]
    fn a_rule_whose_rename_committed_keeps_its_record() {
        let system = FakeSystem::new();

        let enabled = enable_with_checkpoint(&system.files, &mut |step| {
            if step == Step::RuleWritten {
                Fault::Fail
            } else {
                Fault::None
            }
        });

        assert!(enabled.is_err());
        assert!(system.files.rule.exists(), "the rename went through");
        assert!(!shows_off_while_active(&system.files));
    }

    /// Changes run one at a time, and never beside an installation: while
    /// another helper holds the lock, turning the setting on or off is
    /// refused before anything is written, so two administrators changing
    /// it at once cannot interleave their steps.
    /// parity: UPD-003
    #[test]
    fn a_change_is_refused_while_another_helper_holds_the_lock() {
        let system = FakeSystem::new();
        let held = super::super::install_helper::lock_staging_folder(&system.files.lock).unwrap();

        let enabled = enable_passwordless(&system.files);
        assert!(matches!(enabled, Err(PasswordlessError::Busy)), "{enabled:?}");
        assert!(!system.files.rule.exists() && !system.files.record.exists());

        drop(held);
        enable_passwordless(&system.files).unwrap();
        let held = super::super::install_helper::lock_staging_folder(&system.files.lock).unwrap();
        let disabled = disable_passwordless(&system.files);
        assert!(matches!(disabled, Err(PasswordlessError::Busy)), "{disabled:?}");
        assert!(system.files.rule.exists());
        assert_eq!(record(&system.files), RECORD_ON);
        drop(held);
        disable_passwordless(&system.files).unwrap();
        assert!(!system.files.rule.exists());
    }

    /// A rule that cannot be removed keeps its record.
    #[test]
    fn a_failed_disable_keeps_the_record_while_the_rule_remains() {
        if rustix::process::geteuid().is_root() {
            return;
        }
        let system = FakeSystem::new();
        enable_passwordless(&system.files).unwrap();
        let folder = system.rules_folder();
        std::fs::set_permissions(folder, std::fs::Permissions::from_mode(0o555)).unwrap();

        let disabled = disable_passwordless(&system.files);
        std::fs::set_permissions(folder, std::fs::Permissions::from_mode(0o755)).unwrap();

        assert!(disabled.is_err());
        assert!(system.files.rule.exists());
        assert!(!shows_off_while_active(&system.files));
    }

    /// A rule that cannot be written sets the record back to `off`, so
    /// the switch does not show ON for nothing.
    #[test]
    fn a_rule_that_cannot_be_written_removes_the_record_again() {
        if rustix::process::geteuid().is_root() {
            // Root writes into any folder.
            return;
        }
        let system = FakeSystem::new();
        let folder = system.rules_folder();
        std::fs::set_permissions(folder, std::fs::Permissions::from_mode(0o555)).unwrap();

        let enabled = enable_passwordless(&system.files);
        std::fs::set_permissions(folder, std::fs::Permissions::from_mode(0o755)).unwrap();

        assert!(enabled.is_err());
        assert!(!system.files.rule.exists());
        assert_eq!(record(&system.files), RECORD_OFF);
    }

    /// A link where the rule goes is never followed, and nothing changes.
    #[test]
    fn a_link_at_the_rule_is_refused() {
        let system = FakeSystem::new();
        let elsewhere = system.rules_folder().parent().unwrap().join("elsewhere");
        std::fs::write(&elsewhere, "kept").unwrap();
        std::os::unix::fs::symlink(&elsewhere, &system.files.rule).unwrap();

        let enabled = enable_passwordless(&system.files);
        let disabled = disable_passwordless(&system.files);

        assert!(
            matches!(enabled, Err(PasswordlessError::NotARegularFile { .. })),
            "{enabled:?}"
        );
        assert!(
            matches!(disabled, Err(PasswordlessError::NotARegularFile { .. })),
            "{disabled:?}"
        );
        assert_eq!(std::fs::read_to_string(&elsewhere).unwrap(), "kept");
        assert!(std::fs::symlink_metadata(&system.files.rule)
            .unwrap()
            .is_symlink());
        assert!(!system.files.record.exists());
        assert!(!system.files.is_on(), "a link is not the rule");
    }

    #[test]
    fn a_link_at_the_record_is_refused_before_the_rule_is_written() {
        let system = FakeSystem::new();
        let record_folder = system.files.record.parent().unwrap();
        std::fs::create_dir_all(record_folder).unwrap();
        std::os::unix::fs::symlink("/dev/null", &system.files.record).unwrap();

        let enabled = enable_passwordless(&system.files);

        assert!(
            matches!(enabled, Err(PasswordlessError::NotARegularFile { .. })),
            "{enabled:?}"
        );
        assert!(!system.files.rule.exists());
    }

    /// The rule's text is only ever the package's: a template that is a
    /// link, that others may change, or that holds no rule is refused.
    #[test]
    fn only_the_packages_template_is_copied() {
        let changes: [&dyn Fn(&RuleFiles); 4] = [
            &|files| {
                std::fs::set_permissions(&files.template, std::fs::Permissions::from_mode(0o666)).unwrap();
            },
            &|files| {
                std::fs::remove_file(&files.template).unwrap();
                std::os::unix::fs::symlink("/etc/hostname", &files.template).unwrap();
            },
            &|files| std::fs::write(&files.template, "polkit.addRule(@ACTION_ID@)").unwrap(),
            &|files| std::fs::write(&files.template, "not a rule").unwrap(),
        ];
        for (index, change) in changes.iter().enumerate() {
            let system = FakeSystem::new();
            change(&system.files);

            let enabled = enable_passwordless(&system.files);

            assert!(
                matches!(enabled, Err(PasswordlessError::UntrustedTemplate { .. })),
                "{index}: {enabled:?}"
            );
            assert!(!system.files.rule.exists(), "{index}");
        }
    }

    #[test]
    fn a_template_of_another_owner_is_refused() {
        let system = FakeSystem::new();
        let files = RuleFiles {
            owner: (system.files.owner.0 + 1, system.files.owner.1),
            ..system.files.clone()
        };

        let enabled = enable_passwordless(&files);

        assert!(
            matches!(enabled, Err(PasswordlessError::UntrustedTemplate { .. })),
            "{enabled:?}"
        );
        assert!(!files.rule.exists());
    }

    /// Where the rules folder cannot be read, as for a user on Ubuntu,
    /// the record says whether the setting is on.
    #[test]
    fn an_unreadable_rules_folder_is_read_through_the_record() {
        if rustix::process::geteuid().is_root() {
            // Root reads every folder.
            return;
        }
        let system = FakeSystem::new();
        enable_passwordless(&system.files).unwrap();
        let folder = system.rules_folder();
        std::fs::set_permissions(folder, std::fs::Permissions::from_mode(0o700)).unwrap();
        std::fs::set_permissions(folder, std::fs::Permissions::from_mode(0o000)).unwrap();

        let with_record = system.files.is_on();
        std::fs::write(&system.files.record, RECORD_OFF).unwrap();
        let recorded_off = system.files.is_on();
        std::fs::remove_file(&system.files.record).unwrap();
        let without_record = system.files.is_on();
        std::fs::set_permissions(folder, std::fs::Permissions::from_mode(0o755)).unwrap();

        assert!(with_record);
        assert!(!recorded_off);
        assert!(!without_record);
    }

    /// A record without the rule, in a folder the app can read, is not
    /// the setting on: the rule file decides.
    #[test]
    fn a_readable_rules_folder_decides_over_the_record() {
        let system = FakeSystem::new();
        enable_passwordless(&system.files).unwrap();

        std::fs::remove_file(&system.files.rule).unwrap();

        assert!(system.files.record.exists());
        assert!(!system.files.is_on());
    }

    /// Only the stable Debian package with the helper and the template
    /// has the setting, and it runs only `pkexec <helper> <flag>`.
    #[test]
    fn the_setting_exists_only_where_the_package_installed_the_helper_and_template() {
        let root = tempfile::tempdir().unwrap();
        let helper = root
            .path()
            .join("opt/openxplorer/libexec/openxplorer-install-update");
        std::fs::create_dir_all(helper.parent().unwrap()).unwrap();
        std::fs::write(&helper, b"fixture").unwrap();
        std::fs::set_permissions(&helper, std::fs::Permissions::from_mode(0o755)).unwrap();

        assert_eq!(
            PasswordlessUpdates::detect(Installation::DebianPackage, root.path()),
            None
        );
        let template = root.path().join("usr/share/openxplorer/install-update.rules");
        std::fs::create_dir_all(template.parent().unwrap()).unwrap();
        std::fs::write(&template, RULE).unwrap();

        let setting = PasswordlessUpdates::detect(Installation::DebianPackage, root.path()).unwrap();
        for installation in [
            Installation::DebianPackageWithoutTools,
            Installation::Flatpak,
            Installation::OtherPackage,
            Installation::Unpackaged,
        ] {
            assert_eq!(
                PasswordlessUpdates::detect(installation, root.path()),
                None,
                "{installation:?}"
            );
        }
        assert_eq!(
            setting.command(true),
            [
                OsString::from("/usr/bin/pkexec"),
                helper.clone().into_os_string(),
                OsString::from("--enable-passwordless")
            ]
        );
        assert_eq!(setting.command(false)[2], "--disable-passwordless");
        assert_eq!(setting.files().rule, root.path().join(&PASSWORDLESS_RULE[1..]));
        assert!(!setting.is_on());
    }

    #[test]
    fn the_system_files_are_the_documented_ones() {
        let files = RuleFiles::system();

        assert_eq!(
            files.template,
            Path::new("/usr/share/openxplorer/install-update.rules")
        );
        assert_eq!(
            files.rule,
            Path::new("/etc/polkit-1/rules.d/60-io.winspace.Development.install-update.rules")
        );
        assert_eq!(
            files.record,
            Path::new("/var/lib/openxplorer/install-updates-without-password")
        );
        assert_eq!(files.lock, Path::new("/var/cache/openxplorer-update"));
        assert_eq!(files.owner, (0, 0));
    }
}
