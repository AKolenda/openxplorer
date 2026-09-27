// SPDX-License-Identifier: AGPL-3.0-only
//! Sidebar composition without network I/O or automatic bookmarking.
//!
//! Ports Quick access from `environment` in `desktop/winspace.py`, the
//! standard folders from `desktop/folder_locations.py` and network
//! location merging from `desktop/network_locations.py`. Callers supply a
//! snapshot of mounts; building the sidebar never mounts a share.

mod known_folders;
mod network;
mod user_dirs;

use std::collections::HashMap;
use std::path::PathBuf;

use crate::location::LocationContext;
use crate::settings::{Bookmark, SettingsData};

pub use known_folders::{FolderLocations, KnownFolder, KnownFolderPaths};
pub use network::{
    merge_network_locations, network_key, NetworkKey, NetworkKind, NetworkLocation, NetworkMount, SavedShare,
    StableMount,
};

/// One Quick access sidebar row. Every row can be unpinned, built-in
/// folders included (a built-in folder is then hidden).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Place {
    /// Visible folder name or the user's custom label.
    pub label: String,
    /// Canonical location opened by the row.
    pub uri: String,
    /// The standard folder the row shows, which decides its glyph and
    /// colour; `None` for a user pin. Python's `folderKey`.
    pub known_folder: Option<KnownFolder>,
    /// Whether this location is on an SMB share or a local network mount.
    pub is_shared: bool,
}

impl Place {
    /// The row of a user pin: its label and the ordinary folder artwork.
    /// Whether it is shared is decided later, from the mounts.
    fn from_pin(pin: &Bookmark) -> Self {
        Self {
            label: pin.label.clone(),
            uri: pin.uri.clone(),
            known_folder: None,
            is_shared: false,
        }
    }

    /// The row's glyph name; `None` draws the ordinary folder artwork.
    pub fn glyph(&self) -> Option<&'static str> {
        self.known_folder.map(KnownFolder::glyph)
    }

    /// The glyph colour in CSS hex notation; `None` leaves the artwork's
    /// own colours.
    pub fn glyph_color(&self) -> Option<&'static str> {
        self.known_folder.and_then(KnownFolder::glyph_color)
    }
}

/// The six Quick access standard folders, read from `user-dirs.dirs` now
/// (see [`FolderLocations::read_paths`]). Creates no folder and moves
/// nothing.
pub fn known_folders() -> Vec<Place> {
    FolderLocations::from_environment()
        .read_paths()
        .quick_access_places()
}

/// Known folders and user pins, respecting hidden folders and saved order.
/// Reads `user-dirs.dirs` on every call, as the Python app does. For local
/// CIFS/SMB3 mount badges, use [`compose_quick_access`] with the current
/// network mount paths.
pub fn quick_access(settings: &SettingsData) -> Vec<Place> {
    compose_quick_access(settings, &known_folders(), &[])
}

/// Combines the known-folder rows with settings, retaining built-in labels
/// when a pin duplicates a known folder. Unranked rows keep their input
/// order. `network_mounts` contains local CIFS/SMB3 mount points only.
/// Performs no I/O.
pub fn compose_quick_access(
    settings: &SettingsData,
    known: &[Place],
    network_mounts: &[PathBuf],
) -> Vec<Place> {
    let mut places = visible_known_folders(known, &settings.hidden_quick);
    add_pins(&mut places, &settings.pins);
    sort_by_quick_order(&mut places, &settings.quick_order);
    mark_shared(&mut places, network_mounts);
    places
}

/// The known-folder rows the user has not unpinned.
fn visible_known_folders(known: &[Place], hidden: &[String]) -> Vec<Place> {
    known
        .iter()
        .filter(|place| !hidden.contains(&place.uri))
        .cloned()
        .collect()
}

/// Appends a row for every pin whose location has no row yet, so a pin of
/// a known folder keeps the folder's built-in label and glyph.
fn add_pins(places: &mut Vec<Place>, pins: &[Bookmark]) {
    for pin in pins {
        let is_listed = places.iter().any(|place| place.uri == pin.uri);
        if !is_listed {
            places.push(Place::from_pin(pin));
        }
    }
}

/// Sorts the rows into the order the user dragged them into. Rows the
/// order does not mention go last, and because the sort is stable they
/// keep their input order.
fn sort_by_quick_order(places: &mut [Place], quick_order: &[String]) {
    let ranks: HashMap<&str, usize> = quick_order
        .iter()
        .enumerate()
        .map(|(index, uri)| (uri.as_str(), index))
        .collect();
    let unranked = ranks.len();
    places.sort_by_key(|place| ranks.get(place.uri.as_str()).copied().unwrap_or(unranked));
}

/// Marks the rows on an SMB share or under a local CIFS/SMB3 mount point.
fn mark_shared(places: &mut [Place], network_mounts: &[PathBuf]) {
    let context = LocationContext {
        network_mounts: network_mounts.to_vec(),
        ..LocationContext::default()
    };
    for place in places {
        place.is_shared = context.network_location(&place.uri);
    }
}
