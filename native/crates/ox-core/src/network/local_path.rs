// SPDX-License-Identifier: AGPL-3.0-only
//! A local path for an SMB location, for applications, terminals and drags
//! that need one.
//!
//! Ports `local_path` in `desktop/native_opening.py`: a kernel CIFS mount
//! of the share is preferred ([`resolve_smb_path`]), then the user's `GVfs`
//! FUSE export under `$XDG_RUNTIME_DIR/gvfs`. Nothing is mounted.

use std::fs;
use std::path::{Path, PathBuf};

use gio::prelude::*;

use super::mount_table::{read_mount_table, resolve_smb_path};
use super::server::{ServerKey, DEFAULT_SMB_PORT};
use crate::location::{is_smb_location, normalise, split_location, unquote_lossy};

/// Prefix of `GVfs`'s FUSE directory names for SMB shares, for example
/// `smb-share:server=nas,share=projects`.
const FUSE_SHARE_PREFIX: &str = "smb-share:";

/// The local path of `uri`: its own path for a `file:` location; for an
/// SMB location, the path inside a CIFS mount or the `GVfs` FUSE export.
/// `None` when there is none, or for other locations.
pub fn local_path(uri: &str) -> Option<PathBuf> {
    let file = gio::File::for_uri(&normalise(uri).ok()?);
    if let Some(path) = file.path() {
        return Some(path);
    }
    if !is_smb_location(uri) {
        return None;
    }
    // An unreadable mount table only rules out kernel mounts.
    let mounts = read_mount_table().unwrap_or_default();
    if let Ok(Some(mounted)) = resolve_smb_path(uri, &mounts) {
        return Some(mounted);
    }
    let fuse_root = glib::user_runtime_dir().join("gvfs");
    fuse_export_path(uri, &fuse_root)
}

/// The path of SMB location `uri` inside GNOME's optional FUSE export at
/// `fuse_root`, which only the logged-in user can reach. Server and share
/// compare case-insensitively and the port must match.
pub fn fuse_export_path(uri: &str, fuse_root: &Path) -> Option<PathBuf> {
    let server = ServerKey::for_location(uri)?;
    let parts = split_location(uri).ok()?;
    let decoded = unquote_lossy(&parts.path);
    let components: Vec<&str> = decoded.trim_matches('/').split('/').collect();
    let (share, folders) = components.split_first()?;
    if share.is_empty() {
        return None;
    }
    let mut names: Vec<String> = fs::read_dir(fuse_root)
        .ok()?
        .filter_map(|entry| entry.ok()?.file_name().into_string().ok())
        .collect();
    // Sorted so the same export wins every time.
    names.sort();
    let export = names
        .into_iter()
        .find(|name| exports_share(name, &server, share))?;
    let mut path = fuse_root.join(export);
    path.extend(folders);
    Some(path)
}

/// True when the FUSE directory `name` exports `share` of `server`.
fn exports_share(name: &str, server: &ServerKey, share: &str) -> bool {
    let Some(fields) = name.strip_prefix(FUSE_SHARE_PREFIX) else {
        return false;
    };
    let export_server = unquote_lossy(fuse_field(fields, "server").unwrap_or_default());
    let export_share = unquote_lossy(fuse_field(fields, "share").unwrap_or_default());
    let default_port = DEFAULT_SMB_PORT.to_string();
    let export_port = fuse_field(fields, "port").unwrap_or(&default_port);
    export_server.to_lowercase() == server.host()
        && glib::casefold(&export_share) == glib::casefold(share)
        && export_port == server.port().to_string()
}

/// The value of `key` among the `key=value,...` fields of a FUSE directory
/// name; the last one wins, as in the Python app's `dict`.
fn fuse_field<'a>(fields: &'a str, key: &str) -> Option<&'a str> {
    let pairs = fields.split(',').filter_map(|pair| pair.split_once('='));
    let mut values = pairs.filter(|(name, _)| *name == key).map(|(_, value)| value);
    values.next_back()
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    /// A FUSE root holding the given export directories.
    fn fuse_root(exports: &[&str]) -> tempfile::TempDir {
        let root = tempfile::tempdir().expect("temporary folder");
        for export in exports {
            fs::create_dir(root.path().join(export)).expect("export folder");
        }
        root
    }

    /// parity: NET-026, OPEN-006
    #[test]
    fn a_share_resolves_inside_its_fuse_export() {
        let root = fuse_root(&["smb-share:server=studio-nas,share=projects,user=sam"]);

        let path = fuse_export_path("smb://studio-nas/Projects/Film/cut.mp4", root.path());

        let expected = root
            .path()
            .join("smb-share:server=studio-nas,share=projects,user=sam/Film/cut.mp4");
        assert_eq!(path, Some(expected));
    }

    /// parity: NET-026
    #[test]
    fn exports_of_other_servers_shares_and_ports_are_ignored() {
        let root = fuse_root(&[
            "smb-share:server=other,share=projects",
            "smb-share:server=studio-nas,share=archive",
            "smb-share:port=1445,server=studio-nas,share=projects",
            "sftp:host=studio-nas",
        ]);

        assert_eq!(fuse_export_path("smb://studio-nas/Projects", root.path()), None);
        let custom_port = fuse_export_path("smb://studio-nas:1445/Projects", root.path());
        let expected = root
            .path()
            .join("smb-share:port=1445,server=studio-nas,share=projects");
        assert_eq!(custom_port, Some(expected));
    }

    /// parity: NET-026
    #[test]
    fn a_server_without_a_share_has_no_export() {
        let root = fuse_root(&["smb-share:server=studio-nas,share=projects"]);
        assert_eq!(fuse_export_path("smb://studio-nas/", root.path()), None);
    }

    #[test]
    fn local_files_are_their_own_path() {
        assert_eq!(local_path("file:///tmp/a%20b"), Some(PathBuf::from("/tmp/a b")));
        assert_eq!(local_path("https://example.invalid/"), None);
    }
}
