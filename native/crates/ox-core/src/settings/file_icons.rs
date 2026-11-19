// SPDX-License-Identifier: AGPL-3.0-only
//! File icons (LOOK-029): the icon each kind of item shows, chosen in
//! Settings > Appearance > File icons, for people who cannot tell a ZIP
//! archive from a folder, or one type from another, by its usual icon.
//!
//! Stored as `fileIcons`, an object from a kind of item to its icon. A kind
//! is one of [`FILE_ICON_KINDS`] or a file name ending such as `.iso` or
//! `.tar.gz`; an icon is one of the app's own pictures,
//! [`FILE_ICON_ARTS`], or the `file://` location of a picture of the
//! user's. A kind left out keeps its usual icon.

use std::collections::BTreeMap;

use serde_json::Value;

/// The chosen icons, by kind of item.
pub type FileIcons = BTreeMap<String, String>;

/// The kinds of item the File icons list offers, in its order: folders,
/// ZIP archives, then the file types the colour icons show.
pub const FILE_ICON_KINDS: [&str; 9] = [
    "folder",
    "zip",
    "document",
    "text",
    "spreadsheet",
    "image",
    "video",
    "audio",
    "code",
];

/// The app's own pictures an item may show, by the names `fileIcons`
/// stores them under.
pub const FILE_ICON_ARTS: [&str; 10] = [
    "folder",
    "zip-folder",
    "zip-document",
    "document",
    "text",
    "spreadsheet",
    "image",
    "video",
    "audio",
    "code",
];

/// The most kinds `fileIcons` keeps.
pub const MAX_FILE_ICONS: usize = 64;
/// The longest file name ending kept, dot included.
const MAX_ENDING: usize = 24;
/// The longest picture location kept.
const MAX_PICTURE_LOCATION: usize = 4096;

/// `text`, typed as a file name ending ("iso", ".ISO", "*.tar.gz"), in the
/// form `fileIcons` keeps it: lower case, with one leading dot. `None` for
/// text that is no ending: empty, too long, or with anything but letters,
/// digits, `+`, `-`, `_` and single dots between them.
pub fn file_name_ending(text: &str) -> Option<String> {
    let text = text.trim();
    let text = text.strip_prefix('*').unwrap_or(text);
    let text = text.strip_prefix('.').unwrap_or(text).to_lowercase();
    let parts_are_words = text.split('.').all(|part| {
        !part.is_empty()
            && part
                .chars()
                .all(|c| c.is_alphanumeric() || matches!(c, '+' | '-' | '_'))
    });
    let ending = format!(".{text}");
    (parts_are_words && ending.chars().count() <= MAX_ENDING).then_some(ending)
}

/// Whether `kind` is a kind `fileIcons` may hold: one of
/// [`FILE_ICON_KINDS`] or a file name ending in its kept form.
pub fn is_file_icon_kind(kind: &str) -> bool {
    FILE_ICON_KINDS.contains(&kind) || file_name_ending(kind).as_deref() == Some(kind)
}

/// Whether `icon` is an icon `fileIcons` may hold: one of
/// [`FILE_ICON_ARTS`] or a local picture's `file://` location.
pub fn is_file_icon(icon: &str) -> bool {
    FILE_ICON_ARTS.contains(&icon)
        || (icon.starts_with("file:///")
            && icon.len() <= MAX_PICTURE_LOCATION
            && !icon.contains(char::is_control))
}

/// The entries of `icons` the app can show, at most [`MAX_FILE_ICONS`].
/// The file may have been edited by hand.
pub(super) fn canonical_file_icons(icons: &FileIcons) -> FileIcons {
    icons
        .iter()
        .filter(|(kind, icon)| is_file_icon_kind(kind) && is_file_icon(icon))
        .take(MAX_FILE_ICONS)
        .map(|(kind, icon)| (kind.clone(), icon.clone()))
        .collect()
}

/// The text entries of a `fileIcons` object, or `None` when `value` is no
/// object.
pub(super) fn read_file_icons(value: &Value) -> Option<FileIcons> {
    Some(
        value
            .as_object()?
            .iter()
            .filter_map(|(kind, icon)| Some((kind.clone(), icon.as_str()?.to_owned())))
            .collect(),
    )
}
