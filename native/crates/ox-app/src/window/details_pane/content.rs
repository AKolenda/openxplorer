// SPDX-License-Identifier: AGPL-3.0-only
//! What the details pane says about the selection, computed without
//! widgets so it is tested on its own.
//!
//! It follows Windows 11 File Explorer's Details pane: with nothing
//! selected, the folder's picture, its name and "N items"; with one item,
//! its picture, its name and rows of Type, Size, Date modified, Date
//! created and Availability; with several, their pictures stacked, "N items
//! selected" and their total size. A folder on an SMB share is pictured as
//! its tab and sidebar row picture it, on the network bar (the owner's icon
//! mapping, 2026-09-28).

use std::path::PathBuf;

use gtk::gio;
use gtk::prelude::FileExt;
use ox_core::format;
use ox_core::i18n::message_id;
use ox_core::location::{is_smb_location, ArchiveLocation, LocationContext};
use ox_core::places::NetworkLocation;

use crate::folder_view::item::FileItem;
use crate::icons::Art;
use crate::properties::NOT_SCANNED;

/// What Windows says of an item stored on this computer.
const AVAILABLE_HERE: &str = message_id("Available on this device");

/// How many of several selected items the stacked picture shows.
const STACKED_ITEMS: usize = 3;

/// The picture at the top of the pane.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::window) enum Preview {
    /// The one selected item's art, or the folder's.
    Art(Art),
    /// Several selected items: the art of the first few, stacked.
    Stack(Vec<Art>),
}

/// What the preview area shows of one selected file's content
/// (PROP-011).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::window) enum MediaPreview {
    /// A local image, decoded and scaled; `size` is its size in bytes.
    Image { path: PathBuf, size: u64 },
    /// A local audio (`is_video` false) or video file, with a player;
    /// audio keeps the item's `art` above its controls.
    Recording { path: PathBuf, is_video: bool, art: Art },
    /// Another local document, shown by its cached thumbnail if it has
    /// one.
    Document { uri: String },
}

impl MediaPreview {
    /// The preview of `item`: only a local file has one.
    fn of(item: &FileItem) -> Option<Self> {
        let entry = item.entry();
        if entry.is_dir || entry.is_virtual {
            return None;
        }
        let path = gio::File::for_uri(&entry.uri)
            .path()
            .filter(|_| entry.uri.starts_with("file:"))?;
        let content_type = entry.content_type.as_deref().unwrap_or_default();
        let media = content_type.split('/').next().unwrap_or_default();
        Some(match media {
            "image" => Self::Image {
                path,
                size: entry.size.unwrap_or_default(),
            },
            "audio" | "video" => Self::Recording {
                path,
                is_video: media == "video",
                art: item.art(),
            },
            _ => Self::Document {
                uri: entry.uri.clone(),
            },
        })
    }
}

/// A row the pane can show, which its menu can turn off (PROP-010).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::window) enum Field {
    Type,
    Size,
    Modified,
    Created,
    Availability,
    Dimensions,
    Length,
}

impl Field {
    /// Every field, in the order the pane shows them.
    pub(in crate::window) const ALL: [Self; 7] = [
        Self::Type,
        Self::Size,
        Self::Modified,
        Self::Created,
        Self::Availability,
        Self::Dimensions,
        Self::Length,
    ];

    /// The name the settings save a turned-off field under.
    pub(in crate::window) fn key(self) -> &'static str {
        match self {
            Self::Type => "Type",
            Self::Size => "Size",
            Self::Modified => "Modified",
            Self::Created => "Created",
            Self::Availability => "Availability",
            Self::Dimensions => "Dimensions",
            Self::Length => "Length",
        }
    }

    /// The row's name, as Windows' Details pane writes it.
    pub(in crate::window) fn label(self) -> &'static str {
        let label = match self {
            Self::Type => message_id("Type"),
            Self::Size => message_id("Size"),
            Self::Modified => message_id("Date modified"),
            Self::Created => message_id("Date created"),
            Self::Availability => message_id("Availability"),
            Self::Dimensions => message_id("Dimensions"),
            Self::Length => message_id("Length"),
        };
        ox_core::i18n::gettext_static(label)
    }
}

/// One row of the pane's list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::window) struct Property {
    /// What the row describes.
    pub field: Field,
    /// The value as the pane shows it.
    pub value: String,
}

impl Property {
    fn new(field: Field, value: String) -> Self {
        Self { field, value }
    }
}

/// Everything the pane shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::window) struct PaneContent {
    /// The picture at the top.
    pub preview: Preview,
    /// The item's or folder's name, or "N items selected".
    pub name: String,
    /// The line under a folder's name, "N items"; empty for a selection.
    pub summary: String,
    /// The rows, in display order.
    pub properties: Vec<Property>,
    /// The content preview of one selected file.
    pub media: Option<MediaPreview>,
}

/// What [`pane_content`] reads.
#[derive(Debug, Clone, Copy)]
pub(in crate::window) struct PaneFacts<'a> {
    /// The selected items.
    pub selection: &'a [FileItem],
    /// The folder or page the tab shows.
    pub folder_uri: &'a str,
    /// Items the folder lists, as its status bar counts them.
    pub folder_item_count: u32,
    /// Display names for devices and the home folder.
    pub locations: &'a LocationContext,
    /// The Network list, for the picture of an SMB folder.
    pub network: &'a [NetworkLocation],
    /// Dates show the day only; else the time too (PROP-010).
    pub condensed_dates: bool,
}

/// What the pane shows for the selection in a folder.
pub(in crate::window) fn pane_content(facts: &PaneFacts<'_>) -> PaneContent {
    match facts.selection {
        [] => folder_content(facts),
        [item] => item_content(item, facts),
        several => several_content(several),
    }
}

fn item_content(item: &FileItem, facts: &PaneFacts<'_>) -> PaneContent {
    let entry = item.entry();
    let mut properties = vec![
        Property::new(Field::Type, entry.type_label.clone()),
        Property::new(Field::Size, size_text(item)),
        Property::new(Field::Modified, date_text(entry.modified, facts.condensed_dates)),
    ];
    if entry.meta.created.is_some() {
        properties.push(Property::new(
            Field::Created,
            date_text(entry.meta.created, facts.condensed_dates),
        ));
    }
    if entry.uri.starts_with("file:") {
        properties.push(Property::new(
            Field::Availability,
            ox_core::i18n::gettext_static(AVAILABLE_HERE).to_owned(),
        ));
    }
    PaneContent {
        preview: Preview::Art(item.art()),
        name: entry.name.clone(),
        summary: String::new(),
        properties,
        media: MediaPreview::of(item),
    }
}

fn folder_content(facts: &PaneFacts<'_>) -> PaneContent {
    let uri = facts.folder_uri;
    // A ZIP's own top, open like a folder, has the ZIP's picture.
    let art = if is_zip_top(uri) {
        Art::ZipFolder
    } else {
        folder_art(facts)
    };
    PaneContent {
        preview: Preview::Art(art),
        name: facts.locations.title_for(uri),
        summary: count_text(facts.folder_item_count),
        properties: Vec::new(),
        media: None,
    }
}

/// "N items selected" with the art of the first few stacked, and their
/// total size while every one is a file, as Windows' status bar adds it.
fn several_content(selection: &[FileItem]) -> PaneContent {
    let selected = selection.len() as u64;
    let name = ox_core::i18n::format_message(
        "{selected} items selected",
        &[("selected", &format::group_digits(selected))],
    );
    let arts = selection.iter().take(STACKED_ITEMS).map(FileItem::art).collect();
    let mut total = Some(0_u64);
    for item in selection {
        let entry = item.entry();
        total = total
            .zip(entry.size.filter(|_| !entry.is_dir))
            .map(|(sum, size)| sum.saturating_add(size));
    }
    let properties = total
        .map(|bytes| Property::new(Field::Size, format::pretty_bytes(bytes)))
        .into_iter()
        .collect();
    PaneContent {
        preview: Preview::Stack(arts),
        name,
        summary: String::new(),
        properties,
        media: None,
    }
}

/// "1 item" or "12 items", as the status bar counts them.
fn count_text(count: u32) -> String {
    ox_core::i18n::ngettext("{count} item", "{count} items", u64::from(count))
        .replace("{count}", &format::group_digits(u64::from(count)))
}

/// Whether `uri` is the top of a ZIP opened like a folder (ARC-026).
fn is_zip_top(uri: &str) -> bool {
    ArchiveLocation::from_uri(uri).is_some_and(|location| location.parent().is_none())
}

/// The picture of the folder itself: on an SMB share the art its tab
/// shows, else the folder.
fn folder_art(facts: &PaneFacts<'_>) -> Art {
    if is_smb_location(facts.folder_uri) {
        Art::for_smb_location(facts.folder_uri, facts.network)
    } else {
        Art::Folder
    }
}

/// The size line: a file's size, a folder's measured size, or "Not
/// scanned" for a folder never measured (PROP-027).
fn size_text(item: &FileItem) -> String {
    let entry = item.entry();
    match (entry.is_dir, item.folder_size(), entry.size) {
        (true, Some(measured), _) => measured.size_text(),
        (true, None, _) => ox_core::i18n::gettext_static(NOT_SCANNED).to_owned(),
        (false, _, Some(size)) => format::pretty_bytes(size),
        (false, _, None) => String::new(),
    }
}

/// A date row: the day, or the day and time.
fn date_text(time: Option<u64>, condensed: bool) -> String {
    if condensed {
        format::date_text(time)
    } else {
        format::date_time_text(time)
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::locations::Page;
    use crate::test_support::{file_entry, folder_entry, studio_nas_mapped_drive};

    fn context() -> LocationContext {
        LocationContext {
            home: Some(PathBuf::from("/home/demo")),
            ..LocationContext::default()
        }
    }

    fn content_for(selection: &[FileItem], folder_uri: &str) -> PaneContent {
        let locations = context();
        let network = [studio_nas_mapped_drive()];
        pane_content(&PaneFacts {
            selection,
            folder_uri,
            folder_item_count: 7,
            locations: &locations,
            network: &network,
            condensed_dates: true,
        })
    }

    fn property(content: &PaneContent, field: Field) -> Option<&str> {
        content
            .properties
            .iter()
            .find(|property| property.field == field)
            .map(|property| property.value.as_str())
    }

    /// parity: PROP-009
    #[test]
    fn a_file_shows_windows_rows_under_its_name() {
        let mut entry = file_entry("Notes.txt");
        entry.size = Some(2048);
        entry.meta.created = Some(1_780_000_000);
        let content = content_for(&[FileItem::new(entry.clone())], "file:///tmp/ox-test");
        let fields: Vec<Field> = content.properties.iter().map(|property| property.field).collect();
        assert_eq!(
            fields,
            [
                Field::Type,
                Field::Size,
                Field::Modified,
                Field::Created,
                Field::Availability
            ]
        );
        assert_eq!(content.name, "Notes.txt");
        assert_eq!(
            content.summary, "",
            "the type is a row, not a line under the name"
        );
        assert_eq!(
            property(&content, Field::Size),
            Some(format::pretty_bytes(2048).as_str())
        );
        assert_eq!(
            property(&content, Field::Modified),
            Some(format::date_text(entry.modified).as_str())
        );
        assert_eq!(
            property(&content, Field::Created),
            Some(format::date_text(entry.meta.created).as_str())
        );
        assert_eq!(
            property(&content, Field::Availability),
            Some("Available on this device")
        );
    }

    /// An item off this computer says nothing of its availability, and an
    /// unknown creation time leaves out its row.
    ///
    /// parity: PROP-009
    #[test]
    fn a_shared_file_has_no_availability_or_unknown_creation_row() {
        let mut entry = file_entry("Notes.txt");
        entry.uri = "smb://nas/work/Notes.txt".to_owned();
        entry.meta.created = None;
        let content = content_for(&[FileItem::new(entry)], "smb://nas/work");
        assert_eq!(property(&content, Field::Availability), None);
        assert_eq!(property(&content, Field::Created), None);
    }

    /// Condensed dates give the day only; otherwise the time follows.
    ///
    /// parity: PROP-010
    #[test]
    fn condensed_dates_drop_the_time() {
        let modified = Some(1_790_000_000);
        assert_eq!(date_text(modified, true), format::date_text(modified));
        assert_eq!(date_text(modified, false), format::date_time_text(modified));
        assert!(date_text(modified, false).starts_with(&date_text(modified, true)));
    }

    /// Only a local file has a content preview, chosen by its type.
    ///
    /// parity: PROP-011
    #[test]
    fn a_local_file_is_previewed_by_its_type() {
        let mut photo = file_entry("Beach.png");
        photo.uri = "file:///tmp/ox-test/Beach.png".to_owned();
        photo.content_type = Some("image/png".to_owned());
        photo.size = Some(10);
        let content = content_for(&[FileItem::new(photo.clone())], "file:///tmp/ox-test");
        assert_eq!(
            content.media,
            Some(MediaPreview::Image {
                path: PathBuf::from("/tmp/ox-test/Beach.png"),
                size: 10
            })
        );
        photo.content_type = Some("video/mp4".to_owned());
        assert!(matches!(
            MediaPreview::of(&FileItem::new(photo.clone())),
            Some(MediaPreview::Recording { is_video: true, .. })
        ));
        photo.uri = "smb://nas/media/Beach.png".to_owned();
        assert_eq!(
            MediaPreview::of(&FileItem::new(photo)),
            None,
            "shares are not read for a preview"
        );
        assert_eq!(content_for(&[], "file:///tmp/ox-test").media, None);
    }

    /// parity: PROP-009, PROP-027
    #[test]
    fn a_folder_is_not_scanned_until_measured() {
        let item = FileItem::new(folder_entry("Projects"));
        let content = content_for(&[item], "file:///tmp/ox-test");
        assert_eq!(property(&content, Field::Size), Some("Not scanned"));
    }

    /// parity: PROP-009
    #[test]
    fn no_selection_names_the_folder_and_counts_its_items() {
        let content = content_for(&[], "file:///home/demo");
        assert_eq!(
            (content.name.as_str(), content.summary.as_str()),
            ("Home", "7 items")
        );
        assert_eq!(content.preview, Preview::Art(Art::Folder));
        assert!(content.properties.is_empty());
        let landing = content_for(&[], Page::ThisPc.uri());
        assert_eq!(landing.summary, "7 items");
    }

    /// A ZIP opened like a folder has the ZIP's picture at its top, and its
    /// folders are folders.
    ///
    /// parity: PROP-009, ARC-026
    #[test]
    fn the_top_of_a_zip_has_the_zip_picture() {
        let top = ArchiveLocation::root("file:///tmp/ox-test/Bundle.zip");
        let content = content_for(&[], &top.uri());
        assert_eq!(content.preview, Preview::Art(Art::ZipFolder));
        let inside = content_for(&[], &top.member("Docs/").uri());
        assert_eq!(inside.preview, Preview::Art(Art::Folder));
    }

    /// parity: PROP-009
    #[test]
    fn several_items_are_counted_stacked_and_summed() {
        let mut first = file_entry("a.txt");
        first.size = Some(1000);
        let mut second = file_entry("b.txt");
        second.size = Some(24);
        let files = [FileItem::new(first), FileItem::new(second)];
        let content = content_for(&files, "smb://nas/media");
        assert_eq!(content.name, "2 items selected");
        assert_eq!(content.summary, "");
        assert_eq!(
            content.preview,
            Preview::Stack(files.iter().map(FileItem::art).collect())
        );
        assert_eq!(
            property(&content, Field::Size),
            Some(format::pretty_bytes(1024).as_str())
        );
        let with_folder = [files[0].clone(), FileItem::new(folder_entry("Projects"))];
        let content = content_for(&with_folder, "file:///tmp/ox-test");
        assert_eq!(
            property(&content, Field::Size),
            None,
            "a folder's size is not known"
        );
        let many: Vec<FileItem> = (0..5)
            .map(|n| FileItem::new(file_entry(&format!("{n}.txt"))))
            .collect();
        let Preview::Stack(arts) = content_for(&many, "file:///tmp/ox-test").preview else {
            panic!("several items are stacked");
        };
        assert_eq!(arts.len(), STACKED_ITEMS);
    }

    /// parity: LOOK-016
    #[test]
    fn an_smb_folder_is_pictured_on_the_network_bar_as_its_tab_is() {
        let mapped_drive = studio_nas_mapped_drive();
        let drive_root = content_for(&[], &mapped_drive.uri);
        let drive_art = Art::for_network_row(&mapped_drive);
        assert_eq!(drive_root.preview, Preview::Art(drive_art));
        let inside = content_for(&[], "smb://studio-nas/projects/2024");
        assert_eq!(inside.preview, Preview::Art(Art::SHARE));
        let local = content_for(&[], "file:///home/demo");
        assert_eq!(local.preview, Preview::Art(Art::Folder));
    }
}
