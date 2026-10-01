// SPDX-License-Identifier: AGPL-3.0-only
//! The opt-in "Show in folder" registration: the two per-user files that
//! make the session start the app's `org.freedesktop.FileManager1`
//! service.
//!
//! Ports `v2.0.0:desktop/reveal_integration.py`. Enabling writes exactly a D-Bus
//! service file and a hidden autostart entry, both running
//! `/usr/bin/winspace --filemanager-service` (INT-015); serving requests
//! without a window (INT-017) is the app's job. Nothing else changes: no
//! other file manager is stopped, no portal is replaced and nothing is
//! installed system-wide. Disabling removes only files that still hold
//! exactly what the app wrote.
//!
//! The file names, the managed marker and the `Exec` line are
//! compatibility contracts (AGENTS.md): files written by the Python app
//! are recognised as the app's own, and the reverse.
//!
//! Inside Flatpak the two files would be the sandbox's copies, which the
//! host session never reads. The Flatpak therefore keeps only its opt-in,
//! [`FLATPAK_OPT_IN_FILE`] in the settings folder: the running app owns
//! the name, which the manifest's `--own-name` allows, and asks the
//! Background portal to start it at login
//! ([`request_autostart`](super::request_autostart)), in place of the
//! autostart entry. Without a service file nothing starts it on demand, so
//! it answers from login, or from when it was started, on.

use std::fs;
use std::future::Future;
use std::io;
use std::path::{Path, PathBuf};

use super::private_file::write_private_file;
use super::sandbox::Sandbox;
use super::worker::on_worker;

/// The first line of every file the registration writes.
pub const MANAGED_MARKER: &str = "# Managed by Winspace: file-manager-integration v1\n";

/// The D-Bus service file, which lets the session bus start the service
/// when a browser asks to show a download.
pub const SERVICE_FILE: &str = "# Managed by Winspace: file-manager-integration v1\n\
[D-BUS Service]\n\
Name=org.freedesktop.FileManager1\n\
Exec=/usr/bin/winspace --filemanager-service\n";

/// The hidden autostart entry, which starts the service at login so it
/// owns the name before another file manager is activated.
pub const AUTOSTART_FILE: &str = "# Managed by Winspace: file-manager-integration v1\n\
[Desktop Entry]\n\
Type=Application\n\
Name=Winspace Show in Folder integration\n\
Comment=Handle explicit file-reveal requests without opening a window at login\n\
Exec=/usr/bin/winspace --filemanager-service\n\
Icon=io.winspace.Development\n\
NoDisplay=true\n\
X-GNOME-Autostart-enabled=true\n";

/// The Flatpak's opt-in record, which says that the running app answers
/// `org.freedesktop.FileManager1` and starts at login.
pub const FLATPAK_OPT_IN_FILE: &str = "# Managed by Winspace: file-manager-integration v1\n\
# Show in folder is enabled inside the Flatpak: the running app answers\n\
# org.freedesktop.FileManager1, and the Background portal starts it at login.\n";

/// The service file, relative to the user's data folder.
const SERVICE_PATH: &str = "dbus-1/services/org.freedesktop.FileManager1.service";

/// The autostart entry, relative to the user's configuration folder.
const AUTOSTART_PATH: &str = "autostart/io.winspace.FileManager1.desktop";

/// The record of what the two files held before, in the settings folder.
const RECORD_FILE_NAME: &str = "reveal-integration.json";

/// The Flatpak's opt-in record, in the settings folder.
const FLATPAK_OPT_IN_NAME: &str = "reveal-integration.flatpak";

/// The folders the registration uses.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RevealPaths {
    /// The settings folder (`~/.config/winspace`), which keeps the record.
    pub settings: PathBuf,
    /// The user's configuration folder (`$XDG_CONFIG_HOME`).
    pub config_home: PathBuf,
    /// The user's data folder (`$XDG_DATA_HOME`).
    pub data_home: PathBuf,
}

impl RevealPaths {
    /// The current user's XDG folders, with the record in `settings`.
    pub fn for_user(settings: &Path) -> Self {
        Self {
            settings: settings.to_owned(),
            config_home: glib::user_config_dir(),
            data_home: glib::user_data_dir(),
        }
    }
}

/// Why the registration could not be enabled or disabled. `Display` is
/// the message the Settings card shows.
#[derive(Debug, thiserror::Error)]
pub enum RevealError {
    /// One of the two files is a symlink, which is never replaced.
    #[error("Refusing to replace a symlink: {}", .0.display())]
    Symlink(PathBuf),
    /// One of the two files holds something the app did not write.
    #[error("An existing user override needs review before enabling OpenXplorer: {}", .0.display())]
    ForeignOverride(PathBuf),
    /// Reading, writing or removing a file failed.
    #[error("{error}: {}", path.display())]
    Io {
        /// The file the operation was on.
        path: PathBuf,
        /// What the operating system reported.
        error: io::Error,
    },
}

/// The result of disabling.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DisabledReveal {
    /// Files left in place because they no longer hold what the app
    /// wrote.
    pub preserved_modified_files: Vec<PathBuf>,
}

/// One file the registration manages and what it must hold.
#[derive(Debug, Clone, PartialEq, Eq)]
struct ManagedFile {
    path: PathBuf,
    contents: &'static str,
}

/// The "Show in folder" registration of one user.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RevealRegistration {
    record: PathBuf,
    files: Vec<ManagedFile>,
}

impl RevealRegistration {
    /// The registration in `paths`: the two session files on the host,
    /// the opt-in record inside Flatpak. Creating it touches nothing.
    pub fn new(paths: &RevealPaths, sandbox: Sandbox) -> Self {
        let files = match sandbox {
            Sandbox::Host => vec![
                ManagedFile {
                    path: paths.data_home.join(SERVICE_PATH),
                    contents: SERVICE_FILE,
                },
                ManagedFile {
                    path: paths.config_home.join(AUTOSTART_PATH),
                    contents: AUTOSTART_FILE,
                },
            ],
            Sandbox::Flatpak => vec![ManagedFile {
                path: paths.settings.join(FLATPAK_OPT_IN_NAME),
                contents: FLATPAK_OPT_IN_FILE,
            }],
        };
        Self {
            record: paths.settings.join(RECORD_FILE_NAME),
            files,
        }
    }

    /// The files enabling writes: the two session files, or the Flatpak's
    /// opt-in record.
    pub fn managed_files(&self) -> impl Iterator<Item = &Path> {
        self.files.iter().map(|file| file.path.as_path())
    }

    /// True if every managed file holds exactly what the app writes.
    pub fn is_enabled(&self) -> bool {
        self.files.iter().all(ManagedFile::is_installed)
    }

    /// Writes the managed files, private and atomically (INT-015).
    /// Enabling again is harmless.
    ///
    /// Safety rule "never replace a symlink or someone else's override"
    /// (`enable` in `reveal_integration.py`): every file is checked before
    /// any is written, and if a write fails the files already written are
    /// put back as they were. Safety rule "never write host files from the
    /// sandbox": inside Flatpak only the opt-in record in the settings
    /// folder is written.
    ///
    /// # Errors
    ///
    /// [`RevealError::Symlink`] or [`RevealError::ForeignOverride`] for a
    /// file that must not be replaced, and [`RevealError::Io`] when a file
    /// cannot be read or written.
    pub fn enable(&self) -> Result<(), RevealError> {
        let previous = self
            .files
            .iter()
            .map(ManagedFile::replaceable_contents)
            .collect::<Result<Vec<_>, _>>()?;
        self.record_previous(&previous)?;
        self.install_or_roll_back(&previous)
    }

    /// Removes the files that still hold exactly what the app wrote,
    /// keeps modified ones, and forgets the record.
    ///
    /// # Errors
    ///
    /// [`RevealError::Io`] when a file cannot be inspected or removed.
    pub fn disable(&self) -> Result<DisabledReveal, RevealError> {
        let mut disabled = DisabledReveal::default();
        for file in &self.files {
            match file.state()? {
                FileState::Missing => {}
                FileState::Installed => file.remove()?,
                FileState::Modified | FileState::Symlink => {
                    disabled.preserved_modified_files.push(file.path.clone());
                }
            }
        }
        remove_if_present(&self.record)?;
        Ok(disabled)
    }

    /// Runs `operation` on a worker thread, for example
    /// `registration.run_in_background(|registration| registration.enable())`.
    /// The operations only touch a few small files and need no
    /// cancellation.
    pub fn run_in_background<T, F>(&self, operation: F) -> impl Future<Output = T> + 'static
    where
        T: Send + 'static,
        F: FnOnce(&Self) -> T + Send + 'static,
    {
        let registration = self.clone();
        on_worker(move || operation(&registration))
    }

    /// Records what the files held before the first enable. An existing
    /// record is kept, so enabling again never loses the original state.
    fn record_previous(&self, previous: &[Option<String>]) -> Result<(), RevealError> {
        if self.record.exists() {
            return Ok(());
        }
        let entries: serde_json::Map<String, serde_json::Value> = self
            .files
            .iter()
            .zip(previous)
            .map(|(file, contents)| (file.path.to_string_lossy().into_owned(), contents.clone().into()))
            .collect();
        let record = serde_json::json!({ "previous": entries });
        write_text(&self.record, &record.to_string())
    }

    /// Writes every file; if one fails, restores the ones already written
    /// from `previous` and returns the failure.
    fn install_or_roll_back(&self, previous: &[Option<String>]) -> Result<(), RevealError> {
        for (index, file) in self.files.iter().enumerate() {
            if let Err(error) = write_text(&file.path, file.contents) {
                roll_back(&self.files[..index], previous);
                return Err(error);
            }
        }
        Ok(())
    }
}

/// What a managed file currently is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FileState {
    Missing,
    Installed,
    Modified,
    Symlink,
}

impl ManagedFile {
    /// Whether the file is missing, the app's, changed or a symlink.
    fn state(&self) -> Result<FileState, RevealError> {
        let metadata = match fs::symlink_metadata(&self.path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(FileState::Missing),
            Err(error) => return Err(self.io_error(error)),
        };
        if metadata.is_symlink() {
            return Ok(FileState::Symlink);
        }
        let contents = fs::read(&self.path).map_err(|error| self.io_error(error))?;
        if contents == self.contents.as_bytes() {
            Ok(FileState::Installed)
        } else {
            Ok(FileState::Modified)
        }
    }

    /// True if the file holds exactly what the app writes.
    fn is_installed(&self) -> bool {
        matches!(self.state(), Ok(FileState::Installed))
    }

    /// What the file holds now, if enabling may replace it: `None` when
    /// it is missing, or its current contents when they are already
    /// the app's.
    fn replaceable_contents(&self) -> Result<Option<String>, RevealError> {
        match self.state()? {
            FileState::Missing => Ok(None),
            FileState::Installed => Ok(Some(self.contents.to_owned())),
            FileState::Modified => Err(RevealError::ForeignOverride(self.path.clone())),
            FileState::Symlink => Err(RevealError::Symlink(self.path.clone())),
        }
    }

    fn remove(&self) -> Result<(), RevealError> {
        fs::remove_file(&self.path).map_err(|error| self.io_error(error))
    }

    fn io_error(&self, error: io::Error) -> RevealError {
        RevealError::Io {
            path: self.path.clone(),
            error,
        }
    }
}

/// Puts `files` back as `previous` records them: removed if they did not
/// exist, rewritten otherwise. Best effort, as in Python: the original
/// failure is what gets reported.
fn roll_back(files: &[ManagedFile], previous: &[Option<String>]) {
    for (file, contents) in files.iter().zip(previous) {
        let _ = match contents {
            None => remove_if_present(&file.path),
            Some(contents) => write_text(&file.path, contents),
        };
    }
}

/// Writes `text` to `path` as a private file, creating missing folders.
fn write_text(path: &Path, text: &str) -> Result<(), RevealError> {
    let io_error = |error| RevealError::Io {
        path: path.to_owned(),
        error,
    };
    if let Some(directory) = path.parent() {
        fs::create_dir_all(directory).map_err(io_error)?;
    }
    write_private_file(path, ".winspace-", text.as_bytes()).map_err(io_error)
}

/// Removes `path`; a missing file is not an error.
fn remove_if_present(path: &Path) -> Result<(), RevealError> {
    match fs::remove_file(path) {
        Err(error) if error.kind() != io::ErrorKind::NotFound => Err(RevealError::Io {
            path: path.to_owned(),
            error,
        }),
        _ => Ok(()),
    }
}
