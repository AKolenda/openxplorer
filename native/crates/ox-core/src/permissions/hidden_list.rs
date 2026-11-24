// SPDX-License-Identifier: AGPL-3.0-only
//! The Hidden attribute kept in a folder's `.hidden` list (PROP-002,
//! PROP-005).
//!
//! In Windows, Hidden is an attribute: "The file or directory is hidden.
//! It is not included in an ordinary directory listing." Setting it never
//! renames the item, so links, scripts and other apps' recent lists keep
//! working. Linux has no such attribute, but GIO, Nautilus, Dolphin and
//! this app hide every name listed, one per line, in the folder's
//! `.hidden` file. Properties' Hidden box adds the name there instead of
//! renaming the item to a dot-name; an item hidden by its name's leading
//! dot is still shown hidden, and clearing the box then removes the dot,
//! as before.
//!
//! The list is rewritten whole through a new file renamed over it, so a
//! crash never leaves half a list, and every line the app did not ask to
//! change is kept as it was.

use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use gio::prelude::*;

/// The file in a folder that lists the names hidden there.
pub const HIDDEN_LIST: &str = ".hidden";

/// The local path of the `.hidden` list of the folder that holds the item
/// at `uri`; `None` for an item that is not on this computer, where no
/// list can be written and Hidden renames instead.
pub fn hidden_list_for(uri: &str) -> Option<PathBuf> {
    let file = gio::File::for_uri(uri);
    if file.uri_scheme().as_deref() != Some("file") {
        return None;
    }
    Some(file.path()?.parent()?.join(HIDDEN_LIST))
}

/// Whether `name` is listed in the `.hidden` list at `list`. A missing or
/// unreadable list lists nothing.
pub fn is_listed(list: &Path, name: &str) -> bool {
    fs::read_to_string(list).is_ok_and(|text| text.lines().any(|line| line == name))
}

/// Adds `name` to the `.hidden` list at `list` for `hide`, or removes every
/// line that is exactly `name` for not `hide`. Nothing is written when the
/// list already says so, and a list that would be left empty is removed
/// only if the app is what emptied it.
///
/// # Errors
///
/// The error of reading or writing the list, or of a name that cannot be
/// listed (one holding a line break).
pub fn set_listed(list: &Path, name: &str, hide: bool) -> io::Result<()> {
    if name.is_empty() || name.contains(['\n', '\r']) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "a name with a line break cannot be listed as hidden",
        ));
    }
    let text = match fs::read_to_string(list) {
        Ok(text) => text,
        Err(error) if error.kind() == io::ErrorKind::NotFound => String::new(),
        Err(error) => return Err(error),
    };
    let lines: Vec<&str> = text.lines().collect();
    let listed = lines.contains(&name);
    if listed == hide {
        return Ok(());
    }
    let mut kept: Vec<&str> = lines.into_iter().filter(|line| *line != name).collect();
    if hide {
        kept.push(name);
    }
    if kept.iter().all(|line| line.trim().is_empty()) {
        return fs::remove_file(list).or_else(|error| match error.kind() {
            io::ErrorKind::NotFound => Ok(()),
            _ => Err(error),
        });
    }
    let mut new_text = kept.join("\n");
    new_text.push('\n');
    write_replacing(list, &new_text)
}

/// Writes `text` to a new file beside `list` and renames it over `list`.
fn write_replacing(list: &Path, text: &str) -> io::Result<()> {
    let folder = list.parent().unwrap_or_else(|| Path::new("."));
    let staging = folder.join(format!(".hidden.openxplorer-{}.part", std::process::id()));
    let written = (|| {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&staging)?;
        file.write_all(text.as_bytes())?;
        file.sync_all()?;
        fs::rename(&staging, list)
    })();
    if written.is_err() {
        let _ = fs::remove_file(&staging);
    }
    written
}
