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
//!
//! When the portal service's environment sets [`PORTAL_DIR_VARIABLE`]
//! (`XDG_DESKTOP_PORTAL_DIR`, which some tools set in a drop-in of the
//! unit to add a backend of their own), xdg-desktop-portal reads its
//! `.portal` files and its `(<desktop>-)portals.conf` only from that
//! folder and ignores every other configuration folder. The opt-in reads
//! that environment from the systemd user manager over D-Bus (the
//! manager's environment, then the unit's `Environment=` lines), off the
//! main thread, and then works in that folder: it changes the file the
//! portal reads there, or creates the first desktop's file with nothing
//! but the `[preferred]` line, as a copy of the system file would replace
//! the backends the portal finds there by their `UseIn` key; and it links
//! the packaged `.portal` file into the folder, as the portal finds no
//! other. Disabling restores the file and removes the link if the app
//! made it; a file or link of someone else's with that name is never
//! replaced. A folder the variable names that is not absolute or cannot
//! be created is reported, not taken for success.
//!
//! KDE's own applications (Plasma, Kate, System Settings, a Plasma
//! widget's settings) do not ask the portal: their Qt platform theme shows
//! KDE's dialog itself unless `PLASMA_INTEGRATION_USE_PORTAL=1` is set.
//! On a KDE session, enabling therefore also writes
//! `$XDG_CONFIG_HOME/plasma-workspace/env/openxplorer-file-dialogs.sh`,
//! which Plasma runs at login, exporting that variable; disabling removes
//! it if it still holds what the app wrote. It applies from the next
//! login, as Plasma reads it only then.

use std::fs;
use std::future::Future;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use gio::prelude::*;

use super::background_portal::{DESKTOP_PORTAL_NAME, DESKTOP_PORTAL_PATH};
use super::private_file::write_private_file;
use super::sandbox::Sandbox;
use super::worker::on_worker;

/// The portal interface whose backend the opt-in chooses.
pub const FILE_CHOOSER_KEY: &str = "org.freedesktop.impl.portal.FileChooser";

/// The user's systemd unit of the desktop portal, which reads the
/// configuration when it starts.
pub const PORTAL_SERVICE: &str = "xdg-desktop-portal.service";

/// The variable that makes the desktop portal read its `.portal` files
/// and its configuration from one folder only.
pub const PORTAL_DIR_VARIABLE: &str = "XDG_DESKTOP_PORTAL_DIR";

/// The systemd user manager, which starts [`PORTAL_SERVICE`], on the
/// session bus.
const SYSTEMD_NAME: &str = "org.freedesktop.systemd1";
const SYSTEMD_PATH: &str = "/org/freedesktop/systemd1";
const SYSTEMD_MANAGER: &str = "org.freedesktop.systemd1.Manager";
const SYSTEMD_SERVICE: &str = "org.freedesktop.systemd1.Service";

/// How long the service manager may take to answer, in milliseconds.
const SYSTEMD_TIMEOUT_MS: i32 = 5_000;

/// The folder below each data folder that holds the `.portal` files.
const PORTALS_SUBDIR: &str = "xdg-desktop-portal/portals";

/// The comment above the app's line, which says who set it.
const LINE_COMMENT: &str = "# Open and Save dialogs: set by OpenXplorer (Settings > Default apps).";

/// The configuration folder below each base folder.
const PORTAL_SUBDIR: &str = "xdg-desktop-portal";

/// The record of what the file held before, in the settings folder.
const RECORD_FILE_NAME: &str = "file-dialogs.json";

/// The section that holds the preferences.
const PREFERRED_SECTION: &str = "[preferred]";

/// The variable that makes KDE's Qt platform theme ask the portal for
/// file dialogs instead of showing its own.
pub const KDE_PORTAL_VARIABLE: &str = "PLASMA_INTEGRATION_USE_PORTAL";

/// The folder below `$XDG_CONFIG_HOME` whose `*.sh` scripts Plasma runs at
/// login, and the app's script there.
const KDE_ENV_SUBDIR: &str = "plasma-workspace/env";
const KDE_ENV_FILE_NAME: &str = "openxplorer-file-dialogs.sh";

/// What the app's login script holds.
const KDE_ENV_SCRIPT: &str = "# Open and Save dialogs of KDE apps go to the desktop portal, which sends them to\n\
                              # OpenXplorer. Set by OpenXplorer (Settings > Default apps); Restore removes it.\n\
                              export PLASMA_INTEGRATION_USE_PORTAL=1\n";

/// What is at the KDE login script's path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum KdeScript {
    Missing,
    /// The script the app writes.
    Ours,
    /// A file of the user's, or a symlink.
    Other,
}

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
    /// The one folder the portal reads, when its service's environment
    /// sets [`PORTAL_DIR_VARIABLE`].
    pub portal_dir: PortalDir,
}

impl FileDialogPaths {
    /// The current user's folders and desktop, with the record in
    /// `settings`; the portal's folder is read from its service when the
    /// opt-in works.
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
            portal_dir: PortalDir::FromService,
        }
    }
}

/// Where the portal's [`PORTAL_DIR_VARIABLE`] comes from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PortalDir {
    /// The environment systemd starts [`PORTAL_SERVICE`] with, asked of
    /// the user's service manager over D-Bus. It is asked once per
    /// [`FileDialogRegistration::run_in_background`] operation, on its
    /// worker thread, since a tool may set it at any time.
    FromService,
    /// The variable's value, or `None` when it is not set and the portal
    /// reads the usual folders.
    Fixed(Option<PathBuf>),
    /// Which folder the portal reads cannot be told, for this reason.
    Unknown(String),
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
    /// The portal reads only the folder [`PORTAL_DIR_VARIABLE`] names, and
    /// the app cannot make it read the backend from there.
    #[error(
        "The desktop portal reads its settings only from {} (XDG_DESKTOP_PORTAL_DIR), which cannot be \
         used: {reason}",
        dir.display()
    )]
    PortalDir {
        /// The folder the variable names.
        dir: PathBuf,
        /// What is wrong with it.
        reason: String,
    },
    /// Which folder the portal reads its settings from cannot be told.
    #[error("Cannot tell where the desktop portal reads its settings: {0}")]
    PortalUnknown(String),
    /// Once restarted, the portal reads its settings from another folder
    /// than the one the change was made in.
    #[error(
        "The desktop portal now reads its settings from another folder. Turn Open and Save dialogs on again."
    )]
    PortalMoved,
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
    /// The link to the packaged `.portal` file the app made in the folder
    /// [`PORTAL_DIR_VARIABLE`] names; `None` when it made none, as in
    /// records written before the app knew the variable.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    link: Option<PortalLink>,
}

/// A symlink the app made to its packaged `.portal` file.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
struct PortalLink {
    /// The symlink.
    path: PathBuf,
    /// The packaged file it points to.
    target: PathBuf,
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

    /// The configuration file enabling changes: the one the portal reads
    /// now, else the first one it would read for this desktop, in the
    /// user's folder or the one folder [`PORTAL_DIR_VARIABLE`] names.
    ///
    /// # Errors
    ///
    /// [`FileDialogError::PortalDir`] when the variable names a folder
    /// that is not absolute.
    pub fn config_file(&self) -> Result<PathBuf, FileDialogError> {
        Ok(self.config_file_in(self.portal_dir()?.as_deref()))
    }

    /// [`Self::config_file`] for the portal's folder `only`.
    fn config_file_in(&self, only: Option<&Path>) -> PathBuf {
        let dir = self.user_dir(only);
        first_existing(&dir, &self.paths.desktops).unwrap_or_else(|| {
            let name = self.paths.desktops.first().map_or_else(
                || "portals.conf".to_owned(),
                |desktop| format!("{desktop}-portals.conf"),
            );
            dir.join(name)
        })
    }

    /// The folder of the configuration the user can change: `only`, the
    /// folder the portal reads alone, else the user's configuration
    /// folder.
    fn user_dir(&self, only: Option<&Path>) -> PathBuf {
        only.map_or_else(|| self.paths.config_home.join(PORTAL_SUBDIR), Path::to_path_buf)
    }

    /// The one folder the portal reads, when [`PORTAL_DIR_VARIABLE`] is
    /// set for it.
    fn portal_dir(&self) -> Result<Option<PathBuf>, FileDialogError> {
        let dir = match &self.paths.portal_dir {
            PortalDir::Fixed(dir) => dir.clone(),
            PortalDir::FromService => service_portal_dir()
                .map_err(FileDialogError::PortalUnknown)?
                .map(PathBuf::from),
            PortalDir::Unknown(reason) => return Err(FileDialogError::PortalUnknown(reason.clone())),
        };
        match dir {
            Some(dir) if !dir.is_absolute() => Err(FileDialogError::PortalDir {
                dir,
                reason: "it is not an absolute path".to_owned(),
            }),
            dir => Ok(dir),
        }
    }

    /// Whether the portal would send file dialogs to the app: the file it
    /// reads prefers this backend, and in the one folder
    /// [`PORTAL_DIR_VARIABLE`] names, the backend's `.portal` file is
    /// there too. Reading only reads.
    pub fn is_enabled(&self) -> bool {
        if !self.is_available() {
            return false;
        }
        self.portal_dir()
            .is_ok_and(|only| self.is_enabled_in(only.as_deref()))
    }

    /// Whether an Enable is on record, which [`Self::disable`] undoes even
    /// when the portal's configuration cannot be read now, as
    /// [`Self::is_enabled`] then says it is off. Reading only reads.
    pub fn has_record(&self) -> bool {
        self.record_path().is_file()
    }

    /// [`Self::is_enabled`] for the portal's folder `only`.
    fn is_enabled_in(&self, only: Option<&Path>) -> bool {
        let Ok(contents) = fs::read_to_string(self.config_file_in(only)) else {
            return false;
        };
        self.line_is_ours(&contents) && only.is_none_or(|dir| self.backend_file(dir).is_file())
    }

    /// Who the portal sends file dialogs to now: the first backend named
    /// for the interface, else the default, in the first file found.
    /// `None` when no file names one.
    pub fn current_backend(&self) -> Option<String> {
        let only = self.portal_dir().ok()?;
        let contents = fs::read_to_string(self.effective_file(only.as_deref())?).ok()?;
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
    /// [`FileDialogError::Symlink`] for a symlinked file,
    /// [`FileDialogError::PortalDir`] when the portal reads only a folder
    /// the backend cannot be added to, and [`FileDialogError::Io`] when
    /// reading or writing fails.
    pub fn enable(&self) -> Result<(), FileDialogError> {
        if !self.is_available() {
            return Err(FileDialogError::Unsupported);
        }
        let only = self.portal_dir()?;
        let only = only.as_deref();
        if self.is_enabled_in(only) {
            // Enabled before KDE apps were covered: add their script.
            return self.enable_for_kde_apps().map(|_| ());
        }
        if let Some(dir) = only {
            fs::create_dir_all(dir).map_err(|error| FileDialogError::PortalDir {
                dir: dir.to_owned(),
                reason: error.to_string(),
            })?;
        }
        let file = self.config_file_in(only);
        refuse_symlink(&file)?;
        let previous = read_optional(&file)?;
        // A record of an Enable in a file the portal does not read now
        // (written before XDG_DESKTOP_PORTAL_DIR was set, say) is settled
        // first, so Restore later puts back the file changed now.
        let record = match self.read_record()? {
            Some(record) if !same_path(&record.file, &file) => {
                self.settle(&record)?;
                remove_file(&self.record_path())?;
                None
            }
            record => record,
        };
        // KDE's login script is written first, so an Enable that fails
        // there changes nothing; if the backend's link or the portal file
        // then fails, what was just made is taken away again.
        let wrote_script = self.enable_for_kde_apps()?;
        let undo_script = || {
            if wrote_script {
                let _ = remove_file(&self.kde_env_file());
            }
        };
        let link = match only.map(|dir| self.show_backend(dir)).transpose() {
            Ok(link) => link.flatten(),
            Err(error) => {
                undo_script();
                return Err(error);
            }
        };
        let commit = || -> Result<(), FileDialogError> {
            let base = match (&previous, only) {
                (Some(contents), _) => contents.clone(),
                // The portal reads no other file there: a copy of the
                // system's would replace the backends it chooses by
                // their `UseIn` key for every other interface.
                (None, Some(_)) => String::new(),
                (None, None) => self.system_contents().unwrap_or_default(),
            };
            let written = with_preference(&base, FILE_CHOOSER_KEY, &self.portal_name);
            let record = match record {
                // Keeps the first `previous` but remembers the newest
                // write, and the file's path as named now (a folder alias
                // may have gone since), while the file still holds what
                // the app wrote; once the user changed it, what it holds
                // now is what Restore puts back, so their change is kept.
                Some(record) if previous.as_ref() == Some(&record.written) => Record {
                    file: file.clone(),
                    written: written.clone(),
                    link: link.clone().or(record.link),
                    ..record
                },
                Some(record) => Record {
                    file: file.clone(),
                    previous: previous.clone(),
                    written: written.clone(),
                    link: link.clone().or(record.link),
                },
                None => Record {
                    file: file.clone(),
                    previous: previous.clone(),
                    written: written.clone(),
                    link: link.clone(),
                },
            };
            self.write_record(&record)?;
            write_text(&file, &written)
        };
        commit().inspect_err(|_| {
            undo_script();
            if let Some(link) = &link {
                let _ = remove_link(link);
            }
        })
    }

    /// The backend's `.portal` file in the portal's folder `dir`.
    fn backend_file(&self, dir: &Path) -> PathBuf {
        dir.join(format!("{}.portal", self.portal_name))
    }

    /// The packaged `.portal` file, in the first system data folder that
    /// has it.
    fn packaged_backend(&self) -> Option<PathBuf> {
        self.paths
            .system_dirs
            .iter()
            .map(|dir| self.backend_file(&dir.join(PORTALS_SUBDIR)))
            .find(|path| path.is_file())
    }

    /// Makes the portal that reads only `dir` find the backend: links the
    /// packaged `.portal` file there, as the links to the desktop's own
    /// backends in such a folder are, and returns the link it made.
    /// `None` when a file or a working link of that name is already
    /// there; anything else of that name is not replaced.
    fn show_backend(&self, dir: &Path) -> Result<Option<PortalLink>, FileDialogError> {
        let path = self.backend_file(dir);
        let unusable = |reason: String| FileDialogError::PortalDir {
            dir: dir.to_owned(),
            reason,
        };
        match fs::symlink_metadata(&path) {
            Ok(_) if path.is_file() => return Ok(None),
            Ok(_) => {
                return Err(unusable(format!(
                    "{} is in the way and was not made by OpenXplorer",
                    path.display()
                )))
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(FileDialogError::Io { path, error }),
        }
        let target = self.packaged_backend().ok_or_else(|| {
            unusable(format!(
                "{}.portal is not installed in the system's data folders",
                self.portal_name
            ))
        })?;
        std::os::unix::fs::symlink(&target, &path).map_err(|error| FileDialogError::Io {
            path: path.clone(),
            error,
        })?;
        Ok(Some(PortalLink { path, target }))
    }

    /// Whether this is a KDE session, whose own applications need
    /// [`KDE_PORTAL_VARIABLE`] to ask the portal.
    pub fn is_kde_session(&self) -> bool {
        self.paths.desktops.iter().any(|desktop| desktop == "kde")
    }

    /// The login script that sets [`KDE_PORTAL_VARIABLE`].
    pub fn kde_env_file(&self) -> PathBuf {
        self.paths
            .config_home
            .join(KDE_ENV_SUBDIR)
            .join(KDE_ENV_FILE_NAME)
    }

    /// Whether KDE's own applications are set to ask the portal at the
    /// next login: this is a KDE session and the app's login script is in
    /// place. Reading only reads.
    pub fn covers_kde_apps(&self) -> bool {
        self.is_kde_session() && matches!(self.kde_script(), Ok(KdeScript::Ours))
    }

    /// Whether a file of the user's, or a symlink, sits where the login
    /// script goes on a KDE session, so the app leaves it alone and KDE's
    /// own applications keep KDE's dialog.
    pub fn kde_script_is_someone_elses(&self) -> bool {
        self.is_kde_session() && matches!(self.kde_script(), Ok(KdeScript::Other))
    }

    /// What is at the login script's path: nothing, the app's script, or
    /// anything else (a file of the user's, a symlink, or an entry that
    /// cannot be read as text).
    fn kde_script(&self) -> Result<KdeScript, FileDialogError> {
        let script = self.kde_env_file();
        if refuse_symlink(&script).is_err_and(|error| matches!(error, FileDialogError::Symlink(_))) {
            return Ok(KdeScript::Other);
        }
        match read_optional(&script) {
            Ok(None) => Ok(KdeScript::Missing),
            Ok(Some(contents)) if contents == KDE_ENV_SCRIPT => Ok(KdeScript::Ours),
            Ok(Some(_)) => Ok(KdeScript::Other),
            // Something there that cannot be read as text (a folder, say)
            // is not the app's script either: Enable and Restore leave it.
            Err(_) if fs::symlink_metadata(&script).is_ok() => Ok(KdeScript::Other),
            Err(error) => Err(error),
        }
    }

    /// Writes the login script on a KDE session and says whether it did.
    /// A file of the user's at that name, or a symlink, is left alone.
    fn enable_for_kde_apps(&self) -> Result<bool, FileDialogError> {
        if !self.is_kde_session() || self.kde_script()? != KdeScript::Missing {
            return Ok(false);
        }
        write_text(&self.kde_env_file(), KDE_ENV_SCRIPT)?;
        Ok(true)
    }

    /// Takes the login script away if it still holds what the app wrote.
    /// A file of the user's, a symlink, or anything that cannot be read
    /// there is not the app's and is left alone, so Restore goes on.
    fn disable_for_kde_apps(&self) -> Result<(), FileDialogError> {
        if matches!(self.kde_script(), Ok(KdeScript::Ours)) {
            remove_file(&self.kde_env_file())?;
        }
        Ok(())
    }

    /// Gives file dialogs back: restores the file if it still holds what
    /// the app wrote, otherwise removes only the app's line. KDE's login
    /// script goes first, so if it cannot be removed nothing has changed
    /// and Restore can be tried again.
    ///
    /// # Errors
    ///
    /// [`FileDialogError::Symlink`], [`FileDialogError::PortalDir`] (no
    /// record, and the portal's folder is not absolute) or
    /// [`FileDialogError::Io`].
    pub fn disable(&self) -> Result<DisabledFileDialogs, FileDialogError> {
        self.disable_for_kde_apps()?;
        let outcome = match self.read_record()? {
            Some(record) => self.settle(&record)?,
            None => self.restore(&self.config_file()?, None)?,
        };
        remove_file(&self.record_path())?;
        Ok(outcome)
    }

    /// Undoes what `record` says the app did: restores its file and
    /// removes the backend's link if it is still the one the app made.
    fn settle(&self, record: &Record) -> Result<DisabledFileDialogs, FileDialogError> {
        let outcome = self.restore(&record.file, Some(record))?;
        if let Some(link) = &record.link {
            remove_link(link)?;
        }
        Ok(outcome)
    }

    /// Restores `file` if it still holds what `record` says the app
    /// wrote, otherwise removes only the app's line.
    fn restore(&self, file: &Path, record: Option<&Record>) -> Result<DisabledFileDialogs, FileDialogError> {
        refuse_symlink(file)?;
        let current = read_optional(file)?;
        Ok(match (record, &current) {
            (Some(record), Some(current)) if *current == record.written => {
                match &record.previous {
                    Some(previous) => write_text(file, previous)?,
                    None => remove_file(file)?,
                }
                DisabledFileDialogs::Restored
            }
            (_, Some(current)) if self.line_is_ours(current) => {
                write_text(file, &without_preference(current, FILE_CHOOSER_KEY))?;
                DisabledFileDialogs::LineRemoved
            }
            _ => DisabledFileDialogs::NotEnabled,
        })
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
    /// [`FileDialogError::Unsupported`] inside Flatpak,
    /// [`FileDialogError::Io`] when `systemctl` cannot run, the restart
    /// fails, or the portal is not running after it, and
    /// [`FileDialogError::PortalMoved`] when the restarted portal reads
    /// another folder than the one the operation found, and
    /// [`FileDialogError::PortalUnknown`], before restarting, when that
    /// folder could not be told.
    pub fn restart_portal(&self) -> Result<PortalRestart, FileDialogError> {
        if !self.is_available() {
            return Err(FileDialogError::Unsupported);
        }
        // A change made without knowing the folder could not be checked.
        if let PortalDir::Unknown(reason) = &self.paths.portal_dir {
            return Err(FileDialogError::PortalUnknown(reason.clone()));
        }
        let restart = restart_portal_with(Path::new("systemctl"))?;
        // systemd builds the environment again for the new process, so
        // the folder the change was made for is checked against it.
        if restart == PortalRestart::Restarted {
            if let PortalDir::Fixed(before) = &self.paths.portal_dir {
                let now = service_portal_dir()
                    .map_err(FileDialogError::PortalUnknown)?
                    .map(PathBuf::from);
                if !same_folder(now.as_deref(), before.as_deref()) {
                    return Err(FileDialogError::PortalMoved);
                }
            }
        }
        Ok(restart)
    }

    /// Runs `operation` on a GIO worker thread, as the other integrations'
    /// file work does, after any other operation of this opt-in finished:
    /// Enable, Apply now and Restore each read and write the portal file,
    /// the record and KDE's login script in several steps, so two at once
    /// (Enable clicked, then Restore before it finished) could leave the
    /// script behind with the record gone. The portal's folder
    /// ([`PortalDir::FromService`]) is asked of the service manager once,
    /// there, for the whole operation.
    pub fn run_in_background<T, F>(&self, operation: F) -> impl Future<Output = T> + 'static
    where
        T: Send + 'static,
        F: FnOnce(&Self) -> T + Send + 'static,
    {
        let mut registration = self.clone();
        on_worker(move || {
            one_at_a_time(|| {
                if registration.paths.portal_dir == PortalDir::FromService && registration.is_available() {
                    registration.paths.portal_dir = match service_portal_dir() {
                        Ok(dir) => PortalDir::Fixed(dir.map(PathBuf::from)),
                        Err(reason) => PortalDir::Unknown(reason),
                    };
                }
                operation(&registration)
            })
        })
    }

    /// Whether `contents` prefers this backend for file dialogs.
    fn line_is_ours(&self, contents: &str) -> bool {
        preferred_value(contents, FILE_CHOOSER_KEY)
            .is_some_and(|value| first_portal(&value) == self.portal_name)
    }

    /// The first configuration file the portal would read: the user's,
    /// else the system's, or only one in `only`, the one folder the
    /// portal reads.
    fn effective_file(&self, only: Option<&Path>) -> Option<PathBuf> {
        let user_file = first_existing(&self.user_dir(only), &self.paths.desktops);
        match only {
            Some(_) => user_file,
            None => user_file.or_else(|| self.system_file()),
        }
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
}

/// [`PORTAL_DIR_VARIABLE`] in the environment of [`PORTAL_SERVICE`], asked
/// over D-Bus from the systemd user manager. That is the running portal's
/// own environment, which is exact; a portal that is not running is
/// started first through its D-Bus name, as any application's first portal
/// call starts it. Only when no process can be read is the environment
/// worked out as systemd would start it: the manager's own environment,
/// the unit's `Environment=` lines, then its `EnvironmentFile=` files,
/// each winning over the one before, and last its `UnsetEnvironment=`.
/// `Ok(None)` when it is not set there, or no service manager answers
/// (the portal then has the session's environment and is taken to read
/// the usual folders).
///
/// # Errors
///
/// Why the folder cannot be told: an environment file the fallback cannot
/// read as systemd does ([`environment_file_assignments`]), or a folder
/// or environment file that is not UTF-8.
fn service_portal_dir() -> Result<Option<String>, String> {
    let Ok(connection) = gio::bus_get_sync(gio::BusType::Session, gio::Cancellable::NONE) else {
        return Ok(None);
    };
    let call = |name: &str, path: &str, interface: &str, method: &str, parameters: &glib::Variant| {
        let flags = if name == SYSTEMD_NAME {
            gio::DBusCallFlags::NO_AUTO_START
        } else {
            gio::DBusCallFlags::NONE
        };
        connection
            .call_sync(
                Some(name),
                path,
                interface,
                method,
                Some(parameters),
                None,
                flags,
                SYSTEMD_TIMEOUT_MS,
                gio::Cancellable::NONE,
            )
            .ok()
    };
    let property = |path: &str, interface: &str, name: &str| -> Option<glib::Variant> {
        call(
            SYSTEMD_NAME,
            path,
            "org.freedesktop.DBus.Properties",
            "Get",
            &(interface, name).to_variant(),
        )
        .and_then(|reply| reply.get::<(glib::Variant,)>())
        .map(|(value,)| value)
    };
    let strings = |path: &str, interface: &str, name: &str| -> Vec<String> {
        property(path, interface, name)
            .and_then(|value| value.get::<Vec<String>>())
            .unwrap_or_default()
    };
    let mut environments = vec![strings(SYSTEMD_PATH, SYSTEMD_MANAGER, "Environment")];
    // GetUnit knows the unit once it ran this session; LoadUnit reads its
    // files without starting it.
    let unit = ["GetUnit", "LoadUnit"]
        .into_iter()
        .find_map(|method| {
            call(
                SYSTEMD_NAME,
                SYSTEMD_PATH,
                SYSTEMD_MANAGER,
                method,
                &(PORTAL_SERVICE,).to_variant(),
            )
        })
        .and_then(|reply| reply.get::<(glib::variant::ObjectPath,)>());
    let Some((unit,)) = unit else {
        return Ok(variable_in(&environments, &[], PORTAL_DIR_VARIABLE));
    };
    let unit = unit.as_str();
    let process_environment = || {
        property(unit, SYSTEMD_SERVICE, "MainPID")
            .and_then(|value| value.get::<u32>())
            .filter(|pid| *pid > 0)
            .and_then(|pid| fs::read(format!("/proc/{pid}/environ")).ok())
    };
    let running = process_environment().or_else(|| {
        call(
            DESKTOP_PORTAL_NAME,
            DESKTOP_PORTAL_PATH,
            "org.freedesktop.DBus.Peer",
            "Ping",
            &().to_variant(),
        )?;
        process_environment()
    });
    if let Some(environment) = running {
        return process_variable(&environment, PORTAL_DIR_VARIABLE);
    }
    environments.push(strings(unit, SYSTEMD_SERVICE, "Environment"));
    // `(path, ignore if missing)`; a missing file the unit needs keeps the
    // portal from starting at all, so either way it adds nothing.
    let files = property(unit, SYSTEMD_SERVICE, "EnvironmentFiles")
        .and_then(|value| value.get::<Vec<(String, bool)>>())
        .unwrap_or_default();
    for (file, _) in &files {
        if file.contains(['*', '?', '[']) {
            return Err(format!("its environment file {file} names several files"));
        }
        let Ok(contents) = fs::read(file) else {
            continue;
        };
        // Read as text, a file that is not UTF-8 would be skipped and the
        // folder it sets taken for unset.
        let contents =
            String::from_utf8(contents).map_err(|_| format!("its environment file {file} is not UTF-8"))?;
        let assignments = environment_file_assignments(&contents)
            .ok_or_else(|| format!("its environment file {file} uses escapes or continued lines"))?;
        environments.push(assignments);
    }
    let unset = strings(unit, SYSTEMD_SERVICE, "UnsetEnvironment");
    Ok(variable_in(&environments, &unset, PORTAL_DIR_VARIABLE))
}

/// Whether two portal folders are the same: both unset, or naming one
/// folder ([`same_path`]).
fn same_folder(one: Option<&Path>, other: Option<&Path>) -> bool {
    match (one, other) {
        (Some(one), Some(other)) => same_path(one, other),
        (one, other) => one == other,
    }
}

/// Whether two paths name one file or folder, also through a symlink.
fn same_path(one: &Path, other: &Path) -> bool {
    one == other
        || fs::canonicalize(one).is_ok_and(|one| fs::canonicalize(other).is_ok_and(|other| other == one))
}

/// The value of `name` in a process's environment as `/proc/<pid>/environ`
/// holds it: `NAME=value` entries separated by NUL bytes.
///
/// # Errors
///
/// A value that is not UTF-8, which read lossily would name another
/// folder than the one the portal reads.
fn process_variable(environment: &[u8], name: &str) -> Result<Option<String>, String> {
    let value = environment
        .split(|byte| *byte == 0)
        .find_map(|entry| entry.strip_prefix(name.as_bytes())?.strip_prefix(b"="));
    value
        .map(|value| String::from_utf8(value.to_vec()).map_err(|_| format!("its {name} is not UTF-8")))
        .transpose()
}

/// The value of `name` in `environments`, lists of `NAME=value`
/// assignments applied in order, so the last one wins, unless `unset`
/// (`UnsetEnvironment=`) removes it: by name, or as `NAME=value` when it
/// holds that value.
fn variable_in(environments: &[Vec<String>], unset: &[String], name: &str) -> Option<String> {
    let value = environments.iter().flatten().rev().find_map(|assignment| {
        let (key, value) = assignment.split_once('=')?;
        (key == name).then(|| value.to_owned())
    })?;
    let removed = unset.iter().any(|entry| match entry.split_once('=') {
        Some((key, removed)) => key == name && removed == value,
        None => entry == name,
    });
    (!removed).then_some(value)
}

/// The `NAME=value` assignments of an `EnvironmentFile=` file by the
/// rules such files commonly keep to: blank lines and lines starting with
/// `#` or `;` are skipped, space around the name and the value is
/// dropped, and a value in matching single or double quotes loses them.
/// `None` for a file that uses what systemd reads further (a backslash
/// escape, a continued line or a quote left open), so that it is never
/// read differently.
fn environment_file_assignments(contents: &str) -> Option<Vec<String>> {
    contents
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with(['#', ';']))
        .filter_map(|line| line.split_once('='))
        .map(|(key, value)| {
            let value = value.trim();
            let unquoted = ['"', '\'']
                .into_iter()
                .find_map(|quote| value.strip_prefix(quote)?.strip_suffix(quote))
                .unwrap_or(value);
            let plain = !value.contains('\\') && !unquoted.contains(['"', '\'']);
            plain.then(|| format!("{}={unquoted}", key.trim()))
        })
        .collect()
}

/// Removes the backend's link `link` if it is still the symlink the app
/// made, pointing where it did; anything else there is left alone.
fn remove_link(link: &PortalLink) -> Result<(), FileDialogError> {
    match fs::read_link(&link.path) {
        Ok(target) if target == link.target => remove_file(&link.path),
        _ => Ok(()),
    }
}

/// Serialises the opt-in's file changes across the app's windows.
static CHANGES: Mutex<()> = Mutex::new(());

/// Runs `operation` while no other change of the opt-in runs.
fn one_at_a_time<T>(operation: impl FnOnce() -> T) -> T {
    // A change that panicked leaves nothing locked that matters: each one
    // reads the files afresh.
    let _turn = CHANGES.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    operation()
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

    /// Two changes never overlap: the second waits for the first.
    ///
    /// parity: INT-032
    #[test]
    fn changes_run_one_at_a_time() {
        use std::sync::atomic::{AtomicBool, Ordering};
        use std::sync::{mpsc, Arc};

        let first_done = Arc::new(AtomicBool::new(false));
        let (started, wait_started) = mpsc::channel();
        let (release, held) = mpsc::channel::<()>();
        let done = Arc::clone(&first_done);
        let first = std::thread::spawn(move || {
            one_at_a_time(|| {
                started.send(()).expect("the test waits");
                held.recv().expect("the test releases");
                done.store(true, Ordering::SeqCst);
            });
        });
        wait_started.recv().expect("the first change started");
        let seen = Arc::clone(&first_done);
        let second = std::thread::spawn(move || one_at_a_time(|| seen.load(Ordering::SeqCst)));
        std::thread::sleep(std::time::Duration::from_millis(50));
        release.send(()).expect("the first change waits");
        first.join().expect("the first change");
        assert!(
            second.join().expect("the second change"),
            "the second change ran after the first finished"
        );
    }

    /// The portal's folder comes from the service manager's environment
    /// and the unit's `Environment=` lines, the unit's last line winning,
    /// as systemd applies them.
    ///
    /// parity: INT-032
    #[test]
    fn the_portal_folder_is_read_as_systemd_sets_it() {
        let lines = |lines: &[&str]| lines.iter().map(|line| (*line).to_owned()).collect::<Vec<_>>();
        let manager = lines(&["HOME=/home/someone", "XDG_DESKTOP_PORTAL_DIR=/from/manager"]);
        let unit = lines(&[
            "XDG_DATA_DIRS=/usr/share",
            "XDG_DESKTOP_PORTAL_DIR=/first",
            "XDG_DESKTOP_PORTAL_DIR=/home/someone/.local/share/xdg-desktop-portal/portals",
        ]);
        assert_eq!(
            variable_in(&[manager.clone(), unit.clone()], &[], PORTAL_DIR_VARIABLE).as_deref(),
            Some("/home/someone/.local/share/xdg-desktop-portal/portals")
        );
        assert_eq!(
            variable_in(&[manager.clone(), Vec::new()], &[], PORTAL_DIR_VARIABLE).as_deref(),
            Some("/from/manager")
        );
        let unset = lines(&["XDG_DESKTOP_PORTAL_DIR_OLD=/x", "PATH=/usr/bin"]);
        assert_eq!(variable_in(&[unset], &[], PORTAL_DIR_VARIABLE), None);
        assert_eq!(
            variable_in(&[lines(&["XDG_DESKTOP_PORTAL_DIR="])], &[], PORTAL_DIR_VARIABLE).as_deref(),
            Some(""),
            "set to nothing is set, and not absolute"
        );
        let process = b"HOME=/home/someone\0XDG_DESKTOP_PORTAL_DIR_OLD=/x\0XDG_DESKTOP_PORTAL_DIR=/running\0";
        assert_eq!(
            process_variable(process, PORTAL_DIR_VARIABLE),
            Ok(Some("/running".to_owned()))
        );
        assert_eq!(
            process_variable(b"HOME=/home/someone\0", PORTAL_DIR_VARIABLE),
            Ok(None)
        );
        assert!(
            process_variable(b"XDG_DESKTOP_PORTAL_DIR=/tmp/caf\xe9\0", PORTAL_DIR_VARIABLE).is_err(),
            "a folder that is not UTF-8 is not read as another folder"
        );
        let file = environment_file_assignments(
            "# portal folder\n; old\n\n  XDG_DESKTOP_PORTAL_DIR = \"/from/file\"  \nOTHER='x'\n",
        );
        let file = file.expect("plain assignments are read");
        assert_eq!(file, lines(&["XDG_DESKTOP_PORTAL_DIR=/from/file", "OTHER=x"]));
        for unreadable in [
            "XDG_DESKTOP_PORTAL_DIR=/tmp/portal\\ files\n",
            "XDG_DESKTOP_PORTAL_DIR=/tmp/\\\nportal\n",
            "XDG_DESKTOP_PORTAL_DIR=\"/tmp/portal\n",
        ] {
            assert_eq!(environment_file_assignments(unreadable), None, "{unreadable:?}");
        }
        assert_eq!(
            variable_in(&[manager.clone(), unit.clone(), file], &[], PORTAL_DIR_VARIABLE).as_deref(),
            Some("/from/file"),
            "an EnvironmentFile= wins over Environment="
        );
        for removing in ["XDG_DESKTOP_PORTAL_DIR", "XDG_DESKTOP_PORTAL_DIR=/from/manager"] {
            assert_eq!(
                variable_in(
                    std::slice::from_ref(&manager),
                    &lines(&[removing]),
                    PORTAL_DIR_VARIABLE
                ),
                None,
                "{removing}"
            );
        }
        assert_eq!(
            variable_in(
                &[manager],
                &lines(&["XDG_DESKTOP_PORTAL_DIR=/elsewhere"]),
                PORTAL_DIR_VARIABLE
            )
            .as_deref(),
            Some("/from/manager"),
            "UnsetEnvironment= with another value keeps it"
        );
    }

    /// The folder a restarted portal reads is the same when it is named
    /// through a symlink to it.
    ///
    /// parity: INT-032
    #[test]
    fn a_folder_named_through_a_symlink_is_the_same_folder() {
        let root = tempfile::tempdir().expect("temporary folder");
        let folder = root.path().join("portals");
        fs::create_dir(&folder).expect("folder");
        let alias = root.path().join("alias");
        std::os::unix::fs::symlink(&folder, &alias).expect("link");
        let elsewhere = root.path().join("elsewhere");
        fs::create_dir(&elsewhere).expect("folder");
        assert!(same_folder(Some(&folder), Some(&alias)));
        assert!(same_folder(None, None));
        assert!(!same_folder(Some(&folder), Some(&elsewhere)));
        assert!(!same_folder(Some(&folder), None));
        assert!(!same_folder(
            Some(&root.path().join("missing")),
            Some(&root.path().join("missing-too"))
        ));
    }

    /// A record written before the app knew the portal's folder has no
    /// link and still reads; one without a link is written as before.
    ///
    /// parity: INT-032
    #[test]
    fn an_older_record_reads_without_a_link() {
        let older = r#"{"file": "/home/someone/.config/xdg-desktop-portal/gnome-portals.conf",
                        "previous": null, "written": "[preferred]\n"}"#;
        let record: Record = serde_json::from_str(older).expect("an older record reads");
        assert_eq!(record.link, None);
        let text = serde_json::to_string(&record).expect("a record serialises");
        assert!(!text.contains("link"), "{text}");
    }

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
