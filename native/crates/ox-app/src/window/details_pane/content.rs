// SPDX-License-Identifier: AGPL-3.0-only
//! What the details pane says about the selection, computed without
//! widgets so it is tested on its own.
//!
//! Laid out as Windows 11 Explorer's Details pane (PROP-009). One selected
//! item shows its large preview, its small icon and name, and under
//! "Details" its Type, Size, File location, Date modified and Date
//! created; a picture, video or audio file adds the rows its own metadata
//! gives once the reader has read them ([`MetadataRequest`]). Several
//! selected items show their pictures stacked, "N items selected" and
//! their total size while every one is a file; nothing selected shows the
//! folder's item count under its picture, which is its tab's: a place's
//! own icon (This PC, Network, the Recycle Bin, Home, a standard folder),
//! and for a folder on an SMB share its picture on the network bar (the
//! owner's icon mapping, 2026-09-28).

use std::path::PathBuf;

use gtk::gio;
use gtk::prelude::FileExt;
use ox_core::format;
use ox_core::location::{is_smb_location, parent_location, ArchiveLocation, LocationContext};
use ox_core::places::{NetworkLocation, Place};

use crate::folder_view::item::FileItem;
use crate::i18n::message_id;
use crate::icons::{Art, Tint};
use crate::locations::Page;
use crate::media_details::MediaKind;
use crate::window::landing::TileFacts;
use crate::window::location_view::tab_icon;

use super::preview_page::{preview_page, PreviewPage, PreviewRules, NO_PREVIEW};

/// The keys of the rows the pane's menu can turn off (PROP-010), which
/// are also the names the settings keep them under.
pub(in crate::window) mod keys {
    pub(in crate::window) const TYPE: &str = "Type";
    pub(in crate::window) const SIZE: &str = "Size";
    pub(in crate::window) const LOCATION: &str = "Location";
    pub(in crate::window) const MODIFIED: &str = "Modified";
    pub(in crate::window) const SPACE_USED: &str = "SpaceUsed";
    pub(in crate::window) const SPACE_FREE: &str = "SpaceFree";
    pub(in crate::window) const TOTAL_SIZE: &str = "TotalSize";
    pub(in crate::window) const FILE_SYSTEM: &str = "FileSystem";
    pub(in crate::window) const DIMENSIONS: &str = "Dimensions";
    pub(in crate::window) const LENGTH: &str = "Length";
}

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

impl Preview {
    /// The art beside the name: the same picture, small, or the first
    /// of the stack.
    pub(in crate::window) fn art(&self) -> Art {
        match self {
            Preview::Art(art) => *art,
            Preview::Stack(arts) => arts.first().copied().unwrap_or(Art::Folder),
        }
    }
}

/// What the preview area shows of one selected file's content
/// (PROP-011).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::window) enum MediaPreview {
    /// A local image, decoded and scaled; `size` is its size in bytes.
    Image { path: PathBuf, size: u64 },
    /// An audio (`is_video` false) or video file, with a player; audio
    /// keeps the item's `art` above its controls. `uri` is the file the
    /// item stands for (a Recent files row's target), on this computer
    /// or an SMB share; its path is found, and with `local_only`
    /// (network previews off) checked to be on no network file system,
    /// off the main thread before the player opens it.
    Recording {
        uri: String,
        local_only: bool,
        is_video: bool,
        art: Art,
    },
    /// Another local document, shown by its cached thumbnail if it has
    /// one, else by the picture `chosen` for its kind in File icons
    /// (LOOK-029), if any.
    Document { uri: String, chosen: Option<String> },
    /// A local folder, shown by its own picture if it has one
    /// ([`super::media`]'s `folder_picture`), else by the picture `chosen`
    /// for folders in File icons, else by its art.
    Folder { uri: String, chosen: Option<String> },
}

impl MediaPreview {
    /// Whether this is the preview of the item at `uri`.
    pub(in crate::window) fn is_of(&self, uri: &str) -> bool {
        match self {
            Self::Image { path, .. } => gio::File::for_path(path).uri() == uri,
            Self::Recording { uri: shown, .. }
            | Self::Document { uri: shown, .. }
            | Self::Folder { uri: shown, .. } => shown == uri,
        }
    }

    /// The preview of `item` under the network `rules`: a local file or
    /// folder has one, and a recording on a share or behind a Recent
    /// files row too ([`Self::recording`]).
    fn of(item: &FileItem, rules: PreviewRules) -> Option<Self> {
        let entry = item.entry();
        if entry.is_virtual {
            return None;
        }
        let content_type = entry.content_type.as_deref().unwrap_or_default();
        if !entry.is_dir && matches!(content_type.split('/').next(), Some("audio" | "video")) {
            return Self::recording(item, rules);
        }
        if entry.is_dir {
            return entry.uri.starts_with("file:").then(|| Self::Folder {
                uri: entry.uri.clone(),
                chosen: crate::icons::file_icons::picture_for(entry),
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
            _ => Self::Document {
                uri: entry.uri.clone(),
                chosen: crate::icons::file_icons::picture_for(entry),
            },
        })
    }
}

impl MediaPreview {
    /// The player of the audio or video `item`, as the Preview pane's
    /// pages treat a file: the file a Recent files row stands for; with
    /// network previews off, none for a remote folder, item or SMB file
    /// (and the player checks the file system itself, `local_only`); and
    /// none for a location without a path of this computer's or an SMB
    /// share's (an archive's member, a phone's file).
    fn recording(item: &FileItem, rules: PreviewRules) -> Option<Self> {
        let entry = item.entry();
        let uri = entry.meta.recent_target.as_deref().unwrap_or(&entry.uri);
        let remote = rules.remote || is_smb_location(uri);
        if remote && !rules.remote_previews {
            return None;
        }
        if gio::File::for_uri(uri).path().is_none() && !is_smb_location(uri) {
            return None;
        }
        let content_type = entry.content_type.as_deref().unwrap_or_default();
        Some(Self::Recording {
            uri: uri.to_owned(),
            local_only: !rules.remote_previews,
            is_video: content_type.starts_with("video/"),
            art: item.art(),
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
            uri: entry.item_uri().to_owned(),
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
    /// The small icon shows beside the name: for a file; Windows shows a
    /// folder's name alone, and several items have their stacked
    /// pictures.
    pub small_icon: bool,
    /// What the pane describes: with nothing selected, Windows' hint box
    /// takes the Details rows' place; a selection may be shared.
    pub subject: Subject,
    /// Windows' hint to select a single file shows in place of the
    /// Details rows: with nothing or several items selected.
    pub hint: bool,
    /// Properties can open for what the pane shows: not on a landing
    /// page with nothing selected.
    pub has_properties: bool,
    /// The location whose Properties the pane's button opens in place of
    /// the selection's: a drive selected on This PC.
    pub properties_of: Option<String>,
    /// The drive whose space the pane reads and adds under Type
    /// ([`drive_rows`]): a drive selected on This PC.
    pub measures_drive: Option<String>,
    /// The Details rows, in display order.
    pub properties: Vec<Property>,
    /// The content preview of one selected file.
    pub media: Option<MediaPreview>,
    /// The file whose own details are read and added to the rows.
    pub metadata: Option<MetadataRequest>,
    /// What the Preview pane shows (VIEW-070).
    pub page: PreviewPage,
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
    /// The standard folders, for the picture of one with nothing
    /// selected.
    pub known_folders: &'a [Place],
    /// Dates show the day only; else the time too (PROP-010).
    pub condensed_dates: bool,
    /// Whether the Preview pane may read the folder's files.
    pub preview_rules: PreviewRules,
}

/// What the pane shows for the selection in a folder.
pub(in crate::window) fn pane_content(facts: &PaneFacts<'_>) -> PaneContent {
    let mut content = match facts.selection {
        [] => folder_content(facts),
        [item] => item_content(item, facts),
        several => several_content(several, facts),
    };
    content.page = preview_page(facts.selection, facts.preview_rules);
    content
}

fn item_content(item: &FileItem, facts: &PaneFacts<'_>) -> PaneContent {
    let entry = item.entry();
    let dates = |seconds: Option<u64>| date_text(seconds, facts.condensed_dates);
    // Windows' pane names no size for a folder, and nothing is counted
    // for it.
    let size = if entry.is_dir {
        String::new()
    } else {
        entry.size.map(format::short_size).unwrap_or_default()
    };
    // A file lists Type, Size, File location and Date modified, and a
    // folder the same without Size, as Windows 11's pane does (checked on
    // Windows 11, 2026-11-22: no Date created). A file in the Recycle Bin
    // is located in the trash folder that holds it, as Windows names
    // C:\$Recycle.Bin\S-1-5-..., and headed by the path it was deleted
    // from.
    let location = entry
        .meta
        .trash_file
        .as_deref()
        .unwrap_or_else(|| entry.item_uri());
    let mut properties = vec![
        Property::new(keys::TYPE, message_id("Type"), entry.type_label.clone()),
        Property::new(keys::SIZE, message_id("Size"), size),
        Property::new(
            keys::LOCATION,
            message_id("File location"),
            container_text(location, facts.locations),
        ),
        Property::new(keys::MODIFIED, message_id("Date modified"), dates(entry.modified)),
    ];
    properties.retain(|property| !property.value.is_empty());
    let name = entry
        .trash_orig_path
        .as_ref()
        .filter(|_| entry.uri.starts_with("trash:"))
        .map_or_else(|| entry.name.clone(), |path| path.display().to_string());
    PaneContent {
        preview: Preview::Art(item.art()),
        name,
        small_icon: !entry.is_dir,
        subject: Subject::Selection,
        hint: false,
        // Items inside a ZIP have no file of their own to describe.
        has_properties: !ox_core::location::is_archive_location(facts.folder_uri),
        properties_of: None,
        measures_drive: None,
        properties,
        media: MediaPreview::of(item, facts.preview_rules),
        metadata: MetadataRequest::of(item),
        page: PreviewPage::Message(NO_PREVIEW),
    }
}

/// "N items selected" with the art of the first few stacked, over
/// Windows' hint to select a single file and no Details rows, as Windows
/// 11's pane shows several items; their size is in the status bar.
fn several_content(items: &[FileItem], facts: &PaneFacts<'_>) -> PaneContent {
    PaneContent {
        preview: Preview::Stack(items.iter().take(STACKED_ITEMS).map(FileItem::art).collect()),
        name: items_selected(items.len()),
        small_icon: false,
        subject: Subject::Selection,
        hint: true,
        has_properties: !ox_core::location::is_archive_location(facts.folder_uri),
        properties_of: None,
        measures_drive: None,
        properties: Vec::new(),
        media: None,
        metadata: None,
        page: PreviewPage::Message(NO_PREVIEW),
    }
}

/// "N items selected", for `count` items.
fn items_selected(count: usize) -> String {
    let count = ox_core::format::group_digits(count as u64);
    ox_core::i18n::format_message("{selected} items selected", &[("selected", &count)])
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
        hint: true,
        has_properties: false,
        properties_of: None,
        measures_drive: None,
        properties: Vec::new(),
        media: None,
        metadata: None,
        page: PreviewPage::Message(NO_PREVIEW),
    }
}

/// The Details pane's `content` for This PC with the tiles `selected`:
/// one tile's picture, name and what it says under its name ("27.6 GB
/// free of 252 GB"), with Properties for a drive that has them (as
/// Alt+Enter opens them), or "N items selected" with their pictures over
/// Windows' hint. Nothing selected keeps This PC's own picture and count.
pub(in crate::window) fn describe_tiles(content: &mut PaneContent, selected: &[TileFacts]) {
    match selected {
        [] => {}
        [tile] => {
            content.preview = Preview::Art(tile.art);
            content.name.clone_from(&tile.name);
            content.subject = Subject::Selection;
            content.hint = false;
            content.has_properties = tile.properties.is_some();
            content.properties_of.clone_from(&tile.properties);
            content.measures_drive.clone_from(&tile.properties);
            content.properties = if tile.properties.is_some() {
                // A drive that can be measured: its Type and space, which
                // the pane reads from it off the main thread
                // ([`drive_rows`]), as Windows 11 lists C: (checked on
                // Windows 11, 2026-11-22).
                Vec::new()
            } else {
                // A drive or share to connect: what its tile says.
                tile.note
                    .iter()
                    .map(|note| Property::new(keys::SIZE, message_id("Space"), note.clone()))
                    .collect()
            };
        }
        several => {
            content.name = items_selected(several.len());
            content.preview =
                Preview::Stack(several.iter().take(STACKED_ITEMS).map(|tile| tile.art).collect());
            content.subject = Subject::Selection;
            content.hint = true;
            content.properties = Vec::new();
            content.measures_drive = None;
        }
    }
}

/// The rows of a drive of type `kind` ("Local Disk") with `free` of
/// `size` bytes free on a file system `filesystem` ("NTFS", "ext4"), as
/// Windows 11's pane lists a drive: Type, Space used as a whole
/// percentage ("37%"), Space free, Total size and File system; Type and
/// File system only when known.
pub(in crate::window) fn drive_rows(
    kind: Option<&str>,
    free: u64,
    size: u64,
    filesystem: Option<&str>,
) -> Vec<Property> {
    let used = size.saturating_sub(free);
    let percent = if size == 0 {
        0
    } else {
        (u128::from(used) * 100 + u128::from(size) / 2) / u128::from(size)
    };
    let mut rows: Vec<Property> = kind
        .map(|kind| Property::new(keys::TYPE, message_id("Type"), kind.to_owned()))
        .into_iter()
        .collect();
    rows.extend([
        Property::new(keys::SPACE_USED, message_id("Space used"), format!("{percent}%")),
        Property::new(
            keys::SPACE_FREE,
            message_id("Space free"),
            format::short_size(free),
        ),
        Property::new(
            keys::TOTAL_SIZE,
            message_id("Total size"),
            format::short_size(size),
        ),
    ]);
    if let Some(filesystem) = filesystem.filter(|filesystem| !filesystem.is_empty()) {
        rows.push(Property::new(
            keys::FILE_SYSTEM,
            message_id("File system"),
            filesystem.to_owned(),
        ));
    }
    rows
}

/// Whether `uri` is the top of a ZIP opened like a folder (ARC-026).
fn is_zip_top(uri: &str) -> bool {
    ArchiveLocation::from_uri(uri).is_some_and(|location| location.parent().is_none())
}

/// The picture of the folder itself, as its tab pictures it: This PC's,
/// Network's or the Recycle Bin's icon, a standard folder's, Home's for
/// the home folder, a ZIP's top as a compressed folder, on an SMB share
/// the art its tab shows, else the folder. Home and Network take the
/// colors of their navigation pane rows, as This PC does, since the pane
/// draws them as large as Windows draws their icons; a folder inside a
/// ZIP is a folder.
fn folder_art(facts: &PaneFacts<'_>) -> Art {
    let uri = facts.folder_uri;
    if ox_core::location::is_archive_location(uri) && !is_zip_top(uri) {
        return Art::Folder;
    }
    match tab_icon(uri, facts.locations, (facts.network, facts.known_folders, &[])) {
        Art::Glyph(icon) if icon == Page::Home.icon() => Art::TintedGlyph(icon, Tint::Home),
        Art::Glyph(icon) if icon == Page::Network.icon() => Art::TintedGlyph(icon, Tint::Network),
        art => art,
    }
}

/// The folder the item at `uri` is in, as its path or display name.
fn container_text(uri: &str, locations: &LocationContext) -> String {
    let container = parent_location(uri).unwrap_or_else(|| uri.to_owned());
    locations.display_location(&container)
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

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
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
            known_folders: &[],
            condensed_dates: false,
            preview_rules: PreviewRules {
                remote: false,
                remote_previews: false,
            },
        })
    }

    /// A folder on this computer, with network previews off.
    const NO_NETWORK_PREVIEWS: PreviewRules = PreviewRules {
        remote: false,
        remote_previews: false,
    };

    fn content_with(selection: &[FileItem], folder_uri: &str, rules: PreviewRules) -> PaneContent {
        let locations = context();
        pane_content(&PaneFacts {
            selection,
            folder_uri,
            folder_item_count: 7,
            locations: &locations,
            network: &[],
            known_folders: &[],
            condensed_dates: false,
            preview_rules: rules,
        })
    }

    /// A recording is played from the file it stands for, under the same
    /// network rule as the other previews: one on a share plays with
    /// network previews on, a Recent files row plays its target, and
    /// nothing remote plays with them off.
    ///
    /// parity: VIEW-070, PROP-011
    #[test]
    fn a_recording_follows_the_files_location_and_the_network_rule() {
        let song = |uri: &str| {
            let mut entry = file_entry("Song.mp3");
            entry.uri = uri.to_owned();
            entry.content_type = Some("audio/mpeg".to_owned());
            entry
        };
        let on = PreviewRules {
            remote: true,
            remote_previews: true,
        };
        let off = PreviewRules {
            remote: true,
            remote_previews: false,
        };
        let share = "smb://studio-nas/projects/Song.mp3";
        let content = content_with(&[FileItem::new(song(share))], "smb://studio-nas/projects", on);
        let media = content.media.expect("a player for the share's song");
        assert!(matches!(media, MediaPreview::Recording { .. }) && media.is_of(share));

        let content = content_with(&[FileItem::new(song(share))], "smb://studio-nas/projects", off);
        assert_eq!(content.media, None, "no player with network previews off");
        let local = "file:///mnt/nas/Song.mp3";
        let content = content_with(&[FileItem::new(song(local))], "file:///mnt/nas", off);
        assert_eq!(content.media, None, "nor for a share's file mounted here");

        let mut recent = song("recent:///5b2d");
        recent.meta.recent_target = Some("file:///tmp/ox-test/Song.mp3".to_owned());
        let content = content_with(
            &[FileItem::new(recent)],
            "recent:///",
            PreviewRules {
                remote: false,
                remote_previews: false,
            },
        );
        let media = content.media.expect("a player for the Recent row's song");
        assert!(media.is_of("file:///tmp/ox-test/Song.mp3"), "{media:?}");
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

    /// A file's size reads as the status bar and the tooltip write it,
    /// with one formatter: 4,486 bytes are "4.38 KB" in all three.
    ///
    /// parity: PROP-009
    #[test]
    fn the_size_reads_as_the_status_bar_and_the_tooltip_write_it() {
        let mut entry = file_entry("Notes.txt");
        entry.size = Some(4_486);
        let one = content_for(&[FileItem::new(entry)], "file:///tmp/ox-test");
        assert_eq!(property(&one, keys::SIZE), Some("4.38\u{a0}KB"));
    }

    /// One file shows Windows 11's rows, in its order and wording: Type,
    /// Size, File location and Date modified, with no Date created
    /// (checked on Windows 11, 2026-11-22).
    ///
    /// parity: PROP-009
    #[test]
    fn a_file_shows_its_type_size_location_and_date_modified() {
        let mut entry = file_entry("Notes.txt");
        entry.size = Some(2048);
        entry.modified = Some(1_790_000_000);
        entry.meta.created = Some(1_780_000_000);
        let content = content_for(&[FileItem::new(entry.clone())], "file:///tmp/ox-test");
        assert_eq!(
            names(&content),
            ["Type", "Size", "File location", "Date modified"]
        );
        assert_eq!(content.name, "Notes.txt");
        assert_eq!(
            property(&content, keys::SIZE),
            Some(format::short_size(2048).as_str())
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
        entry.modified = None;
        let content = content_for(&[FileItem::new(entry)], "file:///tmp/ox-test");
        assert_eq!(property(&content, keys::MODIFIED), None);
    }

    /// A file in the Recycle Bin is headed by the full path it was
    /// deleted from, and located in the trash folder that holds it, with
    /// no Original location or Date deleted rows, as Windows 11's pane
    /// shows a recycled file (C:\$Recycle.Bin\S-1-5-..., checked on
    /// Windows 11, 2026-11-22).
    ///
    /// parity: PROP-009
    #[test]
    fn a_recycled_file_is_headed_by_where_it_was_and_located_in_the_trash() {
        let mut entry = file_entry("f2.txt");
        entry.uri = "trash:///f2.txt".to_owned();
        entry.size = Some(3);
        entry.modified = Some(1_790_000_000);
        entry.meta.created = Some(1_780_000_000);
        entry.trash_orig_path = Some(PathBuf::from("/home/demo/OXT2/f2.txt"));
        entry.trash_deletion_date = Some(1_795_000_000);
        entry.meta.trash_file = Some("file:///home/demo/.local/share/Trash/files/f2.txt".to_owned());
        let content = content_for(&[FileItem::new(entry)], "trash:///");
        assert_eq!(content.name, "/home/demo/OXT2/f2.txt");
        assert_eq!(
            names(&content),
            ["Type", "Size", "File location", "Date modified"]
        );
        assert_eq!(
            property(&content, keys::LOCATION),
            Some(
                context()
                    .display_location("file:///home/demo/.local/share/Trash/files")
                    .as_str()
            )
        );
        assert!(content.has_properties);
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
            MediaPreview::of(&FileItem::new(photo.clone()), NO_NETWORK_PREVIEWS),
            Some(MediaPreview::Recording { is_video: true, .. })
        ));
        photo.uri = "smb://nas/media/Beach.png".to_owned();
        assert_eq!(
            MediaPreview::of(&FileItem::new(photo), NO_NETWORK_PREVIEWS),
            None,
            "shares are not read for a preview with network previews off"
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
            Some(MediaPreview::Folder {
                uri: uri.clone(),
                chosen: None
            }),
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
        assert_eq!(
            content.preview,
            Preview::Art(Art::TintedGlyph(Page::Home.icon(), Tint::Home)),
            "the home folder pictured as Home"
        );
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

    /// Several items: their pictures stacked, "N items selected" and
    /// Windows' hint to select a single file, with no Details rows, as
    /// Windows 11's pane shows them; the status bar gives their size.
    ///
    /// parity: PROP-009
    #[test]
    fn several_items_are_counted_stacked_and_hinted() {
        let mut first = file_entry("a");
        first.size = Some(1000);
        let mut second = file_entry("b");
        second.size = Some(2000);
        let selection = [FileItem::new(first.clone()), FileItem::new(second)];
        let content = content_for(&selection, "file:///tmp/ox-test");
        assert_eq!(content.name, "2 items selected");
        assert_eq!(
            content.preview,
            Preview::Stack(selection.iter().map(FileItem::art).collect())
        );
        assert!(!content.small_icon, "the stack stands for them");
        assert!(content.hint, "Windows' hint to select a single file");
        assert!(
            content.properties.is_empty(),
            "no Details rows: {:?}",
            names(&content)
        );
        assert!(
            content.subject == Subject::Selection && content.has_properties,
            "Share and Properties stay for the selection"
        );

        let four: Vec<FileItem> = ["a", "b", "c", "d"]
            .into_iter()
            .map(|name| FileItem::new(file_entry(name)))
            .collect();
        let Preview::Stack(arts) = content_for(&four, "file:///tmp/ox-test").preview else {
            panic!("a stack");
        };
        assert_eq!(arts.len(), STACKED_ITEMS);
        let one = content_for(&four[..1], "file:///tmp/ox-test");
        assert!(!one.hint, "one file has its rows");
    }

    /// One drive selected on This PC offers Properties, which Alt+Enter
    /// opens for it, as Windows' pane does; several drives have Windows'
    /// hint.
    ///
    /// parity: PROP-009
    #[test]
    fn a_drive_selected_on_this_pc_offers_properties() {
        let tile = |name: &str| TileFacts {
            art: Art::Folder,
            name: name.to_owned(),
            note: Some("23.0 GB free of 251 GB".to_owned()),
            properties: Some("file:///".to_owned()),
        };
        let mut content = content_for(&[], Page::ThisPc.uri());
        describe_tiles(&mut content, &[tile("Local Disk")]);
        assert!(content.has_properties, "Properties for the drive");
        assert_eq!(content.properties_of.as_deref(), Some("file:///"));
        assert!(!content.hint);
        assert_eq!(content.name, "Local Disk");
        assert!(
            content.properties.is_empty(),
            "its rows are read from the drive: {:?}",
            names(&content)
        );
        assert_eq!(content.measures_drive.as_deref(), Some("file:///"));
        let mut content = content_for(&[], Page::ThisPc.uri());
        describe_tiles(&mut content, &[tile("Local Disk"), tile("Backup")]);
        assert_eq!(content.name, "2 items selected");
        assert!(content.hint && content.properties.is_empty());
    }

    /// A drive's rows, as Windows 11's pane lists them: Type, Space used
    /// as a percentage, Space free, Total size and File system
    /// (checked on Windows 11, 2026-11-22: "37%", "39.7 GB", "62.8 GB",
    /// "NTFS").
    ///
    /// parity: PROP-009
    #[test]
    fn a_drives_space_rows_read_as_windows_lists_them() {
        // 39.7 GB free of 62.8 GB.
        let rows = drive_rows(Some("Local Disk"), 42_628_000_000, 67_432_000_000, Some("NTFS"));
        let shown: Vec<(&str, &str)> = rows
            .iter()
            .map(|row| (row.name.as_str(), row.value.as_str()))
            .collect();
        assert_eq!(
            shown,
            [
                ("Type", "Local Disk"),
                ("Space used", "37%"),
                ("Space free", "39.7\u{a0}GB"),
                ("Total size", "62.8\u{a0}GB"),
                ("File system", "NTFS"),
            ]
        );
        assert_eq!(
            drive_rows(None, 0, 10, None).len(),
            3,
            "no Type or File system unknown"
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
        let local = content_for(&[], "file:///home/demo/Projects");
        assert_eq!(local.preview, Preview::Art(Art::Folder));
    }

    /// With nothing selected, a place is pictured as its tab and its
    /// navigation pane row picture it, as Windows 11's pane shows the
    /// Recycle Bin's, Network's or the Downloads folder's own icon: never
    /// a plain folder for the Recycle Bin, Network, Home or a standard
    /// folder.
    ///
    /// parity: PROP-009
    #[test]
    fn a_place_is_pictured_as_its_tab_is() {
        use ox_core::places::KnownFolder;
        let downloads = Place {
            label: String::new(),
            uri: "file:///home/demo/Downloads".to_owned(),
            known_folder: Some(KnownFolder::Downloads),
            is_shared: false,
        };
        let locations = context();
        let picture = |uri: &str| {
            pane_content(&PaneFacts {
                selection: &[],
                folder_uri: uri,
                folder_item_count: 2,
                locations: &locations,
                network: &[],
                known_folders: std::slice::from_ref(&downloads),
                condensed_dates: false,
                preview_rules: NO_NETWORK_PREVIEWS,
            })
            .preview
        };
        assert_eq!(
            picture("trash:///"),
            Preview::Art(Art::Glyph(crate::icons::Icon::Delete))
        );
        assert_eq!(
            picture(Page::Network.uri()),
            Preview::Art(Art::TintedGlyph(Page::Network.icon(), Tint::Network))
        );
        assert_eq!(
            picture("file:///home/demo"),
            Preview::Art(Art::TintedGlyph(Page::Home.icon(), Tint::Home))
        );
        assert_eq!(
            picture("file:///home/demo/Downloads"),
            Preview::Art(Art::for_known_folder(KnownFolder::Downloads))
        );
        assert_eq!(
            picture("recent:///"),
            Preview::Art(Art::Glyph(crate::icons::Icon::History))
        );
        assert_eq!(picture("file:///home/demo/Projects"), Preview::Art(Art::Folder));
    }

    /// parity: PROP-009
    #[test]
    fn landing_pages_offer_no_properties() {
        let content = content_for(&[], Page::ThisPc.uri());
        assert!(!content.has_properties);
        assert!(content.properties.is_empty());
    }
}
