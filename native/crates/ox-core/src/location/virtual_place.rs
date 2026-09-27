// SPDX-License-Identifier: AGPL-3.0-only
//! Places that are not folder paths: the app's own pages and GIO's virtual
//! folders.
//!
//! The web UI used the bare strings `home:`, `pc:`, `network:` and
//! `settings:` (see `VIRTUAL` in `desktop/window_state.py`). The native app
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

use super::parts::url_scheme;
use super::text::{has_control, python_strip, quote_component, unquote_strict};
use super::{normalise_location, LocationError};
use std::path::Path;

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
    /// The Settings page.
    Settings,
}

impl VirtualPlace {
    /// Every place, in sidebar order.
    pub const ALL: [VirtualPlace; 6] = [
        VirtualPlace::Home,
        VirtualPlace::ThisPc,
        VirtualPlace::Network,
        VirtualPlace::RecycleBin,
        VirtualPlace::Recent,
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
            VirtualPlace::Settings => "Settings",
        }
    }

    /// True for pages the app draws itself; false for folders GIO lists.
    pub fn is_page(self) -> bool {
        self.gio_scheme().is_none()
    }

    /// The GIO scheme of a listable virtual folder.
    pub fn gio_scheme(self) -> Option<&'static str> {
        match self {
            VirtualPlace::Network => Some("network"),
            VirtualPlace::RecycleBin => Some("trash"),
            VirtualPlace::Recent => Some("recent"),
            VirtualPlace::Home | VirtualPlace::ThisPc | VirtualPlace::Settings => None,
        }
    }

    /// The place `uri` is the root of, including the web UI's spellings
    /// (`home:`, `pc:`, `network:`, `settings:`) and root variants such as
    /// `trash:`. Schemes are case-insensitive, as in every URI. `None` for
    /// items inside a virtual folder.
    pub fn from_uri(uri: &str) -> Option<Self> {
        match uri.to_ascii_lowercase().as_str() {
            HOME_URI | "home:" => return Some(VirtualPlace::Home),
            PC_URI | "pc:" => return Some(VirtualPlace::ThisPc),
            SETTINGS_URI | "settings:" => return Some(VirtualPlace::Settings),
            _ => {}
        }
        let folder = VirtualFolder::parse(uri)?.ok()?;
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

    fn from_gio_scheme(scheme: &str) -> Option<Self> {
        match scheme {
            "network" => Some(VirtualPlace::Network),
            "trash" => Some(VirtualPlace::RecycleBin),
            "recent" => Some(VirtualPlace::Recent),
            _ => None,
        }
    }
}

/// The virtual place `uri` is, or `None`. Same as [`VirtualPlace::from_uri`].
pub fn virtual_place(uri: &str) -> Option<VirtualPlace> {
    VirtualPlace::from_uri(uri)
}

/// True for the app's pages and for anything inside `trash:`, `recent:` or
/// `network:`. Such locations are never writable folders.
pub fn is_virtual_location(uri: &str) -> bool {
    VirtualPlace::from_uri(uri).is_some() || VirtualFolder::parse(uri).is_some()
}

/// Normalises a location the app can navigate to: everything
/// [`normalise_location`] accepts plus the virtual places. Use it for the
/// tab history and command-line arguments; the port of `location()` in
/// `desktop/window_state.py`.
pub fn normalise_navigation(value: &str, base: Option<&str>, home: &Path) -> Result<String, LocationError> {
    let trimmed = python_strip(value);
    if let Some(place) = VirtualPlace::from_uri(trimmed) {
        return Ok(place.uri().to_string());
    }
    if let Some(folder) = VirtualFolder::parse(trimmed) {
        return folder.map(|folder| folder.uri());
    }
    normalise_location(value, base, home)
}

/// A location inside one of GIO's virtual folders, split into decoded path
/// components.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct VirtualFolder {
    pub place: VirtualPlace,
    /// Decoded components; `.` and `..` already resolved.
    pub segments: Vec<String>,
}

impl VirtualFolder {
    /// `None` when `uri` is not `trash:`, `recent:` or `network:`; an error
    /// when it is but cannot be canonicalised.
    pub fn parse(uri: &str) -> Option<Result<Self, LocationError>> {
        let (scheme, rest) = url_scheme(uri)?;
        let place = VirtualPlace::from_gio_scheme(&scheme)?;
        Some(Self::parse_path(place, rest))
    }

    fn parse_path(place: VirtualPlace, rest: &str) -> Result<Self, LocationError> {
        if rest.contains(['?', '#']) {
            return Err(LocationError::new(
                "In a URL, encode “?” as %3F and “#” as %23, or enter a normal file/UNC path.",
            ));
        }
        let path = match rest.strip_prefix("//") {
            Some(after_slashes) => {
                let authority_end = after_slashes.find('/').unwrap_or(after_slashes.len());
                if authority_end > 0 {
                    return Err(LocationError::new(format!(
                        "Use {} without a server name.",
                        place.uri()
                    )));
                }
                after_slashes
            }
            None => rest,
        };
        let mut segments: Vec<String> = Vec::new();
        for raw in path.split('/').filter(|raw| !raw.is_empty()) {
            let segment = unquote_strict(raw)?;
            if has_control(&segment) {
                return Err(LocationError::new("Encoded control characters are not allowed."));
            }
            match segment.as_str() {
                "." => {}
                ".." => {
                    segments.pop();
                }
                _ => segments.push(segment),
            }
        }
        Ok(Self { place, segments })
    }

    /// The canonical URI, each component escaped with Python's
    /// `quote(component, safe='')`.
    pub fn uri(&self) -> String {
        let escaped: Vec<String> = self
            .segments
            .iter()
            .map(|segment| quote_component(segment))
            .collect();
        format!("{}{}", self.place.uri(), escaped.join("/"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn navigate(value: &str) -> Result<String, LocationError> {
        normalise_navigation(value, None, Path::new("/home/test"))
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
