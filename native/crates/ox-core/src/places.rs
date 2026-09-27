// SPDX-License-Identifier: AGPL-3.0-only
//! Sidebar composition without network I/O or automatic bookmarking.
//!
//! Ports Quick access from `desktop/winspace.py::environment` and network
//! location merging from `desktop/network_locations.py`. Callers supply a
//! snapshot of mounts; building the sidebar never mounts a share.

use std::collections::HashMap;
use std::path::PathBuf;

use crate::location::{file_uri, normalise, split_location, unquote_lossy, LocationContext, LocationError};
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

const KNOWN: [(glib::UserDirectory, &str, &str, &str); 6] = [
    (glib::UserDirectory::Desktop, "Desktop", "desktop", "#3b8ec7"),
    (
        glib::UserDirectory::Downloads,
        "Downloads",
        "downloads",
        "#138266",
    ),
    (
        glib::UserDirectory::Documents,
        "Documents",
        "documents",
        "#4a94d1",
    ),
    (glib::UserDirectory::Pictures, "Pictures", "pictures", "#9a79cb"),
    (glib::UserDirectory::Music, "Music", "music", "#c66b9c"),
    (glib::UserDirectory::Videos, "Videos", "videos", "#b48540"),
];

/// Standard folders using GLib's XDG directory snapshot and Python's fallback
/// names. This does not create directories or move their contents.
///
/// GLib caches special directories. Callers that track XDG changes themselves
/// can pass their own folder snapshot to [`compose_quick_access`].
pub fn known_folders() -> Vec<Place> {
    KNOWN
        .iter()
        .map(|(directory, label, icon, color)| {
            let path = glib::user_special_dir(*directory).unwrap_or_else(|| glib::home_dir().join(label));
            Place {
                label: (*label).to_owned(),
                uri: file_uri(&path),
                icon: Some(icon),
                color: Some(color),
                pinned: true,
                is_shared: false,
            }
        })
        .collect()
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
    let mut places: Vec<Place> = known
        .iter()
        .filter(|place| !settings.hidden_quick.contains(&place.uri))
        .cloned()
        .collect();
    for pin in &settings.pins {
        if places.iter().all(|place| place.uri != pin.uri) {
            places.push(Place {
                label: pin.label.clone(),
                uri: pin.uri.clone(),
                icon: None,
                color: None,
                pinned: true,
                is_shared: false,
            });
        }
    }
    let ranks: HashMap<&str, usize> = settings
        .quick_order
        .iter()
        .enumerate()
        .map(|(index, uri)| (uri.as_str(), index))
        .collect();
    places.sort_by_key(|place| ranks.get(place.uri.as_str()).copied().unwrap_or(ranks.len()));
    let context = LocationContext {
        network_mounts: network_mounts.to_vec(),
        ..LocationContext::default()
    };
    for place in &mut places {
        place.is_shared = context.is_network_location(&place.uri);
    }
    places
}

/// An SMB mount reported by GIO. Disconnected and non-SMB mounts are ignored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NetworkMount {
    /// Root URI reported by the volume monitor.
    pub uri: String,
    /// Display name reported by the volume monitor.
    pub label: String,
    /// Whether the mount is active.
    pub mounted: bool,
}

/// A kernel mount snapshot; only CIFS and SMB3 entries are included.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StableMount {
    /// Absolute local mount point.
    pub path: PathBuf,
    /// Custom label, or empty to use the mount point's name.
    pub label: String,
    /// Filesystem type from the mount table.
    pub filesystem: String,
}

/// What a row in Network represents.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NetworkKind {
    /// A browsed SMB server listing its shares.
    Server,
    /// A saved, browsed or mounted SMB shared folder.
    Share,
    /// A local kernel CIFS/SMB3 mount point.
    Mount,
}

/// One merged Network row. Its presence alone never means credentials exist.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NetworkLocation {
    /// Canonical URI; a saved bookmark takes precedence over a mount spelling.
    pub uri: String,
    /// Visible label; a saved bookmark takes precedence over a mount label.
    pub label: String,
    /// Whether a saved network bookmark contributes to this row.
    pub saved: bool,
    /// Whether a currently active mount contributes to this row.
    pub connected: bool,
    /// Server, share or local mount, determined by its first contributor.
    pub kind: NetworkKind,
}

/// Equality key for display deduplication, never for sharing credentials.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum NetworkKey {
    /// SMB host, explicit or default port, and case-folded decoded path.
    Smb {
        /// Canonical host name; aliases are deliberately not resolved.
        host: String,
        /// Zero and absent ports follow Python's default of 445.
        port: u16,
        /// Decoded, Unicode case-folded path without a trailing slash.
        path: String,
    },
    /// Canonical URI of a non-SMB location.
    Other(String),
}

/// Canonical display identity matching Python's `network_key`.
pub fn network_key(uri: &str) -> Result<NetworkKey, LocationError> {
    let uri = normalise(uri)?;
    let parts = split_location(&uri)?;
    if parts.scheme != "smb" {
        return Ok(NetworkKey::Other(uri));
    }
    Ok(NetworkKey::Smb {
        host: parts.hostname().unwrap_or_default(),
        port: parts.port()?.filter(|port| *port != 0).unwrap_or(445),
        path: glib::casefold(unquote_lossy(&parts.path).trim_end_matches('/')).to_string(),
    })
}

/// Merges saved bookmarks, active GIO mounts, kernel mounts and visited SMB
/// locations in that order. Invalid inputs are ignored. Saved labels win,
/// connection state is combined, and host aliases remain distinct.
///
/// Each saved bookmark carries its current connection state, computed by the
/// caller from its mount snapshot. This function performs no file or network I/O.
pub fn merge_network_locations(
    saved: &[(Bookmark, bool)],
    mounts: &[NetworkMount],
    stable: &[StableMount],
    visited: &[Bookmark],
) -> Vec<NetworkLocation> {
    let mut merged = NetworkRows::default();
    for (bookmark, connected) in saved {
        merged.add(
            &bookmark.uri,
            &bookmark.label,
            true,
            *connected,
            NetworkKind::Share,
        );
    }
    for mount in mounts
        .iter()
        .filter(|mount| mount.mounted && mount.uri.starts_with("smb:"))
    {
        merged.add(&mount.uri, &mount.label, false, true, remote_kind(&mount.uri));
    }
    for mount in stable {
        if matches!(mount.filesystem.as_str(), "cifs" | "smb3") && mount.path.is_absolute() {
            let label = if mount.label.is_empty() {
                mount.path.file_name().unwrap_or_default().to_string_lossy()
            } else {
                std::borrow::Cow::Borrowed(mount.label.as_str())
            };
            merged.add(&file_uri(&mount.path), &label, false, true, NetworkKind::Mount);
        }
    }
    for bookmark in visited {
        merged.add(
            &bookmark.uri,
            &bookmark.label,
            false,
            false,
            remote_kind(&bookmark.uri),
        );
    }
    merged.rows
}

fn remote_kind(uri: &str) -> NetworkKind {
    match split_location(uri) {
        Ok(parts) if parts.path.is_empty() || parts.path == "/" => NetworkKind::Server,
        _ => NetworkKind::Share,
    }
}

#[derive(Default)]
struct NetworkRows {
    rows: Vec<NetworkLocation>,
    indexes: HashMap<NetworkKey, usize>,
}

impl NetworkRows {
    fn add(&mut self, uri: &str, label: &str, saved: bool, connected: bool, kind: NetworkKind) {
        let Ok(uri) = normalise(uri) else { return };
        if !uri.starts_with("smb:") && kind != NetworkKind::Mount {
            return;
        }
        let Ok(key) = network_key(&uri) else { return };
        let label = if label.is_empty() {
            fallback_label(&uri)
        } else {
            label.to_owned()
        };
        if let Some(&index) = self.indexes.get(&key) {
            let row = &mut self.rows[index];
            if saved {
                row.uri = uri;
                row.label = label;
            }
            row.saved |= saved;
            row.connected |= connected;
        } else {
            self.indexes.insert(key, self.rows.len());
            self.rows.push(NetworkLocation {
                uri,
                label,
                saved,
                connected,
                kind,
            });
        }
    }
}

fn fallback_label(uri: &str) -> String {
    let Ok(parts) = split_location(uri) else {
        return String::new();
    };
    let path = unquote_lossy(&parts.path);
    let name = path.trim_end_matches('/').rsplit('/').next().unwrap_or_default();
    if name.is_empty() {
        parts.hostname().unwrap_or_default()
    } else {
        name.to_owned()
    }
}
