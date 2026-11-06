// SPDX-License-Identifier: AGPL-3.0-only
//! What the details pane says about the selection, computed without
//! widgets so it is tested on its own.
//!
//! Laid out as Windows 11 Explorer's Details pane (PROP-009). One selected
//! item shows its large preview, its small icon and name, and under
//! "Details" its Type, Size, File location, Date modified and Date
//! created; a picture, video or audio file adds the rows its own metadata
//! gives once the reader has read them ([`MetadataRequest`]). Several
//! selected items show "N items selected" with their total size, the
//! range of their dates and the folder they are in; nothing selected
//! shows the folder's item count. A folder on an SMB share is pictured as
//! its tab and sidebar row picture it, on the network bar (the owner's
//! icon mapping, 2026-09-28).

use std::path::PathBuf;

use gtk::gio;
use gtk::prelude::FileExt;
use ox_core::format;
use ox_core::location::{is_smb_location, parent_location, ArchiveLocation, LocationContext};
use ox_core::places::NetworkLocation;

use crate::folder_view::item::FileItem;
use crate::i18n::message_id;
use crate::icons::{Art, Icon};
use crate::media_details::MediaKind;

/// The keys of the rows the pane's menu can turn off (PROP-010), which
/// are also the names the settings keep them under.
pub(in crate::window) mod keys {
    pub(in crate::window) const TYPE: &str = "Type";
    pub(in crate::window) const SIZE: &str = "Size";
    pub(in crate::window) const LOCATION: &str = "Location";
    pub(in crate::window) const MODIFIED: &str = "Modified";
    pub(in crate::window) const CREATED: &str = "Created";
    pub(in crate::window) const DIMENSIONS: &str = "Dimensions";
    pub(in crate::window) const LENGTH: &str = "Length";
}

/// The picture at the top of the pane.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::window) enum Preview {
    /// The one selected item's art, or the folder's.
    Art(Art),
    /// The copy glyph for several items.
    Several,
}

impl Preview {
    /// The art beside the name: the same picture, small.
    pub(in crate::window) fn art(self) -> Art {
        match self {
            Preview::Art(art) => art,
            Preview::Several => Art::Glyph(Icon::Copy),
        }
    }
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
    /// A local folder, shown by its own picture if it has one
    /// ([`super::media`]'s `folder_picture`), else by its art.
    Folder { uri: String },
}

impl MediaPreview {
    /// Whether this is the preview of the item at `uri`.
    pub(in crate::window) fn is_of(&self, uri: &str) -> bool {
        match self {
            Self::Image { path, .. } | Self::Recording { path, .. } => gio::File::for_path(path).uri() == uri,
            Self::Document { uri: shown } | Self::Folder { uri: shown } => shown == uri,
        }
    }

    /// The preview of `item`: only a local file or folder has one.
    fn of(item: &FileItem) -> Option<Self> {
        let entry = item.entry();
        if entry.is_virtual {
            return None;
        }
        if entry.is_dir {
            return entry.uri.starts_with("file:").then(|| Self::Folder {
                uri: entry.uri.clone(),
            });
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

/// The file whose own details (a photo's camera, a song's artist) the
/// pane has the metadata reader read, as Properties' Details tab does
/// (PROP-034). A request for the same file, unchanged, is the same
/// request, so a redraw never reads it again.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::window) struct MetadataRequest {
    pub uri: String,
    pub kind: MediaKind,
    pub size: Option<u64>,
    pub modified: Option<u64>,
}

impl MetadataRequest {
    /// The request for `item`: a file, not a folder, of a type with media
    /// details.
    fn of(item: &FileItem) -> Option<Self> {
        let entry = item.entry();
        if entry.is_dir {
            return None;
        }
        let kind = entry
            .content_type
            .as_deref()
            .and_then(MediaKind::for_content_type)?;
        Some(Self {
            uri: entry.uri.clone(),
            kind,
            size: entry.size,
            modified: entry.modified,
        })
    }
}

/// One row of the Details list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::window) struct Property {
    /// The row's key, which the pane's menu turns it off by.
    pub key: &'static str,
    /// What the row describes, as shown, such as "Date modified".
    pub name: String,
    /// The value as the pane shows it.
    pub value: String,
}

impl Property {
    fn new(key: &'static str, name: &'static str, value: String) -> Self {
        Self {
            key,
            name: ox_core::i18n::gettext_static(name).to_owned(),
            value,
        }
    }
}

/// What the pane describes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::window) enum Subject {
    /// Nothing is selected: the folder, with Windows' hint box and
    /// nothing to share.
    Folder,
    /// One or more selected items, which may be shared.
    Selection,
}

/// Everything the pane shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::window) struct PaneContent {
    /// The picture at the top, and the small art beside the name.
    pub preview: Preview,
    /// The item's name, "N items selected", or with nothing selected the
    /// folder's name and count: "Downloads (2 items)".
    pub name: String,
    /// The small icon shows beside the name: for a file, or several
    /// items; Windows shows a folder's name alone.
    pub small_icon: bool,
    /// What the pane describes: with nothing selected, Windows' hint box
    /// takes the Details rows' place; a selection may be shared.
    pub subject: Subject,
    /// Properties can open for what the pane shows: not on a landing
    /// page with nothing selected.
    pub has_properties: bool,
    /// The Details rows, in display order.
    pub properties: Vec<Property>,
    /// The content preview of one selected file.
    pub media: Option<MediaPreview>,
    /// The file whose own details are read and added to the rows.
    pub metadata: Option<MetadataRequest>,
}

/// What [`pane_content`] reads.
#[derive(Debug, Clone, Copy)]
pub(in crate::window) struct PaneFacts<'a> {
    /// The selected items.
    pub selection: &'a [FileItem],
    /// The folder or page the tab shows.
    pub folder_uri: &'a str,
    /// Items the list shows: the folder's, hidden ones while they are
    /// shown, or a search's results.
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
        several => several_content(several, facts),
    }
}

fn item_content(item: &FileItem, facts: &PaneFacts<'_>) -> PaneContent {
    let entry = item.entry();
    let dates = |seconds: Option<u64>| date_text(seconds, facts.condensed_dates);
    // Windows' pane names no size for a folder, and nothing is counted
    // for it.
    let size = if entry.is_dir {
        String::new()
    } else {
        entry.size.map(format::pretty_bytes).unwrap_or_default()
    };
    // A folder lists Type, File location and Date modified, as Windows'
    // pane does; a file adds its Size and Date created.
    let created = if entry.is_dir {
        String::new()
    } else {
        dates(entry.meta.created)
    };
    let mut properties = vec![
        Property::new(keys::TYPE, message_id("Type"), entry.type_label.clone()),
        Property::new(keys::SIZE, message_id("Size"), size),
        Property::new(
            keys::LOCATION,
            message_id("File location"),
            container_text(&entry.uri, facts.locations),
        ),
        Property::new(keys::MODIFIED, message_id("Date modified"), dates(entry.modified)),
        Property::new(keys::CREATED, message_id("Date created"), created),
    ];
    properties.retain(|property| !property.value.is_empty());
    PaneContent {
        preview: Preview::Art(item.art()),
        name: entry.name.clone(),
        small_icon: !entry.is_dir,
        subject: Subject::Selection,
        // Items inside a ZIP have no file of their own to describe.
        has_properties: !ox_core::location::is_archive_location(facts.folder_uri),
        properties,
        media: MediaPreview::of(item),
        metadata: MetadataRequest::of(item),
    }
}

/// "N items selected", their total size, the range of their dates and,
/// when they share one, their folder, as Windows' pane sums a selection.
fn several_content(items: &[FileItem], facts: &PaneFacts<'_>) -> PaneContent {
    let count = ox_core::format::group_digits(items.len() as u64);
    let name = ox_core::i18n::format_message("{selected} items selected", &[("selected", &count)]);
    let mut properties = Vec::new();
    if let Some(size) = total_size_text(items) {
        properties.push(Property::new(keys::SIZE, message_id("Total size"), size));
    }
    let modified = date_range_text(items.iter().filter_map(|item| item.entry().modified));
    if let Some(modified) = modified {
        properties.push(Property::new(
            keys::MODIFIED,
            message_id("Date modified"),
            modified,
        ));
    }
    let first_folder = parent_location(&items[0].entry().uri);
    let one_folder = items
        .iter()
        .all(|item| parent_location(&item.entry().uri) == first_folder);
    if let Some(folder) = first_folder.filter(|_| one_folder) {
        properties.push(Property::new(
            keys::LOCATION,
            message_id("File location"),
            facts.locations.display_location(&folder),
        ));
    }
    PaneContent {
        preview: Preview::Several,
        name,
        small_icon: true,
        subject: Subject::Selection,
        has_properties: !ox_core::location::is_archive_location(facts.folder_uri),
        properties,
        media: None,
        metadata: None,
    }
}

/// Nothing selected: the folder's picture, its name with its item count
/// ("Downloads (2 items)") and Windows' hint box, with no Details rows,
/// Share or Properties, as Windows 11's pane shows a folder.
fn folder_content(facts: &PaneFacts<'_>) -> PaneContent {
    let uri = facts.folder_uri;
    let count = u64::from(facts.folder_item_count);
    let name = ox_core::i18n::format_message(
        &ox_core::i18n::ngettext("{folder} ({count} item)", "{folder} ({count} items)", count),
        &[
            ("folder", &facts.locations.title_for(uri)),
            ("count", &ox_core::format::group_digits(count)),
        ],
    );
    PaneContent {
        preview: Preview::Art(folder_art(facts)),
        name,
        small_icon: false,
        subject: Subject::Folder,
        has_properties: false,
        properties: Vec::new(),
        media: None,
        metadata: None,
    }
}

/// Whether `uri` is the top of a ZIP opened like a folder (ARC-026).
fn is_zip_top(uri: &str) -> bool {
    ArchiveLocation::from_uri(uri).is_some_and(|location| location.parent().is_none())
}

/// The picture of the folder itself: a ZIP's top as a compressed folder,
/// on an SMB share the art its tab shows, else the folder.
fn folder_art(facts: &PaneFacts<'_>) -> Art {
    if is_zip_top(facts.folder_uri) {
        Art::ZipFolder
    } else if is_smb_location(facts.folder_uri) {
        Art::for_smb_location(facts.folder_uri, facts.network)
    } else {
        Art::Folder
    }
}

/// The folder the item at `uri` is in, as its path or display name.
fn container_text(uri: &str, locations: &LocationContext) -> String {
    let container = parent_location(uri).unwrap_or_else(|| uri.to_owned());
    locations.display_location(&container)
}

/// The total size of the files among `items`, as Windows' pane sums a
/// selection: folders are left out and never counted; `None` when no
/// file has a known size.
fn total_size_text(items: &[FileItem]) -> Option<String> {
    let sizes = items
        .iter()
        .map(FileItem::entry)
        .filter(|entry| !entry.is_dir)
        .filter_map(|entry| entry.size);
    let (bytes, known) = sizes.fold((0_u64, false), |(bytes, _), size| {
        (bytes.saturating_add(size), true)
    });
    known.then(|| format::pretty_bytes(bytes))
}

/// A date row: the day and the time without seconds, as the Date
/// modified column writes it; the day only with condensed dates; empty
/// without a date.
fn date_text(seconds: Option<u64>, condensed: bool) -> String {
    if seconds.is_none() {
        String::new()
    } else if condensed {
        format::date_text(seconds)
    } else {
        format::date_short_time_text(seconds)
    }
}

/// The days of the earliest and latest of `dates`, as `first - last`,
/// or the one day when they fall on it: a range shows no time.
fn date_range_text(dates: impl Iterator<Item = u64>) -> Option<String> {
    let (first, last) = dates.fold(None, |range: Option<(u64, u64)>, date| {
        Some(range.map_or((date, date), |(first, last)| (first.min(date), last.max(date))))
    })?;
    let first = format::date_text(Some(first));
    let last = format::date_text(Some(last));
    Some(if first == last {
        first
    } else {
        ox_core::i18n::format_message("{first} - {last}", &[("first", &first), ("last", &last)])
    })
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
            condensed_dates: false,
        })
    }

    fn property<'a>(content: &'a PaneContent, key: &str) -> Option<&'a str> {
        content
            .properties
            .iter()
            .find(|property| property.key == key)
            .map(|property| property.value.as_str())
    }

    fn names(content: &PaneContent) -> Vec<&str> {
        content
            .properties
            .iter()
            .map(|property| property.name.as_str())
            .collect()
    }

    /// One file shows Windows 11's rows, in its order and wording.
    ///
    /// parity: PROP-009
    #[test]
    fn a_file_shows_its_type_size_location_and_dates() {
        let mut entry = file_entry("Notes.txt");
        entry.size = Some(2048);
        entry.modified = Some(1_790_000_000);
        entry.meta.created = Some(1_780_000_000);
        let content = content_for(&[FileItem::new(entry.clone())], "file:///tmp/ox-test");
        assert_eq!(
            names(&content),
            ["Type", "Size", "File location", "Date modified", "Date created"]
        );
        assert_eq!(content.name, "Notes.txt");
        assert_eq!(
            property(&content, keys::SIZE),
            Some(format::pretty_bytes(2048).as_str())
        );
        assert_eq!(
            property(&content, keys::MODIFIED),
            Some(format::date_short_time_text(entry.modified).as_str()),
            "the day and the time, as the Date modified column"
        );
        assert_eq!(property(&content, keys::LOCATION), Some("/tmp/ox-test"));
        assert!(content.subject == Subject::Selection && content.has_properties);
        assert_eq!(content.metadata, None, "a text file has no media details");
    }

    /// A date the backend does not give leaves its row out.
    ///
    /// parity: PROP-009
    #[test]
    fn a_missing_date_leaves_its_row_out() {
        let mut entry = file_entry("Notes.txt");
        entry.meta.created = None;
        let content = content_for(&[FileItem::new(entry)], "file:///tmp/ox-test");
        assert_eq!(property(&content, keys::CREATED), None);
    }

    /// Condensed dates give the day only; otherwise the time follows.
    ///
    /// parity: PROP-010
    #[test]
    fn condensed_dates_drop_the_time() {
        let modified = Some(1_790_000_000);
        assert_eq!(date_text(modified, true), format::date_text(modified));
        assert_eq!(date_text(modified, false), format::date_short_time_text(modified));
        assert!(date_text(modified, false).starts_with(&date_text(modified, true)));
    }

    /// Only a local file has a content preview, chosen by its type; a
    /// picture, video or audio file, local or not, has its details read.
    ///
    /// parity: PROP-011, PROP-009
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
        assert_eq!(
            content.metadata,
            Some(MetadataRequest {
                uri: photo.uri.clone(),
                kind: MediaKind::Picture,
                size: Some(10),
                modified: photo.modified,
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

    /// parity: PROP-009
    #[test]
    fn a_folder_has_no_size_and_no_media_details() {
        let mut entry = folder_entry("Projects");
        entry.modified = Some(1_790_000_000);
        entry.meta.created = Some(1_780_000_000);
        let item = FileItem::new(entry);
        let uri = item.entry().uri.clone();
        let content = content_for(&[item], "file:///tmp/ox-test");
        assert_eq!(
            names(&content),
            ["Type", "File location", "Date modified"],
            "Windows' rows for a folder"
        );
        assert!(
            !content.small_icon,
            "Windows names a folder without its small icon"
        );
        assert_eq!(
            content.media,
            Some(MediaPreview::Folder { uri: uri.clone() }),
            "a local folder's own picture is looked up"
        );
        assert!(content.media.as_ref().is_some_and(|media| media.is_of(&uri)));
        assert_eq!(content.metadata, None);
    }

    /// Nothing selected: the folder's name and item count, then Windows'
    /// hint, with no Details rows and no Properties, as Windows 11's pane
    /// shows "Downloads (2 items)".
    ///
    /// parity: PROP-009
    #[test]
    fn no_selection_counts_the_folders_items() {
        let content = content_for(&[], "file:///home/demo");
        assert_eq!(content.name, "Home (7 items)");
        assert_eq!(content.preview, Preview::Art(Art::Folder));
        assert!(content.subject == Subject::Folder && !content.small_icon);
        assert!(content.properties.is_empty(), "no Details rows");
        assert!(!content.has_properties, "no Properties button");
    }

    /// A ZIP opened like a folder is pictured as a compressed folder at
    /// its top, and its items offer no Properties, as Properties is off
    /// there.
    ///
    /// parity: PROP-009, ARC-026
    #[test]
    fn the_top_of_a_zip_is_a_compressed_folder() {
        let top = ArchiveLocation::root("file:///tmp/ox-test/Bundle.zip");
        let content = content_for(&[], &top.uri());
        assert_eq!(content.preview, Preview::Art(Art::ZipFolder));
        let inside = content_for(&[], &top.member("Docs/").uri());
        assert_eq!(inside.preview, Preview::Art(Art::Folder));
        let member = FileItem::new(file_entry("Read me.txt"));
        let content = content_for(&[member], &top.uri());
        assert!(!content.has_properties, "a ZIP's item has no Properties");
    }

    /// Several items: "N items selected", their total size, their dates
    /// and the folder they share.
    ///
    /// parity: PROP-009
    #[test]
    fn several_items_are_counted_and_summed() {
        let mut first = file_entry("a");
        first.size = Some(1000);
        first.modified = Some(1_790_000_000);
        let mut second = file_entry("b");
        second.size = Some(2000);
        second.modified = Some(1_790_000_000 - 30 * 86_400);
        let selection = [FileItem::new(first.clone()), FileItem::new(second.clone())];
        let content = content_for(&selection, "file:///tmp/ox-test");
        assert_eq!(content.name, "2 items selected");
        assert_eq!(content.preview, Preview::Several);
        assert_eq!(names(&content), ["Total size", "Date modified", "File location"]);
        assert_eq!(
            property(&content, keys::SIZE),
            Some(format::pretty_bytes(3000).as_str())
        );
        let range = format!(
            "{} - {}",
            format::date_text(second.modified),
            format::date_text(first.modified)
        );
        assert_eq!(property(&content, keys::MODIFIED), Some(range.as_str()));

        let with_folder = [FileItem::new(first), FileItem::new(folder_entry("Projects"))];
        let content = content_for(&with_folder, "file:///tmp/ox-test");
        assert_eq!(
            property(&content, keys::SIZE),
            Some(format::pretty_bytes(1000).as_str()),
            "only the files are summed, as in Windows"
        );
        let folders = [
            FileItem::new(folder_entry("Projects")),
            FileItem::new(folder_entry("Drafts")),
        ];
        assert_eq!(
            property(&content_for(&folders, "file:///tmp/ox-test"), keys::SIZE),
            None
        );
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

    /// parity: PROP-009
    #[test]
    fn landing_pages_offer_no_properties() {
        let content = content_for(&[], Page::ThisPc.uri());
        assert!(!content.has_properties);
        assert!(content.properties.is_empty());
    }
}
