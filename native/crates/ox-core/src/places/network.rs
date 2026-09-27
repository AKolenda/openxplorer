// SPDX-License-Identifier: AGPL-3.0-only
//! The Network list: saved shares, active mounts and visited servers.
//!
//! Ports `network_key` and `merge_network_locations` in
//! `desktop/network_locations.py`. Callers pass snapshots of their mounts
//! and of the servers browsed this session; merging performs no file or
//! network I/O and never saves a bookmark. Port 445 is filled in, but host
//! aliases are never guessed, so a row never implies shared credentials.

use std::borrow::Cow;
use std::collections::hash_map::Entry;
use std::collections::HashMap;
use std::path::PathBuf;

use crate::location::{file_uri, normalise, split_location, unquote_lossy, LocationError};
use crate::settings::Bookmark;

/// SMB's port, which Python fills in when a URI has none or port 0.
const DEFAULT_SMB_PORT: u16 = 445;

/// A saved network share, and whether one of the caller's current mounts
/// serves it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SavedShare {
    /// The share as saved in settings.
    pub bookmark: Bookmark,
    /// Whether a current mount serves the share.
    pub connected: bool,
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
///
/// # Errors
///
/// The [`LocationError`] of a URI that is not a supported location, or of
/// an SMB URI whose port is not a number.
pub fn network_key(uri: &str) -> Result<NetworkKey, LocationError> {
    let uri = normalise(uri)?;
    let parts = split_location(&uri)?;
    if parts.scheme != "smb" {
        return Ok(NetworkKey::Other(uri));
    }
    let port = parts.port()?.filter(|port| *port != 0);
    let decoded_path = unquote_lossy(&parts.path);
    let path = glib::casefold(decoded_path.trim_end_matches('/'));
    Ok(NetworkKey::Smb {
        host: parts.hostname().unwrap_or_default(),
        port: port.unwrap_or(DEFAULT_SMB_PORT),
        path: path.into(),
    })
}

/// Merges saved shares, active GIO mounts, kernel mounts and visited SMB
/// locations in that order. Invalid inputs are ignored. Saved labels win,
/// connection state is combined, and host aliases remain distinct.
///
/// Each saved share carries its current connection state, computed by the
/// caller from its mount snapshot. This function performs no file or
/// network I/O.
pub fn merge_network_locations(
    saved: &[SavedShare],
    mounts: &[NetworkMount],
    stable: &[StableMount],
    visited: &[Bookmark],
) -> Vec<NetworkLocation> {
    let saved = saved.iter().map(Contribution::from_saved_share);
    let gio_mounts = mounts
        .iter()
        .filter(|mount| mount.is_active_smb_mount())
        .map(Contribution::from_gio_mount);
    let kernel_mounts = stable
        .iter()
        .filter(|mount| mount.is_smb_mount_point())
        .map(Contribution::from_kernel_mount);
    let visited = visited.iter().map(Contribution::from_visited);
    let mut merged = NetworkRows::default();
    for contribution in saved.chain(gio_mounts).chain(kernel_mounts).chain(visited) {
        merged.add(&contribution);
    }
    merged.rows
}

impl NetworkMount {
    /// Only active SMB mounts contribute a Network row.
    fn is_active_smb_mount(&self) -> bool {
        self.mounted && self.uri.starts_with("smb:")
    }
}

impl StableMount {
    /// Only CIFS and SMB3 mounts at an absolute path contribute a Network
    /// row, as in Python.
    fn is_smb_mount_point(&self) -> bool {
        let is_smb_filesystem = matches!(self.filesystem.as_str(), "cifs" | "smb3");
        is_smb_filesystem && self.path.is_absolute()
    }

    /// The custom label, or the mount point's folder name.
    fn display_label(&self) -> Cow<'_, str> {
        if self.label.is_empty() {
            self.path.file_name().unwrap_or_default().to_string_lossy()
        } else {
            Cow::Borrowed(&self.label)
        }
    }
}

/// One input's claim on a Network row, before validation.
struct Contribution<'a> {
    uri: Cow<'a, str>,
    /// Empty to name the row after its location.
    label: Cow<'a, str>,
    /// A saved bookmark, whose URI and label replace a mount's spelling.
    saved: bool,
    /// An active mount serves the location.
    connected: bool,
    /// The row's kind, if this contribution creates it.
    kind: NetworkKind,
}

impl<'a> Contribution<'a> {
    fn from_saved_share(share: &'a SavedShare) -> Self {
        Self {
            uri: Cow::Borrowed(&share.bookmark.uri),
            label: Cow::Borrowed(&share.bookmark.label),
            saved: true,
            connected: share.connected,
            kind: NetworkKind::Share,
        }
    }

    fn from_gio_mount(mount: &'a NetworkMount) -> Self {
        Self {
            uri: Cow::Borrowed(&mount.uri),
            label: Cow::Borrowed(&mount.label),
            saved: false,
            connected: true,
            kind: smb_location_kind(&mount.uri),
        }
    }

    fn from_kernel_mount(mount: &'a StableMount) -> Self {
        Self {
            uri: Cow::Owned(file_uri(&mount.path)),
            label: mount.display_label(),
            saved: false,
            connected: true,
            kind: NetworkKind::Mount,
        }
    }

    /// A server or share browsed this session: listed until the app quits,
    /// even before GIO reports a mount for it.
    fn from_visited(bookmark: &'a Bookmark) -> Self {
        Self {
            uri: Cow::Borrowed(&bookmark.uri),
            label: Cow::Borrowed(&bookmark.label),
            saved: false,
            connected: false,
            kind: smb_location_kind(&bookmark.uri),
        }
    }

    /// The validated row this contribution stands for; `None` for an
    /// invalid location, or one that is neither SMB nor a kernel mount.
    fn to_location(&self) -> Option<NetworkLocation> {
        let uri = normalise(&self.uri).ok()?;
        let is_network = uri.starts_with("smb:") || self.kind == NetworkKind::Mount;
        if !is_network {
            return None;
        }
        let label = if self.label.is_empty() {
            fallback_label(&uri)
        } else {
            self.label.to_string()
        };
        Some(NetworkLocation {
            uri,
            label,
            saved: self.saved,
            connected: self.connected,
            kind: self.kind,
        })
    }
}

/// `Server` for `smb://host/`, `Share` for everything below it.
fn smb_location_kind(uri: &str) -> NetworkKind {
    match split_location(uri) {
        Ok(parts) if parts.path.is_empty() || parts.path == "/" => NetworkKind::Server,
        _ => NetworkKind::Share,
    }
}

/// The last decoded path segment of `uri`, or its host name for a server.
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

/// Network rows in the order they were first contributed, with an index
/// by [`NetworkKey`].
#[derive(Default)]
struct NetworkRows {
    rows: Vec<NetworkLocation>,
    indexes: HashMap<NetworkKey, usize>,
}

impl NetworkRows {
    /// Adds a new row, or merges `contribution` into the row with the same
    /// key. Invalid contributions are skipped, as Python's `add` swallows
    /// `ValueError`.
    fn add(&mut self, contribution: &Contribution<'_>) {
        let Some(location) = contribution.to_location() else {
            return;
        };
        let Ok(key) = network_key(&location.uri) else {
            return;
        };
        match self.indexes.entry(key) {
            Entry::Occupied(existing) => self.rows[*existing.get()].merge(location),
            Entry::Vacant(vacant) => {
                vacant.insert(self.rows.len());
                self.rows.push(location);
            }
        }
    }
}

impl NetworkLocation {
    /// Merges a later contribution into this row: a saved bookmark's URI
    /// and label win, the saved and connected flags add up, and the kind
    /// stays the first contributor's.
    fn merge(&mut self, later: NetworkLocation) {
        if later.saved {
            self.uri = later.uri;
            self.label = later.label;
        }
        self.saved |= later.saved;
        self.connected |= later.connected;
    }
}
