// SPDX-License-Identifier: AGPL-3.0-only
//! Location parsing, validation and presentation.
//!
//! Ports `desktop/core.py` (`split_location`, `is_device_location`,
//! `_normalise_device_location`, `validate_name`, `normalise_location`,
//! `require_share`, `new_copy_name`, `safe_label`, `is_smb_server`,
//! `require_item_uri`) and the display helpers from `desktop/ui/app.js`
//! (`displayUri`, `baseName`, `parentUri`, `locationParts`, `deviceParts`,
//! `deviceRoot`, `breadcrumbSegments`, `networkLocation`, `sameLocation`,
//! `writableLocation`, `readonlyLocation`, `titleFor`).
//!
//! Folder locations are always absolute URIs: `file://`, `smb://`, or a
//! connected device (`mtp://`, `gphoto2://`, `afc://`). Their canonical
//! form is byte-for-byte what the Python app produces, because both apps
//! share `~/.config/winspace/settings.json` and compare URIs as strings.
//!
//! Besides folders, the window can show the places in [`VirtualPlace`]:
//!
//! | Place | URI | Title |
//! |---|---|---|
//! | Home page | [`HOME_URI`] (`ox:home`) | Home |
//! | This PC | [`PC_URI`] (`ox:pc`) | This PC |
//! | Settings page | [`SETTINGS_URI`] (`ox:settings`) | Settings |
//! | Network | [`NETWORK_URI`] (`network:///`) | Network |
//! | Trash | [`TRASH_URI`] (`trash:///`) | Recycle Bin |
//! | Recently used files | [`RECENT_URI`] (`recent:///`) | Recent |
//!
//! The web UI's spellings `home:`, `pc:`, `network:` and `settings:` are
//! accepted as input by [`normalise_navigation`] and [`VirtualPlace::from_uri`].
//! Use [`normalise_location`] for anything stored in settings (it rejects
//! virtual places, like the Python function) and [`normalise_navigation`]
//! for tab history, the address bar and command-line arguments.

mod display;
mod names;
mod normalise;
mod parts;
mod text;
mod virtual_place;

pub(crate) use text::{python_strip, unquote_lossy};

pub use display::{
    base_name, breadcrumbs, crumb_divider, device_root, display_location, is_network_filesystem,
    is_smb_share_root, parent_location, same_location, title_for, DeviceLabel, LocationContext,
};
pub use names::{new_copy_name, safe_label, try_new_copy_name, validate_name, MAX_LABEL_CHARS};
pub use normalise::{
    file_uri, is_smb_server, normalise, normalise_location, require_item_uri, require_share,
};
pub use parts::{split_location, LocationParts};
pub use virtual_place::{
    is_virtual_location, normalise_navigation, virtual_place, VirtualPlace, HOME_URI, NETWORK_URI, PC_URI,
    RECENT_URI, SETTINGS_URI, TRASH_URI,
};

/// GIO's schemes for phones, cameras and iOS devices. Their authorities can
/// contain brackets (`mtp://[usb:001,002]/`), which ordinary URL parsers
/// reject.
pub const DEVICE_SCHEMES: [&str; 3] = ["mtp", "gphoto2", "afc"];

/// A user-facing validation error. The message is shown as-is.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct LocationError(pub String);

impl LocationError {
    /// An error with the given user-facing message.
    pub fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }

    /// The user-facing message.
    pub fn message(&self) -> &str {
        &self.0
    }
}

/// One breadcrumb button in the address bar.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Crumb {
    /// The decoded text on the button.
    pub label: String,
    /// The location the button opens.
    pub uri: String,
}

impl Crumb {
    /// A crumb labelled `label` that opens `uri`.
    pub fn new(label: impl Into<String>, uri: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            uri: uri.into(),
        }
    }
}

/// The URI scheme, lower-cased (`file`, `smb`, `mtp`, ...), or an empty
/// string for a plain path. Follows Python's `urlsplit` rule: the text
/// before the first `:` must be a letter followed by letters, digits, `+`,
/// `-` or `.`.
pub fn scheme(uri: &str) -> String {
    parts::url_scheme(uri)
        .map(|(scheme, _)| scheme)
        .unwrap_or_default()
}

/// True for phones, cameras and iOS devices (`mtp:`, `gphoto2:`, `afc:`).
/// Unparseable input is not a device location.
pub fn is_device_location(uri: &str) -> bool {
    match split_location(uri) {
        Ok(parts) => DEVICE_SCHEMES.contains(&parts.scheme.as_str()),
        Err(_) => false,
    }
}
