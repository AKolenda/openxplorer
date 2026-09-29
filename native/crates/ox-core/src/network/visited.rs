// SPDX-License-Identifier: AGPL-3.0-only
//! Servers and shares browsed this session, for the Network list.
//!
//! Ports `remember_network` and `visited_network` in `desktop/winspace.py`.
//! Browsing adds the share root (or the server) to the session's Network
//! list, never every subfolder, and never saves anything: only the user
//! keeps a share in settings. Sign out removes the server's entries.

use super::server::host_name;
use crate::location::{is_remote_location, is_smb_location, split_location};
use crate::settings::Bookmark;

/// The SMB roots and other servers browsed this session, in the order first browsed. One
/// list serves every window.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct VisitedNetwork {
    roots: Vec<String>,
}

impl VisitedNetwork {
    /// Remembers the share root of `uri`, a location that was listed.
    /// Returns true when it is new, so the caller refreshes the sidebar.
    pub fn remember(&mut self, uri: &str) -> bool {
        let Some(root) = session_network_root(uri) else {
            return false;
        };
        if self.roots.iter().any(|known| same_root(known, &root)) {
            return false;
        }
        self.roots.push(root);
        true
    }

    /// Forgets every root on `host`, when signing out of it.
    pub fn forget_host(&mut self, host: &str) {
        self.roots.retain(|root| host_name(root).as_deref() != Some(host));
    }

    /// The remembered roots, oldest first.
    pub fn iter(&self) -> impl Iterator<Item = &str> {
        self.roots.iter().map(String::as_str)
    }

    /// The remembered roots as label-less bookmarks, the visited input of
    /// [`merge_network_locations`](crate::places::merge_network_locations).
    pub fn to_bookmarks(&self) -> Vec<Bookmark> {
        let bookmark = |root: &str| Bookmark {
            uri: root.to_owned(),
            label: String::new(),
        };
        self.iter().map(bookmark).collect()
    }
}

/// The root the Network list shows for SMB location `uri`:
/// `smb://host/share`, or `smb://host/` for a server listing. On SFTP,
/// FTP, WebDAV and NFS servers, whose folders are not shares, it is the
/// folder itself; [`VisitedNetwork::remember`] keeps the first one browsed
/// on each server. `None` for other locations.
pub fn session_network_root(uri: &str) -> Option<String> {
    if is_remote_location(uri) {
        return Some(uri.to_owned());
    }
    if !is_smb_location(uri) {
        return None;
    }
    let parts = split_location(uri).ok()?;
    let share = parts.path.trim_matches('/').split('/').next().unwrap_or_default();
    Some(format!("smb://{}/{share}", parts.authority))
}

/// True when `known` already stands for `root`: the same SMB root, or any
/// folder of the same remote server (scheme, user, host and port), so an
/// SFTP server is listed once, at the first folder opened on it.
fn same_root(known: &str, root: &str) -> bool {
    if !is_remote_location(root) {
        return known == root;
    }
    let server = |uri: &str| split_location(uri).ok().map(|parts| (parts.scheme, parts.authority));
    server(known).is_some_and(|known| Some(known) == server(root))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// parity: NET-030
    #[test]
    fn a_remote_server_is_remembered_once_at_the_first_folder_opened() {
        let mut visited = VisitedNetwork::default();

        assert!(visited.remember("sftp://anna@build/home/anna"));
        assert!(!visited.remember("sftp://anna@build/srv"));
        assert!(visited.remember("sftp://build/"), "another account is another row");
        assert!(visited.remember("davs://cloud.example/remote.php/dav"));

        let roots: Vec<&str> = visited.iter().collect();
        let expected = [
            "sftp://anna@build/home/anna",
            "sftp://build/",
            "davs://cloud.example/remote.php/dav",
        ];
        assert_eq!(roots, expected);
        visited.forget_host("build");
        assert_eq!(visited.iter().count(), 1);
    }

    /// Ported from `desktop/tests/test_v07.py::AppManagerTests::test_session_network_root_not_every_child`
    ///
    /// parity: NET-016, NET-018
    #[test]
    fn browsing_remembers_the_share_root_not_every_subfolder() {
        let mut visited = VisitedNetwork::default();

        assert!(visited.remember("smb://nas/work/nested/a"));
        assert!(!visited.remember("smb://nas/work/b"));

        assert_eq!(visited.iter().collect::<Vec<_>>(), ["smb://nas/work"]);
    }

    /// parity: NET-016
    #[test]
    fn a_server_listing_is_remembered_as_the_server() {
        assert_eq!(session_network_root("smb://nas/").as_deref(), Some("smb://nas/"));
        assert_eq!(session_network_root("smb://nas").as_deref(), Some("smb://nas/"));
        assert_eq!(session_network_root("file:///home/demo"), None);
    }

    /// parity: NET-018
    #[test]
    fn signing_out_forgets_only_that_servers_roots() {
        let mut visited = VisitedNetwork::default();
        visited.remember("smb://nas/work");
        visited.remember("smb://nas/");
        visited.remember("smb://other/share");

        visited.forget_host("nas");

        let expected = [Bookmark {
            uri: "smb://other/share".into(),
            label: String::new(),
        }];
        assert_eq!(visited.to_bookmarks(), expected);
    }
}
