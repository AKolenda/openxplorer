// SPDX-License-Identifier: AGPL-3.0-only
//! Where the default handlers of folders, SMB links and ZIP files are
//! read and set: the desktop's `xdg-mime`, or, in tests, a table in
//! memory, so no test changes the associations of the session it runs
//! in.
//!
//! Ports `DesktopIntegration._run` of `v2.0.0:desktop/desktop_integration.py`
//! as ox-core's [`MimeDefaults`] implements it; the Python tests replaced
//! `_run` with a dictionary in the same way.

#[cfg(test)]
use std::collections::BTreeMap;
#[cfg(test)]
use std::sync::{Arc, Mutex, PoisonError};

use ox_core::integration::{DefaultAppsError, DesktopId, MimeDefaults, MimeType, Sandbox, XdgMime};

/// Where default handlers live.
#[derive(Debug, Clone)]
pub(crate) enum MimeBackend {
    /// The desktop's, through `xdg-mime`.
    Desktop(XdgMime),
    /// A table shared by the test and the app.
    #[cfg(test)]
    InMemory(Arc<Mutex<BTreeMap<MimeType, String>>>),
}

impl MimeBackend {
    /// The desktop's handlers, as reached from `sandbox`.
    pub(crate) fn desktop(sandbox: Sandbox) -> Self {
        Self::Desktop(XdgMime::new(sandbox))
    }

    /// Handlers in memory, every type opened by `handler`; the test keeps
    /// a clone of the table to read and change it.
    #[cfg(test)]
    pub(crate) fn in_memory(handler: &str) -> (Self, Arc<Mutex<BTreeMap<MimeType, String>>>) {
        let handlers = MimeType::ALL
            .into_iter()
            .map(|mime_type| (mime_type, handler.to_owned()))
            .collect();
        let table = Arc::new(Mutex::new(handlers));
        (Self::InMemory(Arc::clone(&table)), table)
    }
}

impl MimeDefaults for MimeBackend {
    fn default_handler(&self, mime_type: MimeType) -> Result<String, DefaultAppsError> {
        match self {
            Self::Desktop(xdg_mime) => xdg_mime.default_handler(mime_type),
            #[cfg(test)]
            Self::InMemory(table) => {
                let table = table.lock().unwrap_or_else(PoisonError::into_inner);
                Ok(table.get(&mime_type).cloned().unwrap_or_default())
            }
        }
    }

    fn set_default_handler(&self, handler: &DesktopId, mime_type: MimeType) -> Result<(), DefaultAppsError> {
        match self {
            Self::Desktop(xdg_mime) => xdg_mime.set_default_handler(handler, mime_type),
            #[cfg(test)]
            Self::InMemory(table) => {
                let mut table = table.lock().unwrap_or_else(PoisonError::into_inner);
                table.insert(mime_type, handler.as_str().to_owned());
                Ok(())
            }
        }
    }
}
