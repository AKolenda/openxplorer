// SPDX-License-Identifier: AGPL-3.0-only
//! What the details pane says about the selection, computed without
//! widgets so it is tested on its own.
//!
//! Ports the text of `renderDetails` in `desktop/ui/app.js`: one selected
//! item shows its type, size, date and containing folder; no selection or
//! several items describe the folder, with the item count, its location
//! and where it is stored. A folder on an SMB share is pictured as its tab
//! and sidebar row picture it, on the network bar (the owner's icon
//! mapping, 2026-09-28), where app.js drew the plain folder.

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
        [item] => item_content(item, facts.locations),
        _ => folder_content(facts),
    }
}

fn item_content(item: &FileItem, locations: &LocationContext) -> PaneContent {
    let entry = item.entry();
    // The folder the item is in, as `displayUri(parentUri(e.uri)||e.uri)`.
    let container = parent_location(&entry.uri).unwrap_or_else(|| entry.uri.clone());
    let properties = vec![
        Property::new("Type", entry.type_label.clone()),
        Property::new("Size", size_text(item)),
        Property::new("Modified", format::date_text(entry.modified)),
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
        })
    }

    fn property<'a>(content: &'a PaneContent, name: &str) -> Option<&'a str> {
        content
            .properties
            .iter()
            .find(|property| property.name == name)
            .map(|property| property.value.as_str())
    }

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

    #[test]
    fn a_folder_can_be_pinned_and_is_not_scanned() {
        let item = FileItem::new(folder_entry("Projects"));
        let content = content_for(&[item], "file:///tmp/ox-test");
        assert_eq!(content.action, PaneAction::Open { can_pin: true });
        assert_eq!(property(&content, "Size"), Some("Not scanned"));
    }

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

    #[test]
    fn landing_pages_offer_no_pin() {
        let content = content_for(&[], Page::ThisPc.uri());
        assert_eq!(content.action, PaneAction::None);
    }
}
