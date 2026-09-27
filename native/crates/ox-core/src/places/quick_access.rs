// SPDX-License-Identifier: AGPL-3.0-only
//! Quick access: the standard folders and the user's pins, in saved order.
//!
//! Ports the Quick access part of `environment` in `desktop/winspace.py`.
//! Standard folders come first unless the user hid them, pins follow, and
//! the saved order then ranks them all.

use std::collections::HashMap;
use std::path::PathBuf;

use crate::location::{file_uri, LocationContext};
use crate::settings::{Bookmark, SettingsData};

/// One Quick access sidebar row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Place {
    /// Visible folder name or the user's custom label.
    pub label: String,
    /// Canonical location opened by the row.
    pub uri: String,
    /// Known-folder glyph; `None` uses the ordinary folder artwork.
    pub icon: Option<&'static str>,
    /// Known-folder glyph colour, in CSS hex notation.
    pub color: Option<&'static str>,
    /// Whether the row can be unpinned, including built-in folders.
    pub pinned: bool,
    /// Whether this location is on an SMB share or a local network mount.
    pub is_shared: bool,
}

impl Place {
    /// The row for a user pin: its saved label and the folder artwork.
    fn for_pin(pin: &Bookmark) -> Self {
        Self {
            label: pin.label.clone(),
            uri: pin.uri.clone(),
            icon: None,
            color: None,
            pinned: true,
            is_shared: false,
        }
    }
}

/// A standard folder that Quick access shows before any pin.
struct KnownFolder {
    /// The XDG user directory it stands for.
    directory: glib::UserDirectory,
    /// The row's label, and the folder's name in the home folder when XDG
    /// names no directory.
    label: &'static str,
    /// The glyph drawn instead of the folder artwork.
    glyph: &'static str,
    /// The glyph's colour, in CSS hex notation.
    color: &'static str,
}

/// The standard folders in sidebar order, with the Python app's glyphs and
/// colours.
const KNOWN_FOLDERS: [KnownFolder; 6] = [
    KnownFolder {
        directory: glib::UserDirectory::Desktop,
        label: "Desktop",
        glyph: "desktop",
        color: "#3b8ec7",
    },
    KnownFolder {
        directory: glib::UserDirectory::Downloads,
        label: "Downloads",
        glyph: "downloads",
        color: "#138266",
    },
    KnownFolder {
        directory: glib::UserDirectory::Documents,
        label: "Documents",
        glyph: "documents",
        color: "#4a94d1",
    },
    KnownFolder {
        directory: glib::UserDirectory::Pictures,
        label: "Pictures",
        glyph: "pictures",
        color: "#9a79cb",
    },
    KnownFolder {
        directory: glib::UserDirectory::Music,
        label: "Music",
        glyph: "music",
        color: "#c66b9c",
    },
    KnownFolder {
        directory: glib::UserDirectory::Videos,
        label: "Videos",
        glyph: "videos",
        color: "#b48540",
    },
];

impl KnownFolder {
    /// The folder's location: its XDG directory, or the folder named
    /// [`KnownFolder::label`] in the home folder, as in Python.
    fn path(&self) -> PathBuf {
        glib::user_special_dir(self.directory).unwrap_or_else(|| glib::home_dir().join(self.label))
    }

    /// The Quick access row for this folder.
    fn to_place(&self) -> Place {
        Place {
            label: self.label.to_owned(),
            uri: file_uri(&self.path()),
            icon: Some(self.glyph),
            color: Some(self.color),
            pinned: true,
            is_shared: false,
        }
    }
}

/// The standard folders, from `glib::user_special_dir` with Python's
/// fallback names. This does not create directories or move their
/// contents.
///
/// `glib::user_special_dir` caches the XDG directories. Callers that track
/// XDG changes themselves can pass their own folder snapshot to
/// [`compose_quick_access`].
pub fn known_folders() -> Vec<Place> {
    KNOWN_FOLDERS.iter().map(KnownFolder::to_place).collect()
}

/// Known folders and user pins, respecting hidden folders and saved order.
/// For local CIFS/SMB3 mount badges, use [`compose_quick_access`] with the
/// current network mount paths.
pub fn quick_access(settings: &SettingsData) -> Vec<Place> {
    compose_quick_access(settings, &known_folders(), &[])
}

/// Combines a known-folder snapshot with settings, retaining built-in labels
/// when a pin duplicates a known folder. Unranked rows keep their input order.
/// `network_mounts` contains local CIFS/SMB3 mount points only.
pub fn compose_quick_access(
    settings: &SettingsData,
    known: &[Place],
    network_mounts: &[PathBuf],
) -> Vec<Place> {
    let mut places = visible_known_folders(known, &settings.hidden_quick);
    append_new_pins(&mut places, &settings.pins);
    sort_by_saved_order(&mut places, &settings.quick_order);
    mark_network_places(&mut places, network_mounts);
    places
}

/// The known folders the user has not hidden from Quick access.
fn visible_known_folders(known: &[Place], hidden: &[String]) -> Vec<Place> {
    known
        .iter()
        .filter(|place| !hidden.contains(&place.uri))
        .cloned()
        .collect()
}

/// Adds a row for every pin not already shown, so a pin of a known folder
/// keeps the built-in label and glyph.
fn append_new_pins(places: &mut Vec<Place>, pins: &[Bookmark]) {
    for pin in pins {
        let is_shown = places.iter().any(|place| place.uri == pin.uri);
        if !is_shown {
            places.push(Place::for_pin(pin));
        }
    }
}

/// Sorts rows by their position in `quick_order`; rows it does not list
/// go last, in their current order (the sort is stable).
fn sort_by_saved_order(places: &mut [Place], quick_order: &[String]) {
    let ranks: HashMap<&str, usize> = quick_order
        .iter()
        .enumerate()
        .map(|(rank, uri)| (uri.as_str(), rank))
        .collect();
    let unranked = ranks.len();
    places.sort_by_key(|place| ranks.get(place.uri.as_str()).copied().unwrap_or(unranked));
}

/// Sets [`Place::is_shared`] for SMB rows and rows inside a network mount.
fn mark_network_places(places: &mut [Place], network_mounts: &[PathBuf]) {
    let context = LocationContext {
        network_mounts: network_mounts.to_vec(),
        ..LocationContext::default()
    };
    for place in places {
        place.is_shared = context.network_location(&place.uri);
    }
}
