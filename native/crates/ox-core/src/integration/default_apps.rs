// SPDX-License-Identifier: AGPL-3.0-only
//! Making the app the user's default file manager and ZIP handler,
//! and putting the previous handlers back.
//!
//! Ports `v2.0.0:desktop/desktop_integration.py`. Nothing changes unless the
//! user asks: installing the app never touches a default, reading the
//! status only queries (INT-010), and every change is per user, through
//! `xdg-mime`, never with `sudo`. Before a type is taken over its current
//! handler is recorded in `previous-defaults.json` (INT-008), and Restore
//! puts a recorded handler back only where the app is still the
//! default, so a later choice by the user wins (INT-011).
//!
//! | Module | Responsibility |
//! |---|---|
//! | `xdg_mime` | Reading and setting default handlers: [`MimeDefaults`], [`XdgMime`] |
//! | `record` | `previous-defaults.json` and [`DesktopId`] |
//! | `error` | [`DefaultAppsError`] |

mod error;
mod record;
mod xdg_mime;

use std::collections::BTreeMap;
use std::future::Future;
use std::path::{Path, PathBuf};

pub use error::DefaultAppsError;
pub use record::DesktopId;
pub use xdg_mime::{MimeDefaults, XdgMime};

use record::PreviousDefaults;

use super::mime_type::MimeType;
use super::sandbox::Sandbox;
use super::worker::on_worker;

/// The app's desktop ID. A compatibility contract (AGENTS.md): it is
/// the ID in every user's `mimeapps.list` that chose the app.
pub const APP_ID: &str = "io.winspace.Development.desktop";

/// The note Restore previous returns, word for word from the Python app.
pub const RESTORE_NOTE: &str =
    "Restored recorded handlers. Types with no previous handler must be changed in desktop settings.";

/// The record's file name in the settings folder.
const RECORD_FILE_NAME: &str = "previous-defaults.json";

/// Whether making the app the default file manager also takes over ZIP
/// files (INT-009).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ZipAssociation {
    /// Leave the ZIP handler alone; the default choice.
    Unchanged,
    /// Open ZIP files in the app too.
    Included,
}

/// Which recorded handlers Restore previous puts back.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RestoreScope {
    /// Folders, SMB links and ZIP files.
    Everything,
    /// Only the ZIP types; folders stay with the app (INT-012).
    ZipOnly,
}

/// The default handlers as the desktop reports them, and what Restore
/// previous can put back (INT-010).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DefaultsStatus {
    /// Each type's current handler exactly as reported; empty when none.
    pub current: BTreeMap<MimeType, String>,
    /// A previous ZIP handler is recorded.
    pub can_restore_zip: bool,
    /// Any previous handler is recorded.
    pub can_restore: bool,
}

impl DefaultsStatus {
    /// The current handler of `mime_type`; empty when there is none.
    pub fn handler(&self, mime_type: MimeType) -> &str {
        self.current.get(&mime_type).map_or("", String::as_str)
    }

    /// True if the app opens folders (`inode/directory`).
    pub fn is_default(&self) -> bool {
        self.is_openxplorer_default_for(&[MimeType::Directory])
    }

    /// True if the app opens folders and SMB links.
    pub fn is_default_for_folder_types(&self) -> bool {
        self.is_openxplorer_default_for(&MimeType::FOLDER_TYPES)
    }

    /// True if the app opens every ZIP type.
    pub fn is_zip_default(&self) -> bool {
        self.is_openxplorer_default_for(&MimeType::ZIP_TYPES)
    }

    /// True if the app is the handler of every one of `mime_types`.
    pub fn is_openxplorer_default_for(&self, mime_types: &[MimeType]) -> bool {
        mime_types
            .iter()
            .all(|mime_type| self.handler(*mime_type) == APP_ID)
    }
}

/// The user's default file manager and ZIP handler.
#[derive(Debug, Clone)]
pub struct DefaultApps<M = XdgMime> {
    record: PathBuf,
    mime_defaults: M,
}

impl DefaultApps<XdgMime> {
    /// The desktop's defaults, with the record kept in the settings folder
    /// `directory` (`~/.config/winspace`).
    pub fn new(directory: &Path, sandbox: Sandbox) -> Self {
        Self::with_mime_defaults(directory, XdgMime::new(sandbox))
    }
}

impl<M: MimeDefaults> DefaultApps<M> {
    /// The defaults kept by `mime_defaults`, with the record in `directory`.
    pub fn with_mime_defaults(directory: &Path, mime_defaults: M) -> Self {
        Self {
            record: directory.join(RECORD_FILE_NAME),
            mime_defaults,
        }
    }

    /// Where the previous handlers are recorded.
    pub fn record_path(&self) -> &Path {
        &self.record
    }

    /// Reads the current handlers. Only queries; nothing changes.
    ///
    /// # Errors
    ///
    /// A [`DefaultAppsError`] when the desktop cannot be asked.
    pub fn status(&self) -> Result<DefaultsStatus, DefaultAppsError> {
        let mut current = BTreeMap::new();
        for mime_type in MimeType::ALL {
            current.insert(mime_type, self.mime_defaults.default_handler(mime_type)?);
        }
        let previous = PreviousDefaults::read(&self.record);
        Ok(DefaultsStatus {
            current,
            can_restore_zip: previous.has_handler_for_any(&MimeType::ZIP_TYPES),
            can_restore: previous.has_handler_for_any(&MimeType::ALL),
        })
    }

    /// Makes the app the default for folders and SMB links, and for
    /// ZIP files only with [`ZipAssociation::Included`] (INT-008, INT-009).
    ///
    /// # Errors
    ///
    /// As [`DefaultApps::take_over`].
    pub fn make_default(&self, zip: ZipAssociation) -> Result<DefaultsStatus, DefaultAppsError> {
        match zip {
            ZipAssociation::Unchanged => self.take_over(&MimeType::FOLDER_TYPES),
            ZipAssociation::Included => self.take_over(&MimeType::ALL),
        }
    }

    /// Makes the app the default for every ZIP type and leaves the
    /// folder types alone (INT-012).
    ///
    /// # Errors
    ///
    /// As [`DefaultApps::take_over`].
    pub fn make_zip_default(&self) -> Result<DefaultsStatus, DefaultAppsError> {
        self.take_over(&MimeType::ZIP_TYPES)
    }

    /// Puts the recorded handlers in `scope` back (INT-011, INT-012).
    ///
    /// Safety rule "a later choice wins" (`restore` in
    /// `desktop_integration.py`): a handler is put back only where
    /// the app is still the default, so a change the user or another
    /// app made afterwards is kept. The record itself is kept too.
    ///
    /// # Errors
    ///
    /// [`DefaultAppsError::NoPreviousHandler`] when nothing in `scope` is
    /// recorded, or a [`DefaultAppsError`] from the desktop.
    pub fn restore(&self, scope: RestoreScope) -> Result<DefaultsStatus, DefaultAppsError> {
        let mut previous = PreviousDefaults::read(&self.record);
        if scope == RestoreScope::ZipOnly {
            previous = previous.zip_only();
        }
        if !previous.has_handler_for_any(&MimeType::ALL) {
            return Err(DefaultAppsError::NoPreviousHandler);
        }
        for (mime_type, handler) in previous.recorded_handlers() {
            if self.mime_defaults.default_handler(mime_type)? == APP_ID {
                self.mime_defaults.set_default_handler(handler, mime_type)?;
            }
        }
        self.status()
    }

    /// Records the current handlers of `mime_types`, makes the app
    /// their default and checks that the desktop reports it.
    ///
    /// Safety rule "record before replacing" (`_make` in
    /// `desktop_integration.py`): the record is saved before any default
    /// changes, and a handler the app already replaced is never
    /// recorded again, so re-applying keeps the original.
    ///
    /// # Errors
    ///
    /// [`DefaultAppsError::UnrecordableHandler`] when a current handler is
    /// not a plain desktop ID, [`DefaultAppsError::NotConfirmed`] when the
    /// desktop does not report the change, or a [`DefaultAppsError`] from
    /// saving the record or from the desktop.
    fn take_over(&self, mime_types: &[MimeType]) -> Result<DefaultsStatus, DefaultAppsError> {
        let before = self.status()?;
        let mut previous = PreviousDefaults::read(&self.record);
        for &mime_type in mime_types {
            let handler = before.handler(mime_type);
            if handler != APP_ID {
                previous.record(mime_type, recordable_handler(handler)?);
            }
        }
        previous.save(&self.record)?;
        let openxplorer = DesktopId::openxplorer();
        for &mime_type in mime_types {
            self.mime_defaults.set_default_handler(&openxplorer, mime_type)?;
        }
        let after = self.status()?;
        if !after.is_openxplorer_default_for(mime_types) {
            return Err(DefaultAppsError::NotConfirmed);
        }
        Ok(after)
    }
}

impl<M: MimeDefaults + Clone + Send + 'static> DefaultApps<M> {
    /// Runs `operation` on a worker thread, for example
    /// `defaults.run_in_background(|defaults| defaults.status())`. Each
    /// `xdg-mime` call is stopped after 8 seconds, so the operations need
    /// no cancellation.
    pub fn run_in_background<T, F>(&self, operation: F) -> impl Future<Output = T> + 'static
    where
        T: Send + 'static,
        F: FnOnce(&Self) -> T + Send + 'static,
    {
        let defaults = self.clone();
        on_worker(move || operation(&defaults))
    }
}

/// The handler to record: `None` for no handler, or its desktop ID.
///
/// # Errors
///
/// [`DefaultAppsError::UnrecordableHandler`] when `handler` is not a
/// plain desktop ID.
fn recordable_handler(handler: &str) -> Result<Option<DesktopId>, DefaultAppsError> {
    if handler.is_empty() {
        return Ok(None);
    }
    DesktopId::new(handler)
        .map(Some)
        .ok_or(DefaultAppsError::UnrecordableHandler)
}
