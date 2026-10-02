// SPDX-License-Identifier: AGPL-3.0-only
//! The opt-in that sends other applications' Open and Save dialogs to the
//! app: one preference in the user's desktop-portal configuration.
//!
//! New in the native app (INT-032). The desktop portal reads
//! `$XDG_CONFIG_HOME/xdg-desktop-portal/<desktop>-portals.conf` before the
//! system's files, and older portals use only the first file they find.
//! Enabling therefore changes the user's file the portal reads now (a
//! desktop's file or `portals.conf`), or, when the user has none, creates
//! the first one as a copy of the system file for the desktop, with one
//! key set under `[preferred]`:
//!
//! ```ini
//! org.freedesktop.impl.portal.FileChooser=io.winspace.Development
//! ```
//!
//! where the value names the packaged `<app id>.portal` file. Every other
//! preference stays as it was, so screenshots, screen sharing and the rest
//! keep their backends.
//!
//! The rules of the other integrations hold: nothing is written until the
//! user asks; the file's previous contents are recorded before the first
//! change and put back by disabling; a symlink is never replaced; and if
//! the user edited the file afterwards, disabling removes only the app's
//! line and keeps the rest. Inside Flatpak the host's portal configuration
//! is out of reach, so the feature is unavailable there.
//!
//! The portal reads its configuration when it starts, so a change applies
//! at the next login, or at once after [`PORTAL_SERVICE`] restarts.

use std::fs;
use std::future::Future;
use std::io;
use std::path::{Path, PathBuf};

use super::private_file::write_private_file;
use super::sandbox::Sandbox;
use super::worker::on_worker;

/// The portal interface whose backend the opt-in chooses.
pub const FILE_CHOOSER_KEY: &str = "org.freedesktop.impl.portal.FileChooser";

/// The user's systemd unit of the desktop portal, which reads the
/// configuration when it starts.
pub const PORTAL_SERVICE: &str = "xdg-desktop-portal.service";

/// The comment above the app's line, which says who set it.
const LINE_COMMENT: &str = "# Open and Save dialogs: set by OpenXplorer (Settings > Default apps).";

/// The configuration folder below each base folder.
const PORTAL_SUBDIR: &str = "xdg-desktop-portal";

/// The record of what the file held before, in the settings folder.
const RECORD_FILE_NAME: &str = "file-dialogs.json";

/// The section that holds the preferences.
const PREFERRED_SECTION: &str = "[preferred]";

/// The folders and the desktop the opt-in works with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileDialogPaths {
    /// The settings folder (`~/.config/winspace`), which keeps the record.
    pub settings: PathBuf,
    /// The user's configuration folder (`$XDG_CONFIG_HOME`).
    pub config_home: PathBuf,
    /// The system configuration and data folders the portal reads after
    /// the user's, in its order (`$XDG_CONFIG_DIRS`, `/etc`,
    /// `$XDG_DATA_DIRS`, `/usr/share`).
    pub system_dirs: Vec<PathBuf>,
    /// The current desktops, lowercase, from `XDG_CURRENT_DESKTOP`.
    pub desktops: Vec<String>,
}

impl FileDialogPaths {
    /// The current user's folders and desktop, with the record in
    /// `settings`.
    pub fn for_user(settings: &Path) -> Self {
        let mut system_dirs: Vec<PathBuf> = glib::system_config_dirs();
        system_dirs.push(PathBuf::from("/etc"));
        system_dirs.extend(glib::system_data_dirs());
        system_dirs.push(PathBuf::from("/usr/share"));
        Self {
            settings: settings.to_owned(),
            config_home: glib::user_config_dir(),
            system_dirs,
            desktops: desktops_from(std::env::var("XDG_CURRENT_DESKTOP").ok().as_deref()),
        }
    }
}

/// The desktops of an `XDG_CURRENT_DESKTOP` value, lowercase, skipping
/// names the portal would not accept.
pub fn desktops_from(value: Option<&str>) -> Vec<String> {
    value
        .unwrap_or_default()
        .split(':')
        .filter(|name| {
            !name.is_empty()
                && name
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || "-_".contains(c))
        })
        .map(str::to_ascii_lowercase)
        .collect()
}

/// Why the opt-in could not be changed. `Display` is the message the
/// Settings page shows.
#[derive(Debug, thiserror::Error)]
pub enum FileDialogError {
    /// Inside Flatpak the host's portal configuration cannot be written.
    #[error("Open and Save dialogs can be changed only in the installed package, not the Flatpak.")]
    Unsupported,
    /// The configuration file is a symlink, which is never replaced.
    #[error("Refusing to replace a symlink: {}", .0.display())]
    Symlink(PathBuf),
    /// Reading, writing or removing a file failed.
    #[error("{error}: {}", path.display())]
    Io {
        /// The file the operation was on.
        path: PathBuf,
        /// What the operating system reported.
        error: io::Error,
    },
}

/// What disabling did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DisabledFileDialogs {
    /// The file is back to what it held before.
    Restored,
    /// The user changed the file since; only the app's line was removed.
    LineRemoved,
    /// The app's line was not there; nothing changed.
    NotEnabled,
}

/// What asking the portal to read its configuration again did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PortalRestart {
    /// The portal was running and restarted; the choice applies now.
    Restarted,
    /// The portal is not running as a user service, so nothing restarted;
    /// the choice applies when it next starts (at the latest, the next
    /// login).
    NotRunning,
}

/// The record kept while the opt-in is on.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
struct Record {
    /// The file the app changed.
    file: PathBuf,
    /// What it held before, or `None` when it did not exist.
    previous: Option<String>,
    /// What the app wrote.
    written: String,
}

/// The opt-in for one user and one packaged backend.
#[derive(Debug, Clone)]
pub struct FileDialogRegistration {
    paths: FileDialogPaths,
    portal_name: String,
    sandbox: Sandbox,
}

impl FileDialogRegistration {
    /// The opt-in that prefers the backend `portal_name` (the packaged
    /// `.portal` file's name without the extension).
    pub fn new(paths: FileDialogPaths, portal_name: &str, sandbox: Sandbox) -> Self {
        Self {
            paths,
            portal_name: portal_name.to_owned(),
            sandbox,
        }
    }

    /// Whether the opt-in can work here: not inside Flatpak.
    pub fn is_available(&self) -> bool {
        !self.sandbox.is_flatpak()
    }

    /// The user's configuration file enabling changes: the one the portal
    /// reads now, else the first one it would read for this desktop.
    pub fn config_file(&self) -> PathBuf {
        self.effective_user_file()
            .unwrap_or_else(|| self.first_config_file())
    }

    /// The user's configuration file the portal reads first for this
    /// desktop, whether or not it exists.
    fn first_config_file(&self) -> PathBuf {
        let name = self.paths.desktops.first().map_or_else(
            || "portals.conf".to_owned(),
            |desktop| format!("{desktop}-portals.conf"),
        );
        self.paths.config_home.join(PORTAL_SUBDIR).join(name)
    }

    /// Whether the portal would send file dialogs to the app: the user's
    /// file prefers this backend. Reading only reads.
    pub fn is_enabled(&self) -> bool {
        if !self.is_available() {
            return false;
        }
        let Ok(contents) = fs::read_to_string(self.config_file()) else {
            return false;
        };
        preferred_value(&contents, FILE_CHOOSER_KEY)
            .is_some_and(|value| first_portal(&value) == self.portal_name)
    }

    /// Who the portal sends file dialogs to now: the first backend named
    /// for the interface, else the default, in the first file found.
    /// `None` when no file names one.
    pub fn current_backend(&self) -> Option<String> {
        let contents = fs::read_to_string(self.effective_file()?).ok()?;
        preferred_value(&contents, FILE_CHOOSER_KEY)
            .or_else(|| preferred_value(&contents, "default"))
            .map(|value| first_portal(&value))
    }

    /// Prefers the app's backend for file dialogs. Enabling again does
    /// nothing.
    ///
    /// # Errors
    ///
    /// [`FileDialogError::Unsupported`] inside Flatpak,
    /// [`FileDialogError::Symlink`] for a symlinked file, and
    /// [`FileDialogError::Io`] when reading or writing fails.
    pub fn enable(&self) -> Result<(), FileDialogError> {
        if !self.is_available() {
            return Err(FileDialogError::Unsupported);
        }
        if self.is_enabled() {
            return Ok(());
        }
        let file = self.config_file();
        refuse_symlink(&file)?;
        let previous = read_optional(&file)?;
        let base = match &previous {
            Some(contents) => contents.clone(),
            None => self.system_contents().unwrap_or_default(),
        };
        let written = with_preference(&base, FILE_CHOOSER_KEY, &self.portal_name);
        if self.read_record()?.is_none() {
            let record = Record {
                file: file.clone(),
                previous,
                written: written.clone(),
            };
            self.write_record(&record)?;
        } else {
            self.update_written(&written)?;
        }
        write_text(&file, &written)
    }

    /// Gives file dialogs back: restores the file if it still holds what
    /// the app wrote, otherwise removes only the app's line.
    ///
    /// # Errors
    ///
    /// [`FileDialogError::Symlink`] or [`FileDialogError::Io`].
    pub fn disable(&self) -> Result<DisabledFileDialogs, FileDialogError> {
        let record = self.read_record()?;
        let file = record
            .as_ref()
            .map_or_else(|| self.config_file(), |record| record.file.clone());
        refuse_symlink(&file)?;
        let current = read_optional(&file)?;
        let outcome = match (&record, &current) {
            (Some(record), Some(current)) if *current == record.written => {
                match &record.previous {
                    Some(previous) => write_text(&file, previous)?,
                    None => remove_file(&file)?,
                }
                DisabledFileDialogs::Restored
            }
            (_, Some(current)) if self.line_is_ours(current) => {
                write_text(&file, &without_preference(current, FILE_CHOOSER_KEY))?;
                DisabledFileDialogs::LineRemoved
            }
            _ => DisabledFileDialogs::NotEnabled,
        };
        remove_file(&self.record_path())?;
        Ok(outcome)
    }

    /// Restarts the user's desktop portal so it reads the configuration
    /// again, if it runs as a user service: `systemctl --user is-active`
    /// first, then `try-restart` (which never starts a portal that was
    /// not running) and `is-active` again, each run with an argument list
    /// and no shell. `try-restart` succeeds even when the portal was not
    /// running, so only an active unit counts as restarted.
    ///
    /// # Errors
    ///
    /// [`FileDialogError::Unsupported`] inside Flatpak, and
    /// [`FileDialogError::Io`] when `systemctl` cannot run, the restart
    /// fails, or the portal is not running after it.
    pub fn restart_portal(&self) -> Result<PortalRestart, FileDialogError> {
        if !self.is_available() {
            return Err(FileDialogError::Unsupported);
        }
        restart_portal_with(Path::new("systemctl"))
    }

    /// Runs `operation` on a GIO worker thread, as the other integrations'
    /// file work does.
    pub fn run_in_background<T, F>(&self, operation: F) -> impl Future<Output = T> + 'static
    where
        T: Send + 'static,
        F: FnOnce(&Self) -> T + Send + 'static,
    {
        let registration = self.clone();
        on_worker(move || operation(&registration))
    }

    /// Whether `contents` prefers this backend for file dialogs.
    fn line_is_ours(&self, contents: &str) -> bool {
        preferred_value(contents, FILE_CHOOSER_KEY)
            .is_some_and(|value| first_portal(&value) == self.portal_name)
    }

    /// The first user configuration file the portal would read, if any.
    fn effective_user_file(&self) -> Option<PathBuf> {
        first_existing(&self.paths.config_home.join(PORTAL_SUBDIR), &self.paths.desktops)
    }

    /// The first configuration file the portal would read: the user's,
    /// else the system's.
    fn effective_file(&self) -> Option<PathBuf> {
        self.effective_user_file().or_else(|| self.system_file())
    }

    /// The system file the portal would read for this desktop.
    fn system_file(&self) -> Option<PathBuf> {
        self.paths
            .system_dirs
            .iter()
            .find_map(|dir| first_existing(&dir.join(PORTAL_SUBDIR), &self.paths.desktops))
    }

    /// The system file's contents, the base of a new user file.
    fn system_contents(&self) -> Option<String> {
        fs::read_to_string(self.system_file()?).ok()
    }

    fn record_path(&self) -> PathBuf {
        self.paths.settings.join(RECORD_FILE_NAME)
    }

    fn read_record(&self) -> Result<Option<Record>, FileDialogError> {
        let path = self.record_path();
        let Some(contents) = read_optional(&path)? else {
            return Ok(None);
        };
        // A record that cannot be read is treated as missing: disabling
        // then removes only the app's line, which is always safe.
        Ok(serde_json::from_str(&contents).ok())
    }

    fn write_record(&self, record: &Record) -> Result<(), FileDialogError> {
        let text = serde_json::to_string_pretty(record).expect("a record serialises");
        write_text(&self.record_path(), &text)
    }

    /// Keeps the first `previous` but remembers the newest write.
    fn update_written(&self, written: &str) -> Result<(), FileDialogError> {
        if let Some(mut record) = self.read_record()? {
            written.clone_into(&mut record.written);
            self.write_record(&record)?;
        }
        Ok(())
    }
}

/// [`FileDialogRegistration::restart_portal`] with `program` as `systemctl`.
fn restart_portal_with(program: &Path) -> Result<PortalRestart, FileDialogError> {
    if !run_systemctl(program, "is-active")? {
        return Ok(PortalRestart::NotRunning);
    }
    let failure = |what: &str| FileDialogError::Io {
        path: program.to_owned(),
        error: io::Error::other(format!("{PORTAL_SERVICE} {what}")),
    };
    if !run_systemctl(program, "try-restart")? {
        return Err(failure("could not be restarted"));
    }
    if !run_systemctl(program, "is-active")? {
        return Err(failure("did not start again"));
    }
    Ok(PortalRestart::Restarted)
}

/// Runs `program --user <verb> --quiet xdg-desktop-portal.service`;
/// returns whether it succeeded.
fn run_systemctl(program: &Path, verb: &str) -> Result<bool, FileDialogError> {
    let status = std::process::Command::new(program)
        .args(["--user", verb, "--quiet", PORTAL_SERVICE])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map_err(|error| FileDialogError::Io {
            path: program.to_owned(),
            error,
        })?;
    Ok(status.success())
}

/// The first `<desktop>-portals.conf`, then `portals.conf`, in `dir`.
fn first_existing(dir: &Path, desktops: &[String]) -> Option<PathBuf> {
    desktops
        .iter()
        .map(|desktop| dir.join(format!("{desktop}-portals.conf")))
        .chain(std::iter::once(dir.join("portals.conf")))
        .find(|path| path.is_file())
}

/// The first backend of a `;`-separated preference list.
fn first_portal(value: &str) -> String {
    value
        .split(';')
        .map(str::trim)
        .find(|name| !name.is_empty())
        .unwrap_or_default()
        .to_owned()
}

/// The value of `key` in the `[preferred]` section, as `GKeyFile` reads
/// it (the last one wins).
pub fn preferred_value(contents: &str, key: &str) -> Option<String> {
    let mut in_section = false;
    let mut value = None;
    for line in contents.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_section = line == PREFERRED_SECTION;
            continue;
        }
        if !in_section || line.starts_with('#') {
            continue;
        }
        if let Some((name, rest)) = line.split_once('=') {
            if name.trim() == key {
                value = Some(rest.trim().to_owned());
            }
        }
    }
    value
}

/// `contents` with `key=value` set in `[preferred]`: an existing line is
/// replaced, otherwise the line, with [`LINE_COMMENT`] above it, is added
/// at the end of the section, which is created when missing.
pub fn with_preference(contents: &str, key: &str, value: &str) -> String {
    let cleaned = without_preference(contents, key);
    let line = format!("{LINE_COMMENT}\n{key}={value}\n");
    let mut lines: Vec<&str> = cleaned.lines().collect();
    let Some(start) = lines.iter().position(|line| line.trim() == PREFERRED_SECTION) else {
        let mut text = cleaned.trim_end().to_owned();
        if !text.is_empty() {
            text.push_str("\n\n");
        }
        return format!("{text}{PREFERRED_SECTION}\n{line}");
    };
    let mut end = lines[start + 1..]
        .iter()
        .position(|line| line.trim().starts_with('['))
        .map_or(lines.len(), |offset| start + 1 + offset);
    while end > start + 1 && lines[end - 1].trim().is_empty() {
        end -= 1;
    }
    let tail = lines.split_off(end);
    let mut text = lines.join("\n");
    text.push('\n');
    text.push_str(&line);
    if !tail.is_empty() {
        text.push_str(&tail.join("\n"));
        text.push('\n');
    }
    text
}

/// `contents` without `key` in `[preferred]`, and without the app's
/// comment line above it.
pub fn without_preference(contents: &str, key: &str) -> String {
    let mut in_section = false;
    let mut kept: Vec<&str> = Vec::new();
    for line in contents.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            in_section = trimmed == PREFERRED_SECTION;
        } else if in_section
            && trimmed
                .split_once('=')
                .is_some_and(|(name, _)| name.trim() == key)
        {
            if kept
                .last()
                .is_some_and(|previous| previous.trim() == LINE_COMMENT)
            {
                kept.pop();
            }
            continue;
        }
        kept.push(line);
    }
    let mut text = kept.join("\n");
    if contents.ends_with('\n') && !text.is_empty() {
        text.push('\n');
    }
    text
}

/// Refuses a symlink at `path`; a missing file is fine.
fn refuse_symlink(path: &Path) -> Result<(), FileDialogError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => Err(FileDialogError::Symlink(path.to_owned())),
        Ok(_) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(FileDialogError::Io {
            path: path.to_owned(),
            error,
        }),
    }
}

/// The file's text, or `None` when it does not exist.
fn read_optional(path: &Path) -> Result<Option<String>, FileDialogError> {
    match fs::read_to_string(path) {
        Ok(contents) => Ok(Some(contents)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(FileDialogError::Io {
            path: path.to_owned(),
            error,
        }),
    }
}

/// Writes `text` atomically as a private file, creating its folder.
fn write_text(path: &Path, text: &str) -> Result<(), FileDialogError> {
    let io_error = |error| FileDialogError::Io {
        path: path.to_owned(),
        error,
    };
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(io_error)?;
    }
    write_private_file(path, ".winspace-", text.as_bytes()).map_err(io_error)
}

/// Removes `path`; a missing file is fine.
fn remove_file(path: &Path) -> Result<(), FileDialogError> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(FileDialogError::Io {
            path: path.to_owned(),
            error,
        }),
    }
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::PermissionsExt;

    use super::*;
    use crate::test_support::temporary_folder;

    /// A stand-in `systemctl` in `folder` that logs its verbs and answers
    /// `is-active` from the file `active` (present: running).
    fn fake_systemctl(folder: &Path) -> PathBuf {
        let program = folder.join("systemctl");
        let script = "#!/bin/sh\ndir=$(dirname \"$0\")\necho \"$2\" >> \"$dir/log\"\ncase \"$2\" in\n  \
                      is-active) [ -e \"$dir/active\" ] ;;\nesac\n";
        fs::write(&program, script).expect("the stand-in is written");
        fs::set_permissions(&program, fs::Permissions::from_mode(0o755)).expect("it runs");
        program
    }

    fn verbs(folder: &Path) -> Vec<String> {
        fs::read_to_string(folder.join("log"))
            .unwrap_or_default()
            .lines()
            .map(str::to_owned)
            .collect()
    }

    /// `try-restart` succeeds when the portal is not running, so Apply now
    /// asks whether it runs first and reports that nothing restarted.
    ///
    /// parity: INT-032
    #[test]
    fn apply_now_restarts_only_a_running_portal() {
        let folder = temporary_folder();
        let program = fake_systemctl(folder.path());

        let outcome = restart_portal_with(&program).expect("systemctl runs");
        assert_eq!(outcome, PortalRestart::NotRunning);
        assert_eq!(verbs(folder.path()), ["is-active"], "nothing is restarted");

        fs::write(folder.path().join("active"), "").expect("the portal runs");
        fs::remove_file(folder.path().join("log")).expect("the log is cleared");
        let outcome = restart_portal_with(&program).expect("systemctl runs");
        assert_eq!(outcome, PortalRestart::Restarted);
        assert_eq!(verbs(folder.path()), ["is-active", "try-restart", "is-active"]);
    }

    /// A portal that is gone after the restart is reported as a failure.
    ///
    /// parity: INT-032
    #[test]
    fn a_portal_that_does_not_come_back_is_a_failure() {
        let folder = temporary_folder();
        let program = folder.path().join("systemctl");
        let script =
            "#!/bin/sh\ndir=$(dirname \"$0\")\ncase \"$2\" in\n  is-active) [ -e \"$dir/active\" ] ;;\n  \
                      try-restart) rm -f \"$dir/active\" ;;\nesac\n";
        fs::write(&program, script).expect("the stand-in is written");
        fs::set_permissions(&program, fs::Permissions::from_mode(0o755)).expect("it runs");
        fs::write(folder.path().join("active"), "").expect("the portal runs");
        let error = restart_portal_with(&program).expect_err("the portal stopped");
        assert!(error.to_string().contains("did not start again"), "{error}");
    }
}
