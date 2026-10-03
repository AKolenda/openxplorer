// SPDX-License-Identifier: AGPL-3.0-only
//! What kind of folder a location is, for the window: a read-only
//! snapshot, a folder where New and Paste may create items, a network
//! folder that gets the network icon, or a whole SMB server or share.
//!
//! Ports `readonlyLocation`, `writableLocation`, `networkLocation` and
//! `isSmbShareRoot` from `v2.0.0:desktop/ui/app.js`, extended with the virtual
//! places in [`VirtualPlace`]. The session facts they read live in
//! [`LocationContext`].

use super::display::{location_parts, same_location, LocationContext};
use super::location_kind;
use super::normalise::is_smb_server;
use super::parts::LocationKind;
use super::text::{decode_uri_component, strip_one_trailing_slash};
use super::virtual_place::{is_in_virtual_folder, VirtualPlace};

/// Path components that mark a read-only snapshot (Btrfs/NAS snapshots
/// and Windows "Previous versions" over SMB).
const SNAPSHOT_DIRECTORIES: [&str; 3] = [".snapshot", ".snapshots", "#snapshot"];

/// Start of the Windows "Previous versions" folder names that SMB servers
/// show, for example `@GMT-2024.01.01-00.00.00`.
const PREVIOUS_VERSION_PREFIX: &str = "@GMT-";

impl LocationContext {
    /// True inside a read-only snapshot: a `.snapshot`, `.snapshots`,
    /// `#snapshot`, `@GMT-…` or `.zfs/snapshot` component, or a configured
    /// snapshot root. The web UI's `readonlyLocation`.
    pub fn is_snapshot_location(&self, uri: &str) -> bool {
        if let Some(path) = uri.strip_prefix("admin:") {
            return self.is_snapshot_location(&format!("file:{path}"));
        }
        if uri.is_empty() {
            return false;
        }
        has_snapshot_component(uri) || self.is_in_snapshot_root(uri)
    }

    /// True where New and Paste may create items: a real folder that is not
    /// an SMB server listing, a virtual place, a snapshot or a ZIP. The web UI's
    /// `writableLocation`.
    pub fn is_writable_location(&self, uri: &str) -> bool {
        // Safety rule (app.js `writableLocation`): New and Paste never
        // create items on a page, in a virtual folder, in a server's list
        // of shares or in a read-only snapshot. A ZIP browsed as a folder
        // is read-only too.
        !uri.is_empty()
            && !super::is_archive_location(uri)
            && VirtualPlace::from_uri(uri).is_none()
            && !is_in_virtual_folder(uri)
            && !is_smb_server(uri)
            && !self.is_snapshot_location(uri)
    }

    /// True for SMB locations and for local folders inside a mounted
    /// CIFS/SMB3 share; such folders get the network icon. The web UI's
    /// `networkLocation`.
    pub fn is_network_location(&self, uri: &str) -> bool {
        match location_kind(uri) {
            LocationKind::Smb | LocationKind::Remote => true,
            LocationKind::Local => self.is_on_network_mount(uri),
            LocationKind::Device | LocationKind::Other => false,
        }
    }

    /// True for a `file:` location at or below a mounted CIFS/SMB3 share.
    fn is_on_network_mount(&self, uri: &str) -> bool {
        let Some(decoded) = decoded_path(uri) else {
            return false;
        };
        let path = match strip_one_trailing_slash(&decoded) {
            "" => "/",
            path => path,
        };
        self.network_mounts
            .iter()
            .any(|mount| is_path_at_or_below(path, &mount.to_string_lossy()))
    }

    /// True for a configured snapshot root and everything below it.
    fn is_in_snapshot_root(&self, uri: &str) -> bool {
        self.snapshot_roots
            .iter()
            .any(|root| is_uri_at_or_below(uri, root))
    }
}

/// True for the mount types whose folders count as network folders:
/// `cifs` and `smb3`. An empty type counts as `cifs`, as in the web UI's
/// `networkLocation`.
pub fn is_network_filesystem(filesystem_type: &str) -> bool {
    matches!(filesystem_type, "" | "cifs" | "smb3")
}

/// True for a whole SMB server or share (`smb://nas/` or `smb://nas/share`),
/// which cannot be renamed, moved or trashed.
pub fn is_smb_share_root(uri: &str) -> bool {
    location_parts(uri).is_some_and(|parts| parts.is_smb() && parts.path_depth() <= 1)
}

/// The decoded path of a `scheme://` location, as the web UI's
/// `decodeURIComponent(locationParts(uri).pathname)`; `None` when `uri`
/// does not split or its path does not decode.
fn decoded_path(uri: &str) -> Option<String> {
    let parts = location_parts(uri)?;
    decode_uri_component(&parts.path)
}

/// True when a decoded path component of `uri` marks a snapshot folder,
/// or `uri` is inside `.zfs/snapshot`. A path that does not decode has no
/// components, as in the web UI.
fn has_snapshot_component(uri: &str) -> bool {
    let decoded = decoded_path(uri).unwrap_or_default();
    let components: Vec<&str> = decoded.split('/').collect();
    let has_marked_directory = components.iter().copied().any(is_snapshot_directory);
    let is_in_zfs_snapshot = components.windows(2).any(|pair| pair == [".zfs", "snapshot"]);
    has_marked_directory || is_in_zfs_snapshot
}

/// `.snapshot`, `.snapshots`, `#snapshot` or a Windows `@GMT-…` previous
/// version.
fn is_snapshot_directory(name: &str) -> bool {
    SNAPSHOT_DIRECTORIES.contains(&name) || name.starts_with(PREVIOUS_VERSION_PREFIX)
}

/// `uri` is `root` (ignoring one trailing slash) or lies below it, as the
/// web UI's `readonlyLocation` compares snapshot roots.
fn is_uri_at_or_below(uri: &str, root: &str) -> bool {
    let prefix = format!("{}/", strip_one_trailing_slash(root));
    same_location(uri, root) || uri.starts_with(&prefix)
}

/// `path` equals the mount point `root` or lies below it, as the web UI's
/// `networkLocation` compares stable mounts. An empty mount point matches
/// nothing.
fn is_path_at_or_below(path: &str, root: &str) -> bool {
    if root.is_empty() {
        return false;
    }
    let prefix = format!("{}/", strip_one_trailing_slash(root));
    path == root || path.starts_with(&prefix)
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::location::{HOME_URI, NETWORK_URI, PC_URI, RECENT_URI, SETTINGS_URI, TRASH_URI};

    /// Virtual places and folders are never writable, not even a malformed
    /// item address inside one.
    #[test]
    fn virtual_places_are_not_writable() {
        let context = LocationContext {
            home: Some(PathBuf::from("/home/test")),
            ..LocationContext::default()
        };
        let virtual_locations = [
            HOME_URI,
            PC_URI,
            NETWORK_URI,
            SETTINGS_URI,
            TRASH_URI,
            RECENT_URI,
            "trash:///a",
            "recent:///%FF",
        ];
        for uri in virtual_locations {
            assert!(!context.is_writable_location(uri), "{uri}");
        }
        assert!(context.is_writable_location("file:///home/test"));
    }

    #[test]
    fn only_smb_addresses_are_smb_locations() {
        assert!(crate::location::is_smb_location("smb://nas/media"));
        assert!(!crate::location::is_smb_location("file:///srv/smb"));
        assert!(!crate::location::is_smb_location("mtp://phone/"));
        assert!(!crate::location::is_smb_location("smb-share:server=nas"));
    }
}
