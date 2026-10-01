// SPDX-License-Identifier: AGPL-3.0-only
//! What the details pane says about the selection, computed without
//! widgets so it is tested on its own.
//!
//! Ports the text of `renderDetails` in `v2.0.0:desktop/ui/app.js`: one selected
//! item shows its type, size, date and containing folder; no selection or
//! several items describe the folder, with the item count, its location
//! and where it is stored. A folder on an SMB share is pictured as its tab
//! and sidebar row picture it, on the network bar (the owner's icon
//! mapping, 2026-09-28), where app.js drew the plain folder.

use std::path::PathBuf;

use gtk::gio;
use gtk::prelude::FileExt;
use ox_core::format;
use ox_core::location::{is_smb_location, parent_location, LocationContext};
use ox_core::places::NetworkLocation;

use crate::folder_view::item::FileItem;
use crate::icons::Art;
use crate::locations::Page;
use crate::properties::NOT_SCANNED;

/// The note for SMB folders.
const NETWORK_NOTE: &str =
    "Files are accessed through GIO/GVfs. A saved location is not a system-wide drive letter.";
/// The note everywhere else.
const LOCAL_NOTE: &str = "Select an item to see its properties. Double-click to open it.";

/// The picture at the top of the pane.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::window) enum Preview {
    /// The one selected item's art, or the folder's.
    Art(Art),
    /// The copy glyph for several items.
    Several,
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

/// The button under the name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::window) enum PaneAction {
    /// "Open", plus "Pin to Quick access" for a folder.
    Open {
        /// The selected item is a folder that can be pinned.
        can_pin: bool,
    },
    /// "Pin to Quick access" for the current folder.
    PinFolder,
    /// Nothing: a landing page.
    None,
}

/// One row of the Properties grid.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::window) struct Property {
    /// What the row describes, such as "Size".
    pub name: &'static str,
    /// The value as the pane shows it.
    pub value: String,
}

impl Property {
    fn new(name: &'static str, value: String) -> Self {
        Self { name, value }
    }
}

/// Everything the pane shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::window) struct PaneContent {
    /// The picture at the top.
    pub preview: Preview,
    /// The item's or folder's name, or "N items selected".
    pub name: String,
    /// The type line under the name.
    pub kind: String,
    /// The buttons under the name.
    pub action: PaneAction,
    /// The Properties rows, in display order.
    pub properties: Vec<Property>,
    /// The note at the bottom.
    pub note: &'static str,
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
    /// Items the folder holds, hidden ones included.
    pub folder_item_count: u32,
    /// Display names for devices and the home folder.
    pub locations: &'a LocationContext,
    /// The Network list, for the picture of an SMB folder.
    pub network: &'a [NetworkLocation],
    /// Dates show the day only; else the time too (PROP-010).
    pub condensed_dates: bool,
}

/// The picture, name and type line when no single item is selected.
#[derive(Debug)]
struct FolderHeading {
    preview: Preview,
    name: String,
    kind: &'static str,
}

/// What the pane shows for the selection in a folder.
pub(in crate::window) fn pane_content(facts: &PaneFacts<'_>) -> PaneContent {
    match facts.selection {
        [item] => item_content(item, facts),
        _ => folder_content(facts),
    }
}

fn item_content(item: &FileItem, facts: &PaneFacts<'_>) -> PaneContent {
    let locations = facts.locations;
    let entry = item.entry();
    // The folder the item is in, as `displayUri(parentUri(e.uri)||e.uri)`.
    let container = parent_location(&entry.uri).unwrap_or_else(|| entry.uri.clone());
    let properties = vec![
        Property::new("Type", entry.type_label.clone()),
        Property::new("Size", size_text(item)),
        Property::new("Modified", modified_text(entry.modified, facts.condensed_dates)),
        Property::new("Location", locations.display_location(&container)),
    ];
    PaneContent {
        preview: Preview::Art(item.art()),
        name: entry.name.clone(),
        kind: entry.type_label.clone(),
        action: PaneAction::Open {
            can_pin: entry.is_dir,
        },
        properties,
        note: note_for(&entry.uri),
        media: MediaPreview::of(item),
    }
}

fn folder_content(facts: &PaneFacts<'_>) -> PaneContent {
    let uri = facts.folder_uri;
    let heading = folder_heading(facts);
    let selected = facts.selection.len();
    let items = if selected > 0 {
        selected.to_string()
    } else {
        facts.folder_item_count.to_string()
    };
    let action = if Page::from_uri(uri).is_some() {
        PaneAction::None
    } else {
        PaneAction::PinFolder
    };
    PaneContent {
        preview: heading.preview,
        name: heading.name,
        kind: heading.kind.to_owned(),
        action,
        properties: vec![
            Property::new("Items", items),
            Property::new("Location", facts.locations.display_location(uri)),
            Property::new("Storage", storage_of(uri).to_owned()),
        ],
        note: note_for(uri),
        media: None,
    }
}

fn folder_heading(facts: &PaneFacts<'_>) -> FolderHeading {
    let selected = facts.selection.len();
    if selected > 1 {
        return FolderHeading {
            preview: Preview::Several,
            name: format!("{selected} items selected"),
            kind: "Multiple items",
        };
    }
    FolderHeading {
        preview: Preview::Art(folder_art(facts)),
        name: facts.locations.title_for(facts.folder_uri),
        kind: "Folder",
    }
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
        (true, None, _) => NOT_SCANNED.to_owned(),
        (false, _, Some(size)) => format::pretty_bytes(size),
        (false, _, None) => String::new(),
    }
}

/// The Modified row: the day, or the day and time.
fn modified_text(modified: Option<u64>, condensed: bool) -> String {
    if condensed {
        format::date_text(modified)
    } else {
        format::date_time_text(modified)
    }
}

fn note_for(uri: &str) -> &'static str {
    if is_smb_location(uri) {
        NETWORK_NOTE
    } else {
        LOCAL_NOTE
    }
}

/// Where a folder's items are stored, as the Storage row says.
fn storage_of(uri: &str) -> &'static str {
    if is_smb_location(uri) {
        "Network share"
    } else {
        "This computer"
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
            condensed_dates: true,
        })
    }

    fn property<'a>(content: &'a PaneContent, name: &str) -> Option<&'a str> {
        content
            .properties
            .iter()
            .find(|property| property.name == name)
            .map(|property| property.value.as_str())
    }

    /// parity: PROP-009
    #[test]
    fn a_file_shows_its_type_size_date_and_containing_folder() {
        let mut entry = file_entry("Notes.txt");
        entry.size = Some(2048);
        let content = content_for(&[FileItem::new(entry.clone())], "file:///tmp/ox-test");
        let names: Vec<&str> = content.properties.iter().map(|property| property.name).collect();
        assert_eq!(names, ["Type", "Size", "Modified", "Location"]);
        assert_eq!(
            property(&content, "Size"),
            Some(format::pretty_bytes(2048).as_str())
        );
        assert_eq!(
            property(&content, "Modified"),
            Some(format::date_text(entry.modified).as_str())
        );
        assert_eq!(property(&content, "Location"), Some("/tmp/ox-test"));
        assert_eq!(content.action, PaneAction::Open { can_pin: false });
    }

    /// Condensed dates give the day only; otherwise the time follows.
    ///
    /// parity: PROP-010
    #[test]
    fn condensed_dates_drop_the_time() {
        let modified = Some(1_790_000_000);
        assert_eq!(modified_text(modified, true), format::date_text(modified));
        assert_eq!(modified_text(modified, false), format::date_time_text(modified));
        assert!(modified_text(modified, false).starts_with(&modified_text(modified, true)));
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

    /// parity: PROP-009
    #[test]
    fn a_folder_can_be_pinned_and_is_not_scanned() {
        let item = FileItem::new(folder_entry("Projects"));
        let content = content_for(&[item], "file:///tmp/ox-test");
        assert_eq!(content.action, PaneAction::Open { can_pin: true });
        assert_eq!(property(&content, "Size"), Some("Not scanned"));
    }

    /// parity: PROP-009
    #[test]
    fn no_selection_describes_the_folder() {
        let content = content_for(&[], "file:///home/demo");
        assert_eq!((content.name.as_str(), content.kind.as_str()), ("Home", "Folder"));
        assert_eq!(property(&content, "Items"), Some("7"));
        assert_eq!(property(&content, "Location"), Some("/home/demo"));
        assert_eq!(property(&content, "Storage"), Some("This computer"));
        assert_eq!(content.action, PaneAction::PinFolder);
        assert_eq!(content.note, LOCAL_NOTE);
    }

    /// parity: PROP-009
    #[test]
    fn several_items_are_counted_and_shown_as_multiple() {
        let selection = [FileItem::new(file_entry("a")), FileItem::new(file_entry("b"))];
        let content = content_for(&selection, "smb://nas/media");
        assert_eq!(content.name, "2 items selected");
        assert_eq!(content.kind, "Multiple items");
        assert_eq!(content.preview, Preview::Several);
        assert_eq!(property(&content, "Items"), Some("2"));
        assert_eq!(property(&content, "Storage"), Some("Network share"));
        assert_eq!(content.note, NETWORK_NOTE);
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
    fn landing_pages_offer_no_pin() {
        let content = content_for(&[], Page::ThisPc.uri());
        assert_eq!(content.action, PaneAction::None);
    }
}
