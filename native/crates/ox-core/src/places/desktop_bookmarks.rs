// SPDX-License-Identifier: AGPL-3.0-only
//! The desktop-wide places list (SIDE-013).
//!
//! GTK's file dialogs, GNOME Files and other GTK file managers list the
//! bookmarks in `$XDG_CONFIG_HOME/gtk-3.0/bookmarks`, one `uri label`
//! per line. Quick access pins are kept in the app's own settings,
//! where the Python app reads them too; this module mirrors them into that
//! file, so a folder pinned here also appears in open and save dialogs.
//! Lines the user or other apps wrote are kept as they are.

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

/// `text`, a bookmarks file, with a line for each of `added` it lacks
/// appended and the lines of `removed` dropped; every other line stays as
/// it was. `None` when nothing changes.
pub fn merged_bookmarks(text: &str, added: &[Bookmark], removed: &[String]) -> Option<String> {
    let mut lines: Vec<String> = text
        .lines()
        .filter(|line| {
            let uri = line_uri(line);
            uri.is_empty() || !removed.iter().any(|gone| same_location(gone, uri))
        })
        .map(str::to_owned)
        .collect();
    let mut changed = lines.len() != text.lines().count();
    for pin in added {
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
        changed = true;
    }
    changed.then(|| lines.iter().flat_map(|line| [line.as_str(), "\n"]).collect())
}

/// Adds `added` to and drops `removed` from the bookmarks file at `path`,
/// creating it if needed. The new file is written beside it and renamed
/// over it, so a failure never leaves half a file.
///
/// # Errors
///
/// The file system's, when the file cannot be read or written.
pub fn sync_bookmarks(path: &Path, added: &[Bookmark], removed: &[String]) -> io::Result<()> {
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == io::ErrorKind::NotFound => String::new(),
        Err(error) => return Err(error),
    };
    let Some(merged) = merged_bookmarks(&text, added, removed) else {
        return Ok(());
    };
    if let Some(folder) = path.parent() {
        fs::create_dir_all(folder)?;
    }
    let partial = path.with_extension("openxplorer-partial");
    fs::write(&partial, merged)?;
    fs::rename(&partial, path)
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
        let added = [
            pin("file:///home/demo/Projects", "Projects"),
            pin("file:///home/demo/Music/", "Music"),
        ];

        let merged = merged_bookmarks(file, &added, &[]).expect("Projects is new");
        assert_eq!(
            merged,
            "file:///home/demo/Music\nsftp://build/srv Build server\nfile:///home/demo/Projects Projects\n"
        );
        let unpinned = merged_bookmarks(&merged, &[], &["file:///home/demo/Projects".into()]);
        assert_eq!(unpinned.as_deref(), Some(file));
        assert_eq!(merged_bookmarks(file, &[], &[]), None, "nothing to write");
    }

    #[test]
    fn the_file_is_created_and_replaced_whole() {
        let folder = tempfile::tempdir().expect("a temporary folder");
        let path = bookmarks_file(folder.path());

        sync_bookmarks(&path, &[pin("file:///srv/work", "Work")], &[]).expect("written");

        assert_eq!(
            fs::read_to_string(&path).expect("the file"),
            "file:///srv/work Work\n"
        );
        assert!(!path.with_extension("openxplorer-partial").exists());
    }
}
