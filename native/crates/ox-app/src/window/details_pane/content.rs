// SPDX-License-Identifier: AGPL-3.0-only
//! What the details pane says about the selection, computed without
//! widgets so it is tested on its own.
//!
//! Ports the text of `renderDetails` in `desktop/ui/app.js`: one selected
//! item shows its type, size, date and containing folder; no selection or
//! several items describe the folder, with the item count, its location
//! and where it is stored.

use ox_core::entry::Entry;
use ox_core::format;
use ox_core::location::{parent_location, LocationContext};

use crate::folder_view::item::FileItem;
use crate::icons::ArtKind;
use crate::locations::Page;

/// The note for SMB folders.
const NETWORK_NOTE: &str =
    "Files are accessed through GIO/GVfs. A saved location is not a system-wide drive letter.";
/// The note everywhere else.
const LOCAL_NOTE: &str = "Select an item to see its properties. Double-click to open it.";

/// The picture at the top of the pane.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::window) enum Preview {
    /// The one selected item's art.
    Art(ArtKind),
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
        Property::new("Size", size_text(entry)),
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
        preview: Preview::Art(ArtKind::Folder),
        name: facts.locations.title_for(facts.folder_uri),
        kind: "Folder",
    }
}

/// The size line: folder sizes are not scanned yet, as in the Python app
/// before a scan.
fn size_text(entry: &Entry) -> String {
    match (entry.is_dir, entry.size) {
        (true, _) => "Not scanned".to_owned(),
        (false, Some(size)) => format::pretty_bytes(size),
        (false, None) => String::new(),
    }
}

fn is_network(uri: &str) -> bool {
    uri.starts_with("smb:")
}

fn note_for(uri: &str) -> &'static str {
    if is_network(uri) {
        NETWORK_NOTE
    } else {
        LOCAL_NOTE
    }
}

/// Where a folder's items are stored, as the Storage row says.
fn storage_of(uri: &str) -> &'static str {
    if is_network(uri) {
        "Network share"
    } else {
        "This computer"
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::test_support::{file_entry, folder_entry};

    fn context() -> LocationContext {
        LocationContext {
            home: Some(PathBuf::from("/home/demo")),
            ..LocationContext::default()
        }
    }

    fn content_for(selection: &[FileItem], folder_uri: &str) -> PaneContent {
        let locations = context();
        pane_content(&PaneFacts {
            selection,
            folder_uri,
            folder_item_count: 7,
            locations: &locations,
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

    #[test]
    fn landing_pages_offer_no_pin() {
        let content = content_for(&[], Page::ThisPc.uri());
        assert_eq!(content.action, PaneAction::None);
    }
}
