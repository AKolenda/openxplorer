// SPDX-License-Identifier: AGPL-3.0-only
//! Preparing to open a file in its default application.
//!
//! Ports `prepare_default` in `desktop/native_opening.py` (OPEN-005,
//! OPEN-006). The file is queried again, its application is chosen by
//! content type, and a file on a share is handed over by its local path
//! when it has one, so every application can read it. The caller launches
//! the result on the main thread with the window's launch context, which
//! gives the application startup notification and focus (INT-023).
//!
//! The local path of a share (`local_path` in `native_opening.py`) is
//! supplied by the caller, from the network service's mount lookup; the
//! archive reader of `native_opening.py` belongs to the archive service.

use std::future::Future;
use std::path::PathBuf;

use super::activation::{choose_application, OpenError};
use super::applications::{ApplicationDatabase, ApplicationInfo, InstalledApplications};
use super::sandbox::Sandbox;
use super::worker::on_worker;
use crate::entry::{inspect, Entry, EntryKind};
use crate::location::normalise;
use crate::transfer::Cancellation;

/// The content type of a file GIO could not identify.
const UNKNOWN_CONTENT_TYPE: &str = "application/octet-stream";

/// What opens the file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Launcher {
    /// The installed application with this desktop ID.
    Application {
        /// The application's desktop ID; launch it with
        /// `gio::DesktopAppInfo::new(id)`.
        id: String,
        /// The application's name, for the "Opened with" message.
        name: String,
    },
    /// The desktop's `OpenURI` portal, which picks the host's application;
    /// used inside Flatpak, where the sandbox's applications are not the
    /// user's.
    DesktopPortal,
}

/// What the launcher receives.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OpenTarget {
    /// The file's local path, also for a share mounted through CIFS or
    /// `GVfs` FUSE.
    LocalPath(PathBuf),
    /// The file's canonical URI, for an application that reads URIs.
    Uri(String),
}

/// A file ready to be opened.
#[derive(Debug, Clone, PartialEq)]
pub struct PreparedOpen {
    /// What opens it.
    pub launcher: Launcher,
    /// What is passed to the launcher.
    pub target: OpenTarget,
    /// The file as just queried, for the recent-files list.
    pub entry: Entry,
}

/// Prepares files to open in their default applications.
///
/// `local_paths` returns the local path of a location, or `None`; the app
/// passes the network service's lookup, which knows CIFS and `GVfs` FUSE
/// mounts.
#[derive(Debug, Clone)]
pub struct DefaultOpener<L, D = InstalledApplications> {
    local_paths: L,
    applications: D,
    sandbox: Sandbox,
}

impl<L: Fn(&str) -> Option<PathBuf>> DefaultOpener<L> {
    /// Opens files with the desktop's installed applications.
    pub fn new(local_paths: L, sandbox: Sandbox) -> Self {
        Self::with_applications(local_paths, InstalledApplications, sandbox)
    }
}

impl<L, D> DefaultOpener<L, D>
where
    L: Fn(&str) -> Option<PathBuf>,
    D: ApplicationDatabase,
{
    /// Opens files with the applications of `applications`.
    pub fn with_applications(local_paths: L, applications: D, sandbox: Sandbox) -> Self {
        Self {
            local_paths,
            applications,
            sandbox,
        }
    }

    /// Queries `uri` and decides how to open it. Runs GIO synchronously;
    /// see [`DefaultOpener::prepare_in_background`].
    ///
    /// Safety rule "choose by content type, never by URI"
    /// (`prepare_default` in `native_opening.py`): asking for the handler of
    /// an `smb://` URI would return the app, the `smb://` scheme
    /// handler, which would reopen a PDF or video as a folder.
    ///
    /// # Errors
    ///
    /// [`OpenError::IsFolder`] or [`OpenError::SpecialObject`] for
    /// anything but a regular file, [`OpenError::NoApplication`] and
    /// [`OpenError::NeedsLocalPath`] when nothing can open it, and
    /// [`OpenError::Entry`] or [`OpenError::Location`] when it cannot be
    /// queried.
    pub fn prepare(&self, uri: &str, cancel: &Cancellation) -> Result<PreparedOpen, OpenError> {
        let uri = normalise(uri)?;
        let entry = inspect(&uri, Some(cancel.cancellable()))?;
        if entry.is_dir {
            return Err(OpenError::IsFolder);
        }
        if entry.kind != EntryKind::File {
            return Err(OpenError::SpecialObject);
        }
        let local_path = (self.local_paths)(&uri).filter(|path| !path.as_os_str().is_empty());
        if self.sandbox.is_flatpak() {
            return through_portal(entry, local_path);
        }
        let content_type = entry.content_type.as_deref().unwrap_or(UNKNOWN_CONTENT_TYPE);
        let default = self.applications.default_for_type(content_type);
        let application = choose_application(self.applications.all_for_type(content_type), default)?;
        if local_path.is_none() && !application.supports_uris() {
            return Err(OpenError::NeedsLocalPath);
        }
        // An application found without a desktop ID cannot be found again
        // on the main thread to be launched.
        let id = application.id().ok_or(OpenError::NoApplication)?;
        let launcher = Launcher::Application {
            id,
            name: application.display_name(),
        };
        let target = local_path.map_or(OpenTarget::Uri(uri), OpenTarget::LocalPath);
        Ok(PreparedOpen {
            launcher,
            target,
            entry,
        })
    }
}

impl<L, D> DefaultOpener<L, D>
where
    L: Fn(&str) -> Option<PathBuf> + Clone + Send + 'static,
    D: ApplicationDatabase + Clone + Send + 'static,
{
    /// [`DefaultOpener::prepare`] on a worker thread. Cancelling `cancel`
    /// aborts the query of the file.
    pub fn prepare_in_background(
        &self,
        uri: String,
        cancel: Cancellation,
    ) -> impl Future<Output = Result<PreparedOpen, OpenError>> + 'static {
        let opener = self.clone();
        on_worker(move || opener.prepare(&uri, &cancel))
    }
}

/// Opens `entry` through the desktop portal, which only accepts files the
/// sandbox can hand over by path.
fn through_portal(entry: Entry, local_path: Option<PathBuf>) -> Result<PreparedOpen, OpenError> {
    // Safety rule "never hand a share URI to the portal": the host would
    // resolve an `smb://` URI with its scheme handler, which may be
    // OpenXplorer itself.
    let path = local_path.ok_or(OpenError::NeedsLocalPath)?;
    Ok(PreparedOpen {
        launcher: Launcher::DesktopPortal,
        target: OpenTarget::LocalPath(path),
        entry,
    })
}
