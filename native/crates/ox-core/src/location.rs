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
//!
//! | Module | Responsibility |
//! |---|---|
//! | `text` | Python's and JavaScript's escaping, stripping and path rules |
//! | `parts` | Splitting a location like `urlsplit` |
//! | `normalise` | One canonical URI for a typed or stored address |
//! | `virtual_place` | The app's pages and GIO's virtual folders |
//! | `names` | File names, "Keep both" names and sidebar labels |
//! | `display` | Titles, address bar text, breadcrumbs and Up |
//! | `classify` | Writable, snapshot and network folders, SMB share roots |

mod classify;
mod display;
mod names;
mod normalise;
mod parts;
mod text;
mod virtual_place;

pub(crate) use text::{decode_uri_component, python_strip, unquote_lossy};

pub use classify::{is_network_filesystem, is_smb_share_root};
pub use display::{crumb_divider, device_root, parent_location, same_location, DeviceLabel, LocationContext};
pub use names::{new_copy_name, safe_label, validate_name, ItemKind, MAX_LABEL_CHARS};
pub use normalise::{
    file_uri, is_smb_server, normalise, normalise_location, require_item_uri, require_share,
};
pub use parts::{split_location, LocationKind, LocationParts};
pub use virtual_place::{
    is_virtual_location, normalise_navigation, VirtualPlace, HOME_URI, NETWORK_URI, PC_URI, RECENT_URI,
    SETTINGS_URI, TRASH_URI,
};

/// A user-facing validation error. The message is shown as-is.
///
/// Where the Rust port refuses what a `raise` in `desktop/core.py` refuses,
/// the message is the Python app's, word for word; `location_python.rs`
/// checks every one of them. Where Python's standard library refused an
/// address in its own words, the message is in the app's wording instead.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{message}")]
pub struct LocationError {
    /// The text shown to the user.
    message: String,
}

impl LocationError {
    /// An error with the given user-facing message. Only this crate
    /// creates location errors, so every message is one of its own.
    pub(crate) fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }

    /// The user-facing message.
    pub fn message(&self) -> &str {
        &self.message
    }

    /// The user-facing message, for errors that wrap this one.
    pub fn into_message(self) -> String {
        self.message
    }

    /// A `?` or `#` in a URL. `urlsplit` would cut the path there, so the
    /// user must escape them or type a plain path, which may hold both.
    pub(crate) fn query_or_fragment() -> Self {
        Self::new("In a URL, encode “?” as %3F and “#” as %23, or enter a normal file/UNC path.")
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

/// The URI scheme, lower-cased (`file`, `smb`, `mtp`, ...), or `None` for
/// a plain path. Follows Python's `urlsplit` rule: the text before the
/// first `:` must be a letter followed by letters, digits, `+`, `-` or `.`.
pub fn scheme(uri: &str) -> Option<String> {
    parts::split_scheme(uri).map(|(scheme, _)| scheme)
}

/// What kind of place `uri` names, by its scheme; [`LocationKind::Other`]
/// for plain paths and schemes the app does not browse as folders.
pub fn location_kind(uri: &str) -> LocationKind {
    scheme(uri).map_or(LocationKind::Other, |scheme| LocationKind::from_scheme(&scheme))
}

/// True for phones, cameras and iOS devices (`mtp:`, `gphoto2:`, `afc:`).
/// Unparseable input is not a device location.
pub fn is_device_location(uri: &str) -> bool {
    split_location(uri).is_ok_and(|parts| parts.is_device())
}
