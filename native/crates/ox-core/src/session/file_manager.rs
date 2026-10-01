// SPDX-License-Identifier: AGPL-3.0-only
//! The arguments of a request to `org.freedesktop.FileManager1`, which
//! browsers and other applications send to show a download or a folder.
//! Ports `filemanager_request` in `v2.0.0:desktop/window_state.py`.

use super::WindowStateError;
use crate::location;

/// The most locations one request may name.
pub const MAX_REQUEST_LOCATIONS: usize = 100;

/// The three `FileManager1` methods.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileManagerMethod {
    /// Open each folder in a new tab.
    ShowFolders,
    /// Show each item selected in its folder.
    ShowItems,
    /// Open each item's folder and show its Properties.
    ShowItemProperties,
}

impl FileManagerMethod {
    /// The method named `name` on the bus.
    ///
    /// # Errors
    ///
    /// [`WindowStateError::UnsupportedMethod`] for any other name.
    pub fn from_dbus_name(name: &str) -> Result<Self, WindowStateError> {
        match name {
            "ShowFolders" => Ok(Self::ShowFolders),
            "ShowItems" => Ok(Self::ShowItems),
            "ShowItemProperties" => Ok(Self::ShowItemProperties),
            _ => Err(WindowStateError::UnsupportedMethod),
        }
    }

    /// The method's name on the bus.
    pub fn dbus_name(self) -> &'static str {
        match self {
            Self::ShowFolders => "ShowFolders",
            Self::ShowItems => "ShowItems",
            Self::ShowItemProperties => "ShowItemProperties",
        }
    }
}

/// A validated `FileManager1` request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileManagerRequest {
    /// What to do.
    pub method: FileManagerMethod,
    /// The canonical locations to do it with, in the caller's order. A
    /// file stays a file: `ShowItems` selects it in its folder.
    pub uris: Vec<String>,
}

impl FileManagerRequest {
    /// Validates the method name and locations another application sent.
    ///
    /// Safety rule "reveal requests are data, never commands"
    /// (`filemanager_request` in `v2.0.0:desktop/window_state.py`): only the three
    /// `FileManager1` methods, 1 to 100 locations, and only local, SMB and
    /// device locations; the app's own pages such as Settings, other
    /// schemes and credentials are refused, and nothing is executed.
    ///
    /// # Errors
    ///
    /// [`WindowStateError::UnsupportedMethod`],
    /// [`WindowStateError::RequestLength`], or the first location the
    /// location rules refuse.
    pub fn new(method: &str, uris: &[impl AsRef<str>]) -> Result<Self, WindowStateError> {
        let method = FileManagerMethod::from_dbus_name(method)?;
        if !(1..=MAX_REQUEST_LOCATIONS).contains(&uris.len()) {
            return Err(WindowStateError::RequestLength);
        }
        let uris = uris
            .iter()
            .map(|uri| location::normalise(uri.as_ref()))
            .collect::<Result<_, _>>()?;
        Ok(Self { method, uris })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn method_names_round_trip() {
        let methods = [
            FileManagerMethod::ShowFolders,
            FileManagerMethod::ShowItems,
            FileManagerMethod::ShowItemProperties,
        ];

        for method in methods {
            assert_eq!(FileManagerMethod::from_dbus_name(method.dbus_name()), Ok(method));
        }
        assert!(FileManagerMethod::from_dbus_name("showitems").is_err());
    }
}
