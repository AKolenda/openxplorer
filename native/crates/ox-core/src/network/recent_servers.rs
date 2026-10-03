// SPDX-License-Identifier: AGPL-3.0-only
//! The recent-servers list GTK's "Other Locations" shares (NET-019).
//!
//! GTK 4's places view, and so Files and the file chooser, keep the
//! servers connected through "Connect to Server" in a bookmark file,
//! `$XDG_DATA_HOME/gtk-4.0/servers`; GTK 3 kept it in
//! `$XDG_CONFIG_HOME/gtk-3.0/servers`. Map network location adds its
//! folders to the GTK 4 list, and the address bar suggests from both, so a
//! server typed once in any of these apps is offered in the others.
//!
//! Only server locations are written, and addresses never hold a password
//! ([`normalise`](crate::location::normalise) refuses one) or a user name
//! ([`without_user`] drops it, SAFE-010), so the file reveals no more than
//! the Network list does.

use std::path::{Path, PathBuf};

use crate::location::{is_server_location, normalise, without_user};

/// The application a bookmark names, as GTK names the app that added it.
const APPLICATION_NAME: &str = "OpenXplorer";
/// How that application opens a bookmark.
const APPLICATION_EXEC: &str = "openxplorer %u";

/// The most servers suggested.
const MAX_SUGGESTIONS: usize = 10;

/// The recent-servers files: the one written and those read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecentServers {
    /// GTK 4's list, written and read.
    list: PathBuf,
    /// GTK 3's list, only read.
    legacy_list: PathBuf,
}

impl RecentServers {
    /// The user's lists.
    pub fn for_user() -> Self {
        Self::in_folders(&glib::user_data_dir(), &glib::user_config_dir())
    }

    /// The lists under a data and a config folder (`XDG_DATA_HOME`,
    /// `XDG_CONFIG_HOME`).
    pub fn in_folders(data: &Path, config: &Path) -> Self {
        Self {
            list: data.join("gtk-4.0").join("servers"),
            legacy_list: config.join("gtk-3.0").join("servers"),
        }
    }

    /// Adds `uri`, without a user name, to GTK 4's list, or marks it
    /// visited now, titled `title`. Anything but a valid server location
    /// is ignored.
    ///
    /// # Errors
    ///
    /// The error of reading a damaged list or of writing it.
    pub fn add(&self, uri: &str, title: &str) -> Result<(), glib::Error> {
        let Ok(uri) = normalise(uri).map(|uri| without_user(&uri)) else {
            return Ok(());
        };
        if !is_server_location(&uri) {
            return Ok(());
        }
        let mut bookmarks = glib::BookmarkFile::new();
        if self.list.exists() {
            bookmarks.load_from_file(&self.list)?;
        }
        if !title.is_empty() {
            bookmarks.set_title(Some(&uri), title);
        }
        let now = glib::DateTime::now_utc().map_err(|error| failed(&error.to_string()))?;
        bookmarks.set_visited_date_time(&uri, &now);
        bookmarks.add_application(&uri, Some(APPLICATION_NAME), Some(APPLICATION_EXEC));
        if let Some(folder) = self.list.parent() {
            std::fs::create_dir_all(folder).map_err(|error| failed(&error.to_string()))?;
        }
        bookmarks.to_file(&self.list)
    }

    /// The recent server locations, most recently visited first, each
    /// once, at most ten. Unreadable lists and entries are skipped.
    pub fn suggestions(&self) -> Vec<String> {
        let mut visited: Vec<(i64, String)> = Vec::new();
        for list in [&self.list, &self.legacy_list] {
            visited.extend(read_list(list));
        }
        visited.sort_by_key(|(time, _)| std::cmp::Reverse(*time));
        let mut servers: Vec<String> = Vec::new();
        for (_, uri) in visited {
            if !servers.contains(&uri) {
                servers.push(uri);
            }
        }
        servers.truncate(MAX_SUGGESTIONS);
        servers
    }
}

/// A `GLib` file error saying `message`.
fn failed(message: &str) -> glib::Error {
    glib::Error::new(glib::FileError::Failed, message)
}

/// The valid server locations of the list at `path`, with when each was
/// last visited (0 when unknown).
fn read_list(path: &Path) -> Vec<(i64, String)> {
    let mut bookmarks = glib::BookmarkFile::new();
    if bookmarks.load_from_file(path).is_err() {
        return Vec::new();
    }
    let visited_at = |uri: &str| {
        let visited = bookmarks.visited_date_time(uri).ok();
        visited.map_or(0, |time| time.to_unix())
    };
    bookmarks
        .uris()
        .iter()
        .filter_map(|uri| {
            let canonical = normalise(uri).ok().filter(|uri| is_server_location(uri))?;
            Some((visited_at(uri), canonical))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// parity: NET-019
    #[test]
    fn mapped_servers_join_gtks_list_and_are_suggested_newest_first() {
        let home = tempfile::tempdir().expect("a temporary folder");
        let servers = RecentServers::in_folders(&home.path().join("data"), &home.path().join("config"));
        let legacy = home.path().join("config/gtk-3.0");
        std::fs::create_dir_all(&legacy).expect("the GTK 3 folder");
        let mut old = glib::BookmarkFile::new();
        old.add_application("ftp://mirror.example/", Some("Files"), Some("nautilus %u"));
        let long_ago = glib::DateTime::from_unix_utc(1).expect("a time");
        old.set_visited_date_time("ftp://mirror.example/", &long_ago);
        old.to_file(legacy.join("servers")).expect("the GTK 3 list");

        servers
            .add("sftp://Build/home/anna", "anna on build")
            .expect("added");
        servers.add("file:///home/anna", "").expect("ignored");
        servers.add("smb://nas/Projects", "").expect("added");

        let mut written = glib::BookmarkFile::new();
        written
            .load_from_file(home.path().join("data/gtk-4.0/servers"))
            .expect("GTK 4's list");
        assert_eq!(
            written
                .title(Some("sftp://build/home/anna"))
                .expect("titled")
                .as_str(),
            "anna on build"
        );
        let suggested = servers.suggestions();
        assert_eq!(suggested.len(), 3, "{suggested:?}");
        assert_eq!(
            suggested.last().map(String::as_str),
            Some("ftp://mirror.example/")
        );
        assert!(suggested.contains(&"smb://nas/Projects".to_owned()));
    }
}
