// SPDX-License-Identifier: AGPL-3.0-only
//! The desktop-wide places list (SIDE-013).
//!
//! GTK's file dialogs, GNOME Files and other GTK file managers list the
//! bookmarks in `$XDG_CONFIG_HOME/gtk-3.0/bookmarks`, one `uri label`
//! per line. Quick access pins are kept in the app's own settings,
//! where the Python app reads them too; this module mirrors them into that
//! file, so a folder pinned here also appears in open and save dialogs.
//!
//! The app only removes the lines it added itself, whose locations it
//! keeps in a small list of its own (one URI per line). A line the user or
//! another app wrote stays, even for a location pinned and then unpinned
//! here.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::location::same_location;
use crate::settings::Bookmark;

/// The bookmarks file under the user's configuration folder.
pub fn bookmarks_file(config_dir: &Path) -> PathBuf {
    config_dir.join("gtk-3.0").join("bookmarks")
}

/// The location a bookmarks line names: the text before the first space.
fn line_uri(line: &str) -> &str {
    line.split_once(' ').map_or(line, |(uri, _)| uri).trim()
}

/// A bookmarks file brought up to date with the pins.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MergedBookmarks {
    /// The new text of the file; `None` when it stays as it was.
    pub text: Option<String>,
    /// The locations whose lines the app added and still owns.
    pub owned: Vec<String>,
}

/// `text`, a bookmarks file, with a line appended for each of `pins` it
/// lacks and the lines the app added (`owned`) for locations no longer
/// pinned dropped; every other line stays as it was.
pub fn merged_bookmarks(text: &str, owned: &[String], pins: &[Bookmark]) -> MergedBookmarks {
    let is_pinned = |uri: &str| pins.iter().any(|pin| same_location(&pin.uri, uri));
    let is_owned = |uri: &str| owned.iter().any(|mine| same_location(mine, uri));
    let mut lines: Vec<String> = text
        .lines()
        .filter(|line| {
            let uri = line_uri(line);
            uri.is_empty() || is_pinned(uri) || !is_owned(uri)
        })
        .map(str::to_owned)
        .collect();
    let mut changed = lines.len() != text.lines().count();
    let mut still_owned: Vec<String> = owned.iter().filter(|uri| is_pinned(uri)).cloned().collect();
    for pin in pins {
        let listed = lines.iter().any(|line| same_location(line_uri(line), &pin.uri));
        if listed || pin.uri.contains(char::is_whitespace) {
            continue;
        }
        let label = pin.label.replace(['\n', '\r'], " ");
        lines.push(if label.is_empty() {
            pin.uri.clone()
        } else {
            format!("{} {label}", pin.uri)
        });
        if !still_owned.iter().any(|mine| same_location(mine, &pin.uri)) {
            still_owned.push(pin.uri.clone());
        }
        changed = true;
    }
    MergedBookmarks {
        text: changed.then(|| lines.iter().flat_map(|line| [line.as_str(), "\n"]).collect()),
        owned: still_owned,
    }
}

/// The text of `path`, or nothing when the file does not exist yet.
fn read_or_empty(path: &Path) -> io::Result<String> {
    match fs::read_to_string(path) {
        Ok(text) => Ok(text),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(String::new()),
        Err(error) => Err(error),
    }
}

/// Writes `text` beside `path` and renames it over it, so a failure never
/// leaves half a file. A symbolic link at `path` is kept: its target is
/// replaced.
fn replace_file(path: &Path, text: &str) -> io::Result<()> {
    let path = fs::canonicalize(path).unwrap_or_else(|_| path.to_owned());
    if let Some(folder) = path.parent() {
        fs::create_dir_all(folder)?;
    }
    let partial = path.with_extension("openxplorer-partial");
    fs::write(&partial, text)?;
    fs::rename(&partial, &path)
}

/// Brings the bookmarks file at `list` up to date with `pins`, creating it
/// if needed. `owned` is the app's own list of the lines it added.
/// The bookmarks file is written first, so an interruption at worst
/// leaves a line the app no longer removes; it never removes a line it
/// did not add.
///
/// # Errors
///
/// The file system's, when a file cannot be read or written.
pub fn sync_bookmarks(list: &Path, owned: &Path, pins: &[Bookmark]) -> io::Result<()> {
    let text = read_or_empty(list)?;
    let owned_before: Vec<String> = read_or_empty(owned)?
        .lines()
        .map(str::trim)
        .filter(|uri| !uri.is_empty())
        .map(str::to_owned)
        .collect();
    let merged = merged_bookmarks(&text, &owned_before, pins);
    if let Some(text) = merged.text {
        replace_file(list, &text)?;
    }
    if merged.owned != owned_before {
        let lines: String = merged.owned.iter().flat_map(|uri| [uri.as_str(), "\n"]).collect();
        replace_file(owned, &lines)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pin(uri: &str, label: &str) -> Bookmark {
        Bookmark {
            uri: uri.into(),
            label: label.into(),
        }
    }

    /// parity: SIDE-013
    #[test]
    fn pins_join_the_desktop_list_and_other_lines_stay() {
        let file = "file:///home/demo/Music\nsftp://build/srv Build server\n";
        let pins = [
            pin("file:///home/demo/Projects", "Projects"),
            pin("file:///home/demo/Music/", "Music"),
        ];

        let merged = merged_bookmarks(file, &[], &pins);
        let text = merged.text.expect("Projects is new");
        assert_eq!(
            text,
            "file:///home/demo/Music\nsftp://build/srv Build server\nfile:///home/demo/Projects Projects\n"
        );
        assert_eq!(
            merged.owned,
            ["file:///home/demo/Projects"],
            "Music was listed already"
        );
        let unpinned = merged_bookmarks(&text, &merged.owned, &[]);
        assert_eq!(
            unpinned.text.as_deref(),
            Some(file),
            "the user's Music line stays"
        );
        assert!(unpinned.owned.is_empty());
        assert_eq!(merged_bookmarks(file, &[], &[]).text, None, "nothing to write");
    }

    #[test]
    fn the_files_are_replaced_whole_and_a_linked_list_stays_linked() {
        let folder = tempfile::tempdir().expect("a temporary folder");
        let real = folder.path().join("dotfiles").join("bookmarks");
        fs::create_dir_all(real.parent().expect("a folder")).expect("the dotfiles");
        fs::write(&real, "file:///home/demo/Work Work\n").expect("the user's list");
        let path = bookmarks_file(folder.path());
        fs::create_dir_all(path.parent().expect("a folder")).expect("gtk-3.0");
        std::os::unix::fs::symlink(&real, &path).expect("a linked list");
        let owned = folder.path().join("owned");

        sync_bookmarks(&path, &owned, &[pin("file:///srv/work", "Work")]).expect("written");

        assert!(fs::symlink_metadata(&path).expect("the link").is_symlink());
        assert_eq!(
            fs::read_to_string(&real).expect("the file"),
            "file:///home/demo/Work Work\nfile:///srv/work Work\n"
        );
        assert_eq!(fs::read_to_string(&owned).expect("owned"), "file:///srv/work\n");
        sync_bookmarks(&path, &owned, &[pin("file:///home/demo/Work", "Work")]).expect("unpinned");
        assert_eq!(
            fs::read_to_string(&real).expect("the file"),
            "file:///home/demo/Work Work\n"
        );
        assert!(!real.with_extension("openxplorer-partial").exists());
    }
}
