// SPDX-License-Identifier: AGPL-3.0-only
//! The rows of the Network section: saved shares, connected mounts and
//! servers browsed in this session, merged without duplicates.
//!
//! Ports `desktop/network_locations.py`. No network I/O happens here, and
//! nothing is saved: a browsed share appears only for this session unless
//! the user keeps it.

use std::borrow::Cow;
use std::collections::HashMap;
use std::path::PathBuf;

use crate::location::{file_uri, normalise, split_location, unquote_lossy, LocationError};
use crate::settings::Bookmark;

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

/// A mapped network share from the settings, with its connection state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SavedShare {
    /// The saved location and label.
    pub bookmark: Bookmark,
    /// Whether a current mount contains it, computed by the caller from its
    /// mount snapshot.
    pub connected: bool,
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

/// SMB's well-known port, which Python's `network_key` assumes when a
/// location gives none.
const DEFAULT_SMB_PORT: u16 = 445;

/// Canonical display identity matching Python's `network_key`.
///
/// # Errors
///
/// [`LocationError`] if `uri` is not a valid location.
pub fn network_key(uri: &str) -> Result<NetworkKey, LocationError> {
    let uri = normalise(uri)?;
    let parts = split_location(&uri)?;
    if parts.scheme != "smb" {
        return Ok(NetworkKey::Other(uri));
    }
    let explicit_port = parts.port()?.filter(|port| *port != 0);
    let decoded_path = unquote_lossy(&parts.path);
    Ok(NetworkKey::Smb {
        host: parts.hostname().unwrap_or_default(),
        port: explicit_port.unwrap_or(DEFAULT_SMB_PORT),
        path: glib::casefold(decoded_path.trim_end_matches('/')).to_string(),
    })
}

/// Merges saved shares, active GIO mounts, kernel mounts and visited SMB
/// locations in that order. Invalid inputs are ignored. Saved labels win,
/// connection state is combined, and host aliases remain distinct.
pub fn merge_network_locations(
    saved: &[SavedShare],
    mounts: &[NetworkMount],
    stable: &[StableMount],
    visited: &[Bookmark],
) -> Vec<NetworkLocation> {
    let active_smb_mounts = mounts
        .iter()
        .filter(|mount| mount.mounted && mount.uri.starts_with("smb:"));
    let network_kernel_mounts = stable.iter().filter(|mount| is_network_mount(mount));
    let mut merged = NetworkRows::default();
    merged.add_all(saved.iter().map(Contribution::saved_share));
    merged.add_all(active_smb_mounts.map(Contribution::gio_mount));
    merged.add_all(network_kernel_mounts.map(Contribution::kernel_mount));
    merged.add_all(visited.iter().map(Contribution::visited));
    merged.rows
}

/// One source's view of a network location (the arguments of `add` in
/// `network_locations.py`).
struct Contribution<'a> {
    uri: Cow<'a, str>,
    label: Cow<'a, str>,
    saved: bool,
    connected: bool,
    kind: NetworkKind,
}

impl<'a> Contribution<'a> {
    /// A share saved in the settings; its URI and label win.
    fn saved_share(share: &'a SavedShare) -> Self {
        Self {
            uri: Cow::Borrowed(&share.bookmark.uri),
            label: Cow::Borrowed(&share.bookmark.label),
            saved: true,
            connected: share.connected,
            kind: NetworkKind::Share,
        }
    }

    /// An active SMB mount reported by GIO.
    fn gio_mount(mount: &'a NetworkMount) -> Self {
        Self {
            uri: Cow::Borrowed(&mount.uri),
            label: Cow::Borrowed(&mount.label),
            saved: false,
            connected: true,
            kind: remote_kind(&mount.uri),
        }
    }

    /// A CIFS or SMB3 kernel mount point, as a `file://` location.
    fn kernel_mount(mount: &'a StableMount) -> Self {
        Self {
            uri: Cow::Owned(file_uri(&mount.path)),
            label: mount_label(mount),
            saved: false,
            connected: true,
            kind: NetworkKind::Mount,
        }
    }

    /// An SMB server or share browsed in this session.
    fn visited(bookmark: &'a Bookmark) -> Self {
        Self {
            uri: Cow::Borrowed(&bookmark.uri),
            label: Cow::Borrowed(&bookmark.label),
            saved: false,
            connected: false,
            kind: remote_kind(&bookmark.uri),
        }
    }
}

/// The merged rows so far, with the index of each row's key.
#[derive(Default)]
struct NetworkRows {
    rows: Vec<NetworkLocation>,
    indexes: HashMap<NetworkKey, usize>,
}

impl NetworkRows {
    /// Adds every contribution of one source in order, each as
    /// [`add`](Self::add) does.
    fn add_all<'a>(&mut self, contributions: impl Iterator<Item = Contribution<'a>>) {
        for contribution in contributions {
            self.add(contribution);
        }
    }

    /// Adds a row, or merges into the row with the same [`NetworkKey`]:
    /// a saved contribution takes over the URI and label, and the saved and
    /// connected flags accumulate.
    fn add(&mut self, contribution: Contribution<'_>) {
        let Ok(uri) = normalise(&contribution.uri) else {
            return;
        };
        if !uri.starts_with("smb:") && contribution.kind != NetworkKind::Mount {
            return;
        }
        let Ok(key) = network_key(&uri) else {
            return;
        };
        let label = if contribution.label.is_empty() {
            fallback_label(&uri)
        } else {
            contribution.label.into_owned()
        };
        let Some(&index) = self.indexes.get(&key) else {
            self.indexes.insert(key, self.rows.len());
            self.rows.push(NetworkLocation {
                uri,
                label,
                saved: contribution.saved,
                connected: contribution.connected,
                kind: contribution.kind,
            });
            return;
        };
        let row = &mut self.rows[index];
        if contribution.saved {
            row.uri = uri;
            row.label = label;
        }
        row.saved |= contribution.saved;
        row.connected |= contribution.connected;
    }
}

/// A server for a URI without a path, otherwise a share.
fn remote_kind(uri: &str) -> NetworkKind {
    match split_location(uri) {
        Ok(parts) if parts.path.is_empty() || parts.path == "/" => NetworkKind::Server,
        _ => NetworkKind::Share,
    }
}

/// A CIFS or SMB3 mount at an absolute path.
fn is_network_mount(mount: &StableMount) -> bool {
    matches!(mount.filesystem.as_str(), "cifs" | "smb3") && mount.path.is_absolute()
}

/// The mount's own label, or the name of its mount point.
fn mount_label(mount: &StableMount) -> Cow<'_, str> {
    if mount.label.is_empty() {
        mount.path.file_name().unwrap_or_default().to_string_lossy()
    } else {
        Cow::Borrowed(&mount.label)
    }
}

/// The label of a row without one: the last path name, else the host
/// (`network_locations.py:28`). Pins fall back differently, see
/// `pin_fallback_label` in the settings module.
fn fallback_label(uri: &str) -> String {
    let Ok(parts) = split_location(uri) else {
        return String::new();
    };
    let name = parts.last_name();
    if name.is_empty() {
        parts.hostname().unwrap_or_default()
    } else {
        name
    }
}
