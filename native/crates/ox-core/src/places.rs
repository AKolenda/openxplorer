// SPDX-License-Identifier: AGPL-3.0-only
//! Sidebar places: Quick access, GTK bookmarks and network locations.
//!
//! Ports the Quick access composition in `environment()` in
//! `desktop/winspace.py` (default known folders with their glyphs and
//! colours, user pins, `hiddenQuick`, `quickOrder`, network-mount detection)
//! and `merge_network_locations` in `desktop/network_locations.py`.

use gio::prelude::*;

use crate::settings::SettingsData;

/// One sidebar row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Place {
    pub label: String,
    pub uri: String,
    /// Glyph name for a known folder (`desktop`, `downloads`, ...); `None`
    /// draws the yellow folder art.
    pub icon: Option<&'static str>,
    /// Glyph colour, CSS hex.
    pub color: Option<&'static str>,
    /// Can be unpinned.
    pub pinned: bool,
    /// On a network share: drawn on the green network pipe.
    pub is_shared: bool,
}

const KNOWN: [(glib::UserDirectory, &str, &str, &str); 6] = [
    (glib::UserDirectory::Desktop, "Desktop", "desktop", "#3a8fd6"),
    (
        glib::UserDirectory::Downloads,
        "Downloads",
        "downloads",
        "#2f9a67",
    ),
    (
        glib::UserDirectory::Documents,
        "Documents",
        "documents",
        "#4a94d1",
    ),
    (glib::UserDirectory::Pictures, "Pictures", "pictures", "#8a67c8"),
    (glib::UserDirectory::Music, "Music", "music", "#d4577b"),
    (glib::UserDirectory::Videos, "Videos", "videos", "#c8892f"),
];

/// Known folders followed by user pins.
pub fn quick_access(settings: &SettingsData) -> Vec<Place> {
    let mut places: Vec<Place> = KNOWN
        .iter()
        .filter_map(|(dir, label, icon, color)| {
            glib::user_special_dir(*dir).map(|path| Place {
                label: (*label).to_string(),
                uri: gio::File::for_path(path).uri().to_string(),
                icon: Some(*icon),
                color: Some(*color),
                pinned: true,
                is_shared: false,
            })
        })
        .collect();
    for pin in &settings.pins {
        if places.iter().all(|p| p.uri != pin.uri) {
            places.push(Place {
                label: pin.label.clone(),
                uri: pin.uri.clone(),
                icon: None,
                color: None,
                pinned: true,
                is_shared: pin.uri.starts_with("smb:"),
            });
        }
    }
    places
}
