// SPDX-License-Identifier: AGPL-3.0-only
//! Checking a request that another application sends through
//! `org.freedesktop.FileManager1`, or that `--select` makes.
//!
//! Ports `filemanager_request` in `desktop/window_state.py`. Only the
//! three methods of the interface are accepted, with 1 to 100 locations,
//! and every location is normalised like one typed in the address bar:
//! a request carries data to show, never a command to run.

use crate::location::{normalise, LocationError};

/// The most locations one request may name, as in the Python app.
pub const MAX_REQUEST_LOCATIONS: usize = 100;

/// A method of the `org.freedesktop.FileManager1` interface.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileManagerMethod {
    /// Open each folder in a tab.
    ShowFolders,
    /// Open each item's folder with the item selected.
    ShowItems,
    /// Open each item's folder and show the item's properties.
    ShowItemProperties,
}

impl FileManagerMethod {
    /// Every method, in the order the interface declares them.
    pub const ALL: [Self; 3] = [Self::ShowFolders, Self::ShowItems, Self::ShowItemProperties];

    /// The method's D-Bus name, for example `ShowItems`.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ShowFolders => "ShowFolders",
            Self::ShowItems => "ShowItems",
            Self::ShowItemProperties => "ShowItemProperties",
        }
    }

    /// The method with D-Bus name `name`, or `None` for any other name.
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|method| method.as_str() == name)
    }
}

/// Why a request was refused. `Display` is the message returned to the
/// caller as `org.freedesktop.DBus.Error.InvalidArgs`.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum FileManagerRequestError {
    /// The method is not one of the three the interface has.
    #[error("Unsupported method.")]
    UnsupportedMethod,
    /// The request names no location, or more than 100.
    #[error("Expected 1–100 file locations.")]
    WrongLocationCount,
    /// A location is not one the app can open.
    #[error(transparent)]
    Location(#[from] LocationError),
}

/// A checked request: a method and its normalised locations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileManagerRequest {
    method: FileManagerMethod,
    uris: Vec<String>,
}

impl FileManagerRequest {
    /// Checks a request for `method` on `locations` (INT-013).
    ///
    /// # Errors
    ///
    /// [`FileManagerRequestError::WrongLocationCount`] for no location or
    /// more than [`MAX_REQUEST_LOCATIONS`], and
    /// [`FileManagerRequestError::Location`] for a location that is not a
    /// local, SMB or device location, including the app's own pages.
    pub fn new<S: AsRef<str>>(
        method: FileManagerMethod,
        locations: &[S],
    ) -> Result<Self, FileManagerRequestError> {
        if !(1..=MAX_REQUEST_LOCATIONS).contains(&locations.len()) {
            return Err(FileManagerRequestError::WrongLocationCount);
        }
        let uris = locations
            .iter()
            .map(|location| normalise(location.as_ref()))
            .collect::<Result<_, _>>()?;
        Ok(Self { method, uris })
    }

    /// Checks a request whose method arrived by name over D-Bus.
    ///
    /// # Errors
    ///
    /// [`FileManagerRequestError::UnsupportedMethod`] for an unknown
    /// method, otherwise as [`FileManagerRequest::new`].
    pub fn from_method_name<S: AsRef<str>>(
        method: &str,
        locations: &[S],
    ) -> Result<Self, FileManagerRequestError> {
        let method =
            FileManagerMethod::from_name(method).ok_or(FileManagerRequestError::UnsupportedMethod)?;
        Self::new(method, locations)
    }

    /// What to do with the locations.
    pub fn method(&self) -> FileManagerMethod {
        self.method
    }

    /// The canonical URIs of the locations, in the order they were sent.
    pub fn uris(&self) -> &[String] {
        &self.uris
    }
}
