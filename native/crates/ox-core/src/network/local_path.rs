// SPDX-License-Identifier: AGPL-3.0-only
//! A local path for an SMB location, for applications, terminals and drags
//! that need one.
//!
//! Ports `local_path` in `v2.0.0:desktop/native_opening.py`: a kernel CIFS mount
//! of the share is preferred ([`resolve_smb_path`]), then the user's `GVfs`
//! FUSE export under `$XDG_RUNTIME_DIR/gvfs`. Nothing is mounted. The
//! reverse, [`fuse_export_location`], gives the remote location of a path
//! in that export, for paths other applications hand over.

use std::fs;
use std::os::unix::fs::MetadataExt;
use std::path::{Component, Path, PathBuf};

use gio::prelude::*;

use super::mount_table::{read_mount_table, resolve_smb_path};
use super::server::{ServerKey, DEFAULT_SMB_PORT};
use crate::location::{is_smb_location, normalise, quote_component, split_location, unquote_lossy};

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

/// The remote location of `path` when it lies inside the current user's
/// `GVfs` FUSE export at `fuse_root`: the reverse of [`fuse_export_path`],
/// reading the export's name with the same field parser. A path in
/// `smb-share:server=nas,share=projects,user=sam/Film` is
/// `smb://nas/projects/Film`: the export's user and domain are left out,
/// as the share's normal address leaves them out, and SMB's own port too.
/// SFTP, FTP, WebDAV, MTP and gPhoto2 exports map the same way.
///
/// `None`, so the caller keeps the local path, when `path` is not below
/// `fuse_root` (a look-alike folder elsewhere, or one that climbs out with
/// `..`), when `fuse_root` belongs to another user, and for an export
/// whose name does not say where it leads. Only `fuse_root` itself is
/// looked at on disk; nothing is mounted.
pub fn fuse_export_location(path: &Path, fuse_root: &Path) -> Option<String> {
    let relative = path.strip_prefix(fuse_root).ok()?;
    let mut parts = Vec::new();
    for component in relative.components() {
        let Component::Normal(part) = component else {
            return None;
        };
        parts.push(part.to_str()?);
    }
    let (export, inside) = parts.split_first()?;
    if !belongs_to_current_user(fuse_root) {
        return None;
    }
    let (kind, fields) = export.split_once(':')?;
    let field = |key: &str| {
        fuse_field(fields, key)
            .map(unquote_lossy)
            .filter(|value| !value.is_empty())
    };
    let port = field("port");
    let (scheme, host, port, first) = match kind {
        "smb-share" => {
            let port = port.filter(|port| *port != DEFAULT_SMB_PORT.to_string());
            ("smb", field("server")?, port, Some(field("share")?))
        }
        "sftp" | "ftp" | "ftps" => (kind, field("host")?, port, None),
        "dav" => {
            let scheme = if field("ssl").as_deref() == Some("true") {
                "davs"
            } else {
                "dav"
            };
            (scheme, field("host")?, port, field("prefix"))
        }
        "mtp" | "gphoto2" => (kind, field("host")?, None, None),
        _ => return None,
    };
    // An IPv6 address goes in brackets; a device's bracketed bus address,
    // such as `[usb:001,004]`, keeps the form the app writes for devices.
    let bracketed = host.starts_with('[') && host.ends_with(']');
    let host = if host.parse::<std::net::Ipv6Addr>().is_ok() {
        format!("[{host}]")
    } else if bracketed && kind != "smb-share" {
        host
    } else {
        quote_component(&host)
    };
    let mut location = format!("{scheme}://{host}");
    if let Some(port) = port {
        location.push(':');
        location.push_str(&quote_component(&port));
    }
    // A WebDAV prefix is a path of its own, such as `/remote.php/webdav`.
    let leading = first.iter().flat_map(|first| first.split('/'));
    for part in leading
        .chain(inside.iter().copied())
        .filter(|part| !part.is_empty())
    {
        location.push('/');
        location.push_str(&quote_component(part));
    }
    if location.matches('/').count() == 2 {
        location.push('/');
    }
    normalise(&location).ok().filter(|uri| !uri.starts_with("file:"))
}

/// `uri`, a location another application handed over, with a path inside
/// the current user's `GVfs` FUSE export (`$XDG_RUNTIME_DIR/gvfs`) turned
/// into the remote location it exports ([`fuse_export_location`]), so the
/// share opens with its own address. Any other location, and an export
/// whose name cannot be reversed, comes back unchanged.
pub fn outside_location(uri: String) -> String {
    let Some(path) = uri
        .starts_with("file://")
        .then(|| gio::File::for_uri(&uri).path())
        .flatten()
    else {
        return uri;
    };
    let fuse_root = glib::user_runtime_dir().join("gvfs");
    fuse_export_location(&path, &fuse_root).unwrap_or(uri)
}

/// True when `folder` is a real folder, not a link, that the current user
/// owns, as the FUSE root in their private runtime folder is.
fn belongs_to_current_user(folder: &Path) -> bool {
    fs::symlink_metadata(folder)
        .is_ok_and(|metadata| metadata.is_dir() && metadata.uid() == rustix::process::getuid().as_raw())
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

    /// The current user's FUSE root in a temporary folder, holding no
    /// export: the reverse reads only the path, never the share.
    fn empty_root() -> tempfile::TempDir {
        fuse_root(&[])
    }

    /// The remote location of `relative` inside `root`.
    fn location_of(root: &tempfile::TempDir, relative: &str) -> Option<String> {
        fuse_export_location(&root.path().join(relative), root.path())
    }

    /// parity: INT-014, NET-026
    #[test]
    fn a_path_in_a_share_export_is_the_share_location() {
        let root = empty_root();

        assert_eq!(
            location_of(&root, "smb-share:server=studio-nas,share=projects"),
            Some("smb://studio-nas/projects".to_owned())
        );
        assert_eq!(
            location_of(
                &root,
                "smb-share:server=studio-nas,share=projects/Film/2026/cut.mp4"
            ),
            Some("smb://studio-nas/projects/Film/2026/cut.mp4".to_owned())
        );
    }

    /// The export's domain and user are left out, as the share's normal
    /// address does; a port other than SMB's own is kept.
    ///
    /// parity: INT-014, NET-026, SAFE-010
    #[test]
    fn the_user_and_domain_are_left_out_and_a_custom_port_kept() {
        let root = empty_root();

        let with_user = "smb-share:domain=STUDIO,server=studio-nas,share=projects,user=sam/Film";
        assert_eq!(
            location_of(&root, with_user),
            Some("smb://studio-nas/projects/Film".to_owned())
        );
        let custom_port = "smb-share:port=1445,server=studio-nas,share=projects/Film";
        assert_eq!(
            location_of(&root, custom_port),
            Some("smb://studio-nas:1445/projects/Film".to_owned())
        );
        let default_port = "smb-share:port=445,server=studio-nas,share=projects";
        assert_eq!(
            location_of(&root, default_port),
            Some("smb://studio-nas/projects".to_owned())
        );
    }

    /// Escapes in the export's name are decoded, and names with spaces,
    /// `#`, `%` or non-ASCII letters come out escaped once.
    ///
    /// parity: INT-014, NET-026
    #[test]
    fn encoded_characters_are_decoded_and_escaped_once() {
        let root = empty_root();

        let location = location_of(
            &root,
            "smb-share:server=studio-nas,share=team%20files/Résumé #2/100% done.txt",
        );

        assert_eq!(
            location,
            Some("smb://studio-nas/team%20files/R%C3%A9sum%C3%A9%20%232/100%25%20done.txt".to_owned())
        );
    }

    /// A share location and its FUSE path lead to each other.
    ///
    /// parity: INT-014, NET-026
    #[test]
    fn a_share_location_and_its_fuse_path_lead_to_each_other() {
        let root = fuse_root(&[
            "smb-share:server=studio-nas,share=team%20files,user=sam",
            "smb-share:port=1445,server=archive-nas,share=old",
        ]);
        for remote in [
            "smb://studio-nas/team%20files/Film/R%C3%A9sum%C3%A9%20%232.txt",
            "smb://archive-nas:1445/old/2019",
        ] {
            let path = fuse_export_path(remote, root.path()).expect("the share is exported");

            assert_eq!(fuse_export_location(&path, root.path()).as_deref(), Some(remote));
        }
    }

    /// Other `GVfs` backends whose export name says where they lead go
    /// through the same mapping; any other export keeps its local path.
    ///
    /// parity: INT-014, NET-026
    #[test]
    fn other_backends_map_where_their_name_says_where_they_lead() {
        let root = empty_root();

        let cases = [
            (
                "sftp:host=build-box,port=2222,user=sam/srv/logs",
                Some("sftp://build-box:2222/srv/logs"),
            ),
            (
                "ftp:host=files.example.test/pub",
                Some("ftp://files.example.test/pub"),
            ),
            (
                "dav:host=cloud.example.test,prefix=%2Fremote.php%2Fwebdav,ssl=true,user=sam/Notes",
                Some("davs://cloud.example.test/remote.php/webdav/Notes"),
            ),
            (
                "dav:host=cloud.example.test/Notes",
                Some("dav://cloud.example.test/Notes"),
            ),
            (
                "mtp:host=Fictional_Phone_0123/Internal storage/DCIM",
                Some("mtp://Fictional_Phone_0123/Internal%20storage/DCIM"),
            ),
            (
                "gphoto2:host=%5Busb%3A001%2C004%5D/DCIM",
                Some("gphoto2://[usb:001,004]/DCIM"),
            ),
            ("afp-volume:host=old-mac,volume=Shared/Docs", None),
            ("nfs:host=nfs-box,prefix=%2Fexport/a", None),
            ("smb-share:share=projects/Film", None),
            ("no-colon-here/Film", None),
        ];
        for (relative, expected) in cases {
            assert_eq!(location_of(&root, relative).as_deref(), expected, "{relative}");
        }
    }

    /// Only a path inside the given FUSE root counts: a look-alike folder
    /// elsewhere, the root itself or a path that climbs out of it does not.
    ///
    /// parity: INT-014, NET-026
    #[test]
    fn look_alike_paths_are_not_exports() {
        let root = empty_root();
        let elsewhere = tempfile::tempdir().expect("temporary folder");
        let look_alike = elsewhere
            .path()
            .join("gvfs/smb-share:server=studio-nas,share=projects/Film");

        assert_eq!(fuse_export_location(&look_alike, root.path()), None);
        assert_eq!(fuse_export_location(root.path(), root.path()), None);
        let climbing = root
            .path()
            .join("smb-share:server=studio-nas,share=projects/../../etc");
        assert_eq!(fuse_export_location(&climbing, root.path()), None);
    }

    /// A FUSE root another user owns is not the current user's export.
    ///
    /// parity: INT-014, NET-026
    #[test]
    fn a_root_another_user_owns_is_not_the_users_export() {
        if rustix::process::geteuid().is_root() {
            return;
        }
        // `/` belongs to root.
        let path = Path::new("/smb-share:server=studio-nas,share=projects/Film");

        assert_eq!(fuse_export_location(path, Path::new("/")), None);
    }

    #[test]
    fn local_files_are_their_own_path() {
        assert_eq!(local_path("file:///tmp/a%20b"), Some(PathBuf::from("/tmp/a b")));
        assert_eq!(local_path("https://example.invalid/"), None);
    }
}
