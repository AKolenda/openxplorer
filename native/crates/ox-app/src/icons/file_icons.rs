// SPDX-License-Identifier: AGPL-3.0-only
//! The icons the user chose for kinds of item (LOOK-029), as Settings >
//! Appearance > File icons saves them in `fileIcons`.
//!
//! A folder, a ZIP archive or a file of one of the colour icons' types can
//! show another of the app's pictures, such as a ZIP archive shown as a
//! document with a zipper so it no longer looks like a folder, or a
//! picture of the user's. Files whose names end a certain way, such as
//! `.iso` or `.tar.gz`, can have their own; the longest ending that
//! matches wins over the file's type.
//!
//! The choice lives here, on the main thread, so that every
//! [`Art::for_entry`] follows it: folder views, Properties, the details
//! pane and the dialogs. Each change counts a new [`generation`], so a
//! folder view knows to draw its items again. A picture of the user's
//! stands in the icon's place where the views look up custom icons, after
//! the item's own icon and its preview.

use std::cell::{Cell, RefCell};

use ox_core::entry::Entry;
use ox_core::settings::FileIcons;

use super::{Art, FileType};

thread_local! {
    /// The chosen icons, by kind of item.
    static CHOSEN: RefCell<FileIcons> = const { RefCell::new(FileIcons::new()) };
    /// How many times the chosen icons changed.
    static GENERATION: Cell<u32> = const { Cell::new(0) };
}

/// One of the app's own pictures an item may show, by the name
/// `fileIcons` stores it under, as the File icons list offers them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ArtChoice {
    /// The name `fileIcons` stores.
    pub(crate) key: &'static str,
    /// What the picture shows.
    pub(crate) art: Art,
    /// Its name for people, translated where it is shown.
    pub(crate) name: &'static str,
}

/// The pictures the File icons list offers, in its order.
pub(crate) const ART_CHOICES: [ArtChoice; 10] = [
    ArtChoice {
        key: "folder",
        art: Art::Folder,
        name: crate::i18n::message_id("Folder"),
    },
    ArtChoice {
        key: "zip-folder",
        art: Art::ZipFolder,
        name: crate::i18n::message_id("Folder with a zipper"),
    },
    ArtChoice {
        key: "zip-document",
        art: Art::ZipDocument,
        name: crate::i18n::message_id("Document with a zipper"),
    },
    ArtChoice {
        key: "document",
        art: Art::File(FileType::Document),
        name: crate::i18n::message_id("Document"),
    },
    ArtChoice {
        key: "text",
        art: Art::File(FileType::Text),
        name: crate::i18n::message_id("Text document"),
    },
    ArtChoice {
        key: "spreadsheet",
        art: Art::File(FileType::Spreadsheet),
        name: crate::i18n::message_id("Spreadsheet"),
    },
    ArtChoice {
        key: "image",
        art: Art::File(FileType::Image),
        name: crate::i18n::message_id("Picture"),
    },
    ArtChoice {
        key: "video",
        art: Art::File(FileType::Video),
        name: crate::i18n::message_id("Video"),
    },
    ArtChoice {
        key: "audio",
        art: Art::File(FileType::Audio),
        name: crate::i18n::message_id("Music"),
    },
    ArtChoice {
        key: "code",
        art: Art::File(FileType::Code),
        name: crate::i18n::message_id("Code"),
    },
];

/// What an icon choice of `fileIcons` shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ChosenIcon {
    /// One of the app's pictures.
    Art(Art),
    /// The picture of the user's at this `file://` location.
    Picture(String),
}

impl ChosenIcon {
    /// The icon `fileIcons` stores as `icon`; `None` for one the app does
    /// not know.
    pub(crate) fn from_key(icon: &str) -> Option<Self> {
        if let Some(choice) = ART_CHOICES.iter().find(|choice| choice.key == icon) {
            return Some(ChosenIcon::Art(choice.art));
        }
        ox_core::settings::is_file_icon(icon).then(|| ChosenIcon::Picture(icon.to_owned()))
    }
}

/// The kind of item the usual art `usual` stands for, as `fileIcons`
/// names it; `None` for art no choice replaces, such as a network share.
/// A standard folder is a folder, and the file kinds Windows icon files
/// tell apart (LOOK-030) are the documents and code they were before.
pub(crate) fn kind_of(usual: Art) -> Option<&'static str> {
    Some(match usual {
        Art::Folder | Art::StandardFolder(_) => "folder",
        Art::ZipFolder => "zip",
        Art::File(
            FileType::Document
            | FileType::Pdf
            | FileType::Archive
            | FileType::Executable
            | FileType::Shortcut,
        ) => "document",
        Art::File(FileType::Text) => "text",
        Art::File(FileType::Spreadsheet) => "spreadsheet",
        Art::File(FileType::Image) => "image",
        Art::File(FileType::Video) => "video",
        Art::File(FileType::Audio) => "audio",
        Art::File(FileType::Code | FileType::Script) => "code",
        _ => return None,
    })
}

/// Counts a new generation without a change of the chosen icons, for
/// another source of the same icons: Windows icon files were read or
/// turned off (LOOK-030), so a view draws its items again.
pub(crate) fn count_new_generation() {
    GENERATION.set(GENERATION.get().wrapping_add(1));
}

/// Follows the icons chosen in `icons`, counting a new generation when
/// they changed.
pub(crate) fn follow(icons: &FileIcons) {
    let changed = CHOSEN.with_borrow_mut(|chosen| {
        if chosen == icons {
            return false;
        }
        chosen.clone_from(icons);
        true
    });
    if changed {
        GENERATION.set(GENERATION.get().wrapping_add(1));
    }
}

/// How many times the chosen icons changed, so a view can tell whether
/// the icons it drew are still the chosen ones.
pub(crate) fn generation() -> u32 {
    GENERATION.get()
}

/// The icon chosen for an item called `name` whose usual art is `usual`:
/// the one for the longest matching name ending, else the one for its
/// kind. `None` when none was chosen.
fn chosen_for(name: &str, usual: Art) -> Option<ChosenIcon> {
    let kind = kind_of(usual)?;
    CHOSEN.with_borrow(|chosen| {
        let by_ending = (!matches!(usual, Art::Folder | Art::StandardFolder(_)))
            .then(|| {
                let name = name.to_lowercase();
                chosen
                    .iter()
                    .filter(|(ending, _)| ending.starts_with('.') && name.ends_with(ending.as_str()))
                    .max_by_key(|(ending, _)| ending.len())
                    .map(|(_, icon)| icon)
            })
            .flatten();
        by_ending
            .or_else(|| chosen.get(kind))
            .and_then(|icon| ChosenIcon::from_key(icon))
    })
}

/// The art an item called `name` shows instead of its usual art `usual`:
/// the picture chosen for it, or `usual` when none was, or when a picture
/// of the user's stands in its place.
pub(crate) fn art_for(name: &str, usual: Art) -> Art {
    match chosen_for(name, usual) {
        Some(ChosenIcon::Art(art)) => art,
        Some(ChosenIcon::Picture(_)) | None => usual,
    }
}

/// The location of the picture of the user's that `entry` shows in its
/// icon's place, if one was chosen for it.
pub(crate) fn picture_for(entry: &Entry) -> Option<String> {
    if entry.is_virtual {
        return None;
    }
    match chosen_for(&entry.name, Art::usual_for_entry(entry))? {
        ChosenIcon::Picture(uri) => Some(uri),
        ChosenIcon::Art(_) => None,
    }
}
