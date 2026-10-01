// SPDX-License-Identifier: AGPL-3.0-only
//! Places that are not folder paths: the app's own pages and GIO's virtual
//! folders.
//!
//! The web UI used the bare strings `home:`, `pc:`, `network:` and
//! `settings:` (see `VIRTUAL` in `v2.0.0:desktop/window_state.py`). The native app
//! writes its own pages under an `ox:` scheme that no GIO backend claims,
//! and uses GIO's own URIs for the folders GIO can list:
//!
//! | Place | URI | Title |
//! |---|---|---|
//! | Home page (Quick access, recent files) | `ox:home` | Home |
//! | This PC (drives, devices, network) | `ox:pc` | This PC |
//! | Settings page | `ox:settings` | Settings |
//! | Network | `network:///` | Network |
//! | Trash | `trash:///` | Recycle Bin |
//! | Recently used files | `recent:///` | Recent |
//! | Recently visited folders | `ox:recent-locations` | Recent locations |
//!
//! The old spellings are accepted as input so a stored or handed-over tab
//! keeps working. Items inside the GIO folders (`trash:///folder/file`)
//! are navigable too; their paths are canonicalised one component at a
//! time so an escaped `/` inside a component (as in `recent:///` item
//! names) survives.
//!
//! The web UI's `network:` page listed saved and discovered network
//! locations. It maps to `network:///`, which GIO lists as the servers it
//! discovers; the window may draw the saved locations above that listing.
//!
//! Only [`normalise_location`] results belong in `settings.json`: the
//! Python app rejects every virtual place there, so neither app may store
//! one.

use std::path::Path;

use super::parts::split_scheme;
use super::text::{python_strip, quote_component, unquote_without_controls};
use super::{normalise_location, LocationError};

/// URI of the Home page: Quick access, shares and recently opened files.
pub const HOME_URI: &str = "ox:home";
/// URI of This PC: drives, connected devices and network locations.
pub const PC_URI: &str = "ox:pc";
/// URI of the full-page Settings.
pub const SETTINGS_URI: &str = "ox:settings";
/// GIO's list of servers discovered on the local network.
pub const NETWORK_URI: &str = "network:///";
/// GIO's Trash, shown as the Recycle Bin.
pub const TRASH_URI: &str = "trash:///";
/// GIO's recently used files.
pub const RECENT_URI: &str = "recent:///";
/// The folders visited lately, which the app lists from the desktop's
/// recently used list (SIDE-026).
pub const RECENT_LOCATIONS_URI: &str = "ox:recent-locations";

/// One of the places described in the module documentation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum VirtualPlace {
    /// The landing page.
    Home,
    /// Drives, devices and network locations.
    ThisPc,
    /// Servers on the local network.
    Network,
    /// The Trash.
    RecycleBin,
    /// Recently used files.
    Recent,
    /// Recently visited folders.
    RecentLocations,
    /// The Settings page.
    Settings,
}

impl VirtualPlace {
    /// Every place, in sidebar order.
    pub const ALL: [VirtualPlace; 7] = [
        VirtualPlace::Home,
        VirtualPlace::ThisPc,
        VirtualPlace::Network,
        VirtualPlace::RecycleBin,
        VirtualPlace::Recent,
        VirtualPlace::RecentLocations,
        VirtualPlace::Settings,
    ];

    /// The canonical URI.
    pub fn uri(self) -> &'static str {
        match self {
            VirtualPlace::Home => HOME_URI,
            VirtualPlace::ThisPc => PC_URI,
            VirtualPlace::Network => NETWORK_URI,
            VirtualPlace::RecycleBin => TRASH_URI,
            VirtualPlace::Recent => RECENT_URI,
            VirtualPlace::RecentLocations => RECENT_LOCATIONS_URI,
            VirtualPlace::Settings => SETTINGS_URI,
        }
    }

    /// The Explorer wording used for titles, tabs and the address bar.
    pub fn title(self) -> &'static str {
        match self {
            VirtualPlace::Home => "Home",
            VirtualPlace::ThisPc => "This PC",
            VirtualPlace::Network => "Network",
            VirtualPlace::RecycleBin => "Recycle Bin",
            VirtualPlace::Recent => "Recent",
            VirtualPlace::RecentLocations => "Recent locations",
            VirtualPlace::Settings => "Settings",
        }
    }

    /// True for pages the app draws or lists itself; false for folders GIO
    /// lists.
    pub fn is_page(self) -> bool {
        self.gio_scheme().is_none()
    }

    /// The GIO scheme of a listable virtual folder.
    pub fn gio_scheme(self) -> Option<&'static str> {
        match self {
            VirtualPlace::Network => Some("network"),
            VirtualPlace::RecycleBin => Some("trash"),
            VirtualPlace::Recent => Some("recent"),
            VirtualPlace::Home
            | VirtualPlace::ThisPc
            | VirtualPlace::RecentLocations
            | VirtualPlace::Settings => None,
        }
    }

    /// The place `uri` is the root of, including the web UI's spellings
    /// (`home:`, `pc:`, `network:`, `settings:`) and root variants such as
    /// `trash:`. Schemes are case-insensitive, as in every URI. `None` for
    /// items inside a virtual folder.
    pub fn from_uri(uri: &str) -> Option<Self> {
        if let Some(page) = Self::page_from_uri(uri) {
            return Some(page);
        }
        let Ok(Some(folder)) = VirtualFolder::parse(uri) else {
            return None;
        };
        folder.segments.is_empty().then_some(folder.place)
    }

    /// The place whose title was typed into the address bar ("This PC",
    /// "recycle bin"), ignoring case and surrounding spaces. GNOME's name
    /// "Trash" also means the Recycle Bin.
    ///
    /// The address bar shows these titles for the places, so pressing
    /// Enter on an unchanged address must stay put. A typed title can also
    /// be a folder name relative to the current folder, so prefer an
    /// existing folder of that name and fall back to this.
    pub fn from_title(text: &str) -> Option<Self> {
        let typed = python_strip(text).to_lowercase();
        if typed == "trash" {
            return Some(VirtualPlace::RecycleBin);
        }
        Self::ALL
            .into_iter()
            .find(|place| place.title().to_lowercase() == typed)
    }

    /// The app page `uri` names, in the native (`ox:home`) or the web UI's
    /// (`home:`) spelling.
    fn page_from_uri(uri: &str) -> Option<Self> {
        match uri.to_ascii_lowercase().as_str() {
            HOME_URI | "home:" => Some(VirtualPlace::Home),
            PC_URI | "pc:" => Some(VirtualPlace::ThisPc),
            SETTINGS_URI | "settings:" => Some(VirtualPlace::Settings),
            RECENT_LOCATIONS_URI => Some(VirtualPlace::RecentLocations),
            _ => None,
        }
    }

    /// The virtual folder GIO lists under `scheme`, which must be
    /// lower-case: the inverse of [`gio_scheme`](Self::gio_scheme).
    fn from_gio_scheme(scheme: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|place| place.gio_scheme() == Some(scheme))
    }
}

/// True for the app's pages and for anything inside `trash:`, `recent:` or
/// `network:`. Such locations are never writable folders.
pub fn is_virtual_location(uri: &str) -> bool {
    VirtualPlace::from_uri(uri).is_some() || is_in_virtual_folder(uri)
}

/// True for anything with a `trash:`, `recent:` or `network:` scheme, even
/// an address that cannot be opened: such a location is never writable.
pub(crate) fn is_in_virtual_folder(uri: &str) -> bool {
    let Some((scheme, _)) = split_scheme(uri) else {
        return false;
    };
    VirtualPlace::from_gio_scheme(&scheme).is_some()
}

/// Normalises a location the app can navigate to: everything
/// [`normalise_location`] accepts plus the virtual places. Use it for the
/// tab history and command-line arguments; the port of `location()` in
/// `v2.0.0:desktop/window_state.py`.
///
/// # Errors
///
/// As [`normalise_location`]. A `trash:`, `recent:` or `network:` location
/// fails when it has a query, fragment or server name, or a component that
/// does not decode or decodes to a control character.
pub fn normalise_navigation(address: &str, base: Option<&str>, home: &Path) -> Result<String, LocationError> {
    let trimmed = python_strip(address);
    if let Some(place) = VirtualPlace::from_uri(trimmed) {
        return Ok(place.uri().to_string());
    }
    if let Some(folder) = VirtualFolder::parse(trimmed)? {
        return Ok(folder.uri());
    }
    normalise_location(address, base, home)
}

/// A location inside one of GIO's virtual folders, split into decoded path
/// components.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct VirtualFolder {
    /// The Network, Recycle Bin or Recent folder the location is in.
    pub(crate) place: VirtualPlace,
    /// Decoded components; `.` and `..` already resolved.
    pub(crate) segments: Vec<String>,
}

impl VirtualFolder {
    /// Splits a `trash:`, `recent:` or `network:` location.
    ///
    /// `Ok(None)` when `uri` has another scheme. To ask only whether `uri`
    /// is virtual, use [`is_in_virtual_folder`].
    ///
    /// # Errors
    ///
    /// When `uri` is in a virtual folder but cannot be canonicalised: it
    /// has a query, fragment or server name, or a component that does not
    /// decode or decodes to a control character. Navigation then shows the
    /// error instead of treating `uri` as a folder path.
    pub(crate) fn parse(uri: &str) -> Result<Option<Self>, LocationError> {
        let Some((scheme, after_scheme)) = split_scheme(uri) else {
            return Ok(None);
        };
        let Some(place) = VirtualPlace::from_gio_scheme(&scheme) else {
            return Ok(None);
        };
        Self::parse_path(place, after_scheme).map(Some)
    }

    /// The folder of `after_scheme`, the text after `trash:`, `recent:` or
    /// `network:`.
    fn parse_path(place: VirtualPlace, after_scheme: &str) -> Result<Self, LocationError> {
        if after_scheme.contains(['?', '#']) {
            return Err(LocationError::query_or_fragment());
        }
        let path = strip_empty_authority(place, after_scheme)?;
        let segments = decode_segments(path)?;
        Ok(Self { place, segments })
    }

    /// The canonical URI, each component escaped with Python's
    /// `quote(component, safe='')`.
    pub(crate) fn uri(&self) -> String {
        let escaped: Vec<String> = self
            .segments
            .iter()
            .map(|segment| quote_component(segment))
            .collect();
        format!("{}{}", self.place.uri(), escaped.join("/"))
    }
}

/// The path of `trash:///a` or `trash:a`. GIO's virtual folders have no
/// server, so `trash://host/a` is refused.
fn strip_empty_authority(place: VirtualPlace, after_scheme: &str) -> Result<&str, LocationError> {
    let Some(after_slashes) = after_scheme.strip_prefix("//") else {
        return Ok(after_scheme);
    };
    let has_authority = !after_slashes.is_empty() && !after_slashes.starts_with('/');
    if has_authority {
        return Err(LocationError::new(format!(
            "Use {} without a server name.",
            place.uri()
        )));
    }
    Ok(after_slashes)
}

/// Decodes the non-empty components of `path` and resolves `.` and `..`.
/// Each component is decoded on its own, so an escaped `/` inside one (as
/// in `recent:///` item names) stays part of it.
fn decode_segments(path: &str) -> Result<Vec<String>, LocationError> {
    let mut segments = Vec::new();
    for escaped in path.split('/').filter(|escaped| !escaped.is_empty()) {
        let segment = unquote_without_controls(escaped)?;
        match segment.as_str() {
            "." => {}
            ".." => {
                segments.pop();
            }
            _ => segments.push(segment),
        }
    }
    Ok(segments)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn navigate(address: &str) -> Result<String, LocationError> {
        normalise_navigation(address, None, Path::new("/home/test"))
    }

    #[test]
    fn places_round_trip_through_their_uris() {
        for place in VirtualPlace::ALL {
            assert_eq!(VirtualPlace::from_uri(place.uri()), Some(place));
            assert_eq!(navigate(place.uri()).as_deref(), Ok(place.uri()));
        }
        assert!(VirtualPlace::Home.is_page());
        assert!(!VirtualPlace::RecycleBin.is_page());
    }

    #[test]
    fn gio_folders_round_trip_through_their_schemes() {
        for place in VirtualPlace::ALL {
            let from_scheme = place.gio_scheme().and_then(VirtualPlace::from_gio_scheme);
            let expected = (!place.is_page()).then_some(place);
            assert_eq!(from_scheme, expected, "{place:?}");
        }
        assert_eq!(VirtualPlace::from_gio_scheme("file"), None);
    }

    #[test]
    fn web_ui_spellings_are_accepted() {
        assert_eq!(navigate("home:").as_deref(), Ok(HOME_URI));
        assert_eq!(navigate("pc:").as_deref(), Ok(PC_URI));
        assert_eq!(navigate("network:").as_deref(), Ok(NETWORK_URI));
        assert_eq!(navigate("settings:").as_deref(), Ok(SETTINGS_URI));
        assert_eq!(navigate("Trash:").as_deref(), Ok(TRASH_URI));
        assert_eq!(navigate(" recent:// ").as_deref(), Ok(RECENT_URI));
        assert_eq!(navigate("OX:Home").as_deref(), Ok(HOME_URI));
        assert_eq!(navigate("PC:").as_deref(), Ok(PC_URI));
    }

    #[test]
    fn typed_titles_name_their_places() {
        for place in VirtualPlace::ALL {
            assert_eq!(VirtualPlace::from_title(place.title()), Some(place));
        }
        assert_eq!(VirtualPlace::from_title("  this pc "), Some(VirtualPlace::ThisPc));
        assert_eq!(
            VirtualPlace::from_title("RECYCLE BIN"),
            Some(VirtualPlace::RecycleBin)
        );
        assert_eq!(VirtualPlace::from_title("Trash"), Some(VirtualPlace::RecycleBin));
        assert_eq!(VirtualPlace::from_title("Documents"), None);
        assert_eq!(VirtualPlace::from_title(""), None);
    }

    #[test]
    fn virtual_folder_items_are_canonical() {
        assert_eq!(
            navigate("trash:///a b/./c/../d").as_deref(),
            Ok("trash:///a%20b/d")
        );
        assert_eq!(navigate("trash:///..").as_deref(), Ok(TRASH_URI));
        let recent_item = "recent:///file%3A%2F%2F%2Fhome%2Fu%2Fa.txt";
        assert_eq!(navigate(recent_item).as_deref(), Ok(recent_item));
        assert_eq!(VirtualPlace::from_uri("trash:///a"), None);
        assert!(is_virtual_location("trash:///a"));
        assert!(is_virtual_location(PC_URI));
        assert!(!is_virtual_location("file:///"));
    }

    /// A malformed address in a virtual folder cannot be opened, but it
    /// still names a virtual folder, which is never writable.
    #[test]
    fn malformed_items_are_still_in_their_virtual_folder() {
        for uri in ["trash:///a", "Recent:///%FF", "trash://host/", "network:x?y"] {
            assert!(is_in_virtual_folder(uri), "{uri}");
            assert!(is_virtual_location(uri), "{uri}");
        }
        for uri in [HOME_URI, "file:///", "/tmp", "smb://nas/"] {
            assert!(!is_in_virtual_folder(uri), "{uri}");
        }
    }

    #[test]
    fn malformed_virtual_addresses_are_rejected() {
        for bad in [
            "trash://host/",
            "trash:///a?b",
            "trash:///%00",
            "recent:///%FF",
            "ox:bogus",
        ] {
            assert!(navigate(bad).is_err(), "{bad} should be rejected");
        }
        // Real folders still go through the Python rules.
        assert_eq!(navigate("\\\\NAS\\x").as_deref(), Ok("smb://nas/x"));
    }
}
