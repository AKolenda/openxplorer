// SPDX-License-Identifier: AGPL-3.0-only
//! The details pane: the selection's properties.
//!
//! Ports `renderDetails` in `desktop/ui/app.js`, laid out as §4.8 of
//! `native/docs/ui-spec.md`: a header with a close button, a preview, the
//! name and type, an Open or "Pin to Quick access" button, a Properties
//! grid and a note. What the pane says is computed by [`pane_content`]
//! without widgets, so it is tested on its own.

use gtk::prelude::*;
use ox_core::entry::Entry;
use ox_core::format;
use ox_core::location::{parent_location, LocationContext};

use crate::folder_view::item::FileItem;
use crate::icons::{self, ArtKind, Glyph};
use crate::locations::Page;
use crate::theme::Appearance;

/// Preview size in the pane, as `fileIcon(e, 84)` in app.js.
const PREVIEW_SIZE: i32 = 84;

/// The note for SMB folders.
const NETWORK_NOTE: &str =
    "Files are accessed through GIO/GVfs. A saved location is not a system-wide drive letter.";
/// The note everywhere else.
const LOCAL_NOTE: &str = "Select an item to see its properties. Double-click to open it.";

/// The picture at the top of the pane.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Preview {
    /// The one selected item's art.
    Art(ArtKind),
    /// The copy glyph for several items.
    Several,
}

/// The button under the name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum PaneAction {
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

/// Everything the pane shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct PaneContent {
    pub preview: Preview,
    pub name: String,
    pub kind: String,
    pub action: PaneAction,
    pub properties: Vec<(&'static str, String)>,
    pub note: &'static str,
}

/// What [`pane_content`] reads.
#[derive(Debug, Clone, Copy)]
pub(super) struct PaneFacts<'a> {
    /// The selected items.
    pub selection: &'a [FileItem],
    /// The folder or page the tab shows.
    pub folder_uri: &'a str,
    /// Items the folder holds, hidden ones included.
    pub folder_item_count: u32,
    /// Display names for devices and the home folder.
    pub locations: &'a LocationContext,
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

fn item_content(item: &FileItem, locations: &LocationContext) -> PaneContent {
    let entry = item.entry();
    // The folder the item is in, as `displayUri(parentUri(e.uri)||e.uri)`.
    let container = parent_location(&entry.uri).unwrap_or_else(|| entry.uri.clone());
    let properties = vec![
        ("Type", entry.type_label.clone()),
        ("Size", size_text(entry)),
        ("Modified", format::date_text(entry.modified)),
        ("Location", locations.display_location(&container)),
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

fn note_for(uri: &str) -> &'static str {
    if is_network(uri) {
        NETWORK_NOTE
    } else {
        LOCAL_NOTE
    }
}

fn folder_content(facts: &PaneFacts<'_>) -> PaneContent {
    let uri = facts.folder_uri;
    let selected = facts.selection.len();
    let (preview, name, kind) = if selected > 1 {
        (
            Preview::Several,
            format!("{selected} items selected"),
            "Multiple items",
        )
    } else {
        (
            Preview::Art(ArtKind::Folder),
            facts.locations.title_for(uri),
            "Folder",
        )
    };
    let items = if selected > 0 {
        selected.to_string()
    } else {
        facts.folder_item_count.to_string()
    };
    let storage = if is_network(uri) {
        "Network share"
    } else {
        "This computer"
    };
    let action = if Page::from_uri(uri).is_some() {
        PaneAction::None
    } else {
        PaneAction::PinFolder
    };
    PaneContent {
        preview,
        name,
        kind: kind.to_owned(),
        action,
        properties: vec![
            ("Items", items),
            ("Location", facts.locations.display_location(uri)),
            ("Storage", storage.to_owned()),
        ],
        note: note_for(uri),
    }
}

/// What the pane shows for the selection in a folder.
pub(super) fn pane_content(facts: &PaneFacts<'_>) -> PaneContent {
    match facts.selection {
        [item] => item_content(item, facts.locations),
        _ => folder_content(facts),
    }
}

/// A full-width pane button.
fn pane_button(label: &str, glyph: Glyph, action: &str) -> gtk::Button {
    let child = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    child.set_halign(gtk::Align::Center);
    child.append(&icons::glyph(glyph, 14));
    child.append(&gtk::Label::new(Some(label)));
    gtk::Button::builder()
        .child(&child)
        .action_name(action)
        .css_classes(["dbutton"])
        .build()
}

fn pane_label(css_class: &str) -> gtk::Label {
    gtk::Label::builder()
        .xalign(0.0)
        .wrap(true)
        .wrap_mode(gtk::pango::WrapMode::WordChar)
        .css_classes([css_class])
        .build()
}

/// The pane's widgets.
#[derive(Debug)]
pub(super) struct DetailsPane {
    /// The pane, shown beside the folder pane.
    pub root: gtk::Box,
    preview: gtk::Image,
    name: gtk::Label,
    kind: gtk::Label,
    open: gtk::Button,
    pin_item: gtk::Button,
    pin_folder: gtk::Button,
    properties: gtk::Grid,
    note: gtk::Label,
}

impl DetailsPane {
    pub fn new(appearance: Appearance) -> Self {
        let root = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(0)
            .width_request(262)
            .css_classes(["details", "details-inner"])
            .build();
        root.append(&header());
        let preview = icons::art_image(ArtKind::Folder, PREVIEW_SIZE, appearance, 1);
        let frame = gtk::CenterBox::new();
        frame.add_css_class("preview");
        frame.set_center_widget(Some(&preview));
        root.append(&frame);
        let name = pane_label("dname");
        name.set_selectable(true);
        let kind = pane_label("dtype");
        let open = pane_button("Open", Glyph::Share, "win.open");
        let pin_item = pane_button("Pin to Quick access", Glyph::Pin, "win.pin-selected");
        let pin_folder = pane_button("Pin to Quick access", Glyph::Pin, "win.pin-folder");
        let section = pane_label("dsection");
        section.set_text("Properties");
        let properties = gtk::Grid::builder().row_spacing(15).column_spacing(8).build();
        let note = pane_label("note-text");
        let note_row = gtk::Box::new(gtk::Orientation::Horizontal, 5);
        note_row.add_css_class("note");
        note_row.append(&icons::glyph(Glyph::Info, 14));
        note_row.append(&note);
        for widget in [
            name.upcast_ref::<gtk::Widget>(),
            kind.upcast_ref(),
            open.upcast_ref(),
            pin_item.upcast_ref(),
            pin_folder.upcast_ref(),
            section.upcast_ref(),
            properties.upcast_ref(),
            note_row.upcast_ref(),
        ] {
            root.append(widget);
        }
        Self {
            root,
            preview,
            name,
            kind,
            open,
            pin_item,
            pin_folder,
            properties,
            note,
        }
    }

    /// Shows `content`, drawing art in `appearance` at `scale`.
    pub fn show(&self, content: &PaneContent, appearance: Appearance, scale: i32) {
        match content.preview {
            Preview::Art(kind) => icons::set_art(&self.preview, kind, PREVIEW_SIZE, appearance, scale),
            Preview::Several => icons::set_glyph(&self.preview, Glyph::Copy, 80),
        }
        self.name.set_text(&content.name);
        self.kind.set_text(&content.kind);
        let (open, pin_item, pin_folder) = match content.action {
            PaneAction::Open { can_pin } => (true, can_pin, false),
            PaneAction::PinFolder => (false, false, true),
            PaneAction::None => (false, false, false),
        };
        self.open.set_visible(open);
        self.pin_item.set_visible(pin_item);
        self.pin_folder.set_visible(pin_folder);
        self.show_properties(&content.properties);
        self.note.set_text(content.note);
    }

    fn show_properties(&self, properties: &[(&'static str, String)]) {
        while let Some(child) = self.properties.first_child() {
            self.properties.remove(&child);
        }
        for (row, (key, value)) in (0..).zip(properties) {
            let key_label = pane_label("dkey");
            key_label.set_text(key);
            key_label.set_width_request(73);
            key_label.set_yalign(0.0);
            let value_label = pane_label("dval");
            value_label.set_text(value);
            value_label.set_selectable(true);
            value_label.set_hexpand(true);
            self.properties.attach(&key_label, 0, row, 1, 1);
            self.properties.attach(&value_label, 1, row, 1, 1);
        }
    }

    /// The property rows shown, for tests.
    #[cfg(test)]
    pub fn shown_properties(&self) -> Vec<(String, String)> {
        let text = |column: i32, row: i32| {
            self.properties
                .child_at(column, row)
                .and_downcast::<gtk::Label>()
                .map(|label| label.text().to_string())
        };
        (0..)
            .map_while(|row| Some((text(0, row)?, text(1, row)?)))
            .collect()
    }
}

/// "Details" with a close button bound to the pane's toggle action.
fn header() -> gtk::Box {
    let header = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    header.add_css_class("detail-header");
    let title = gtk::Label::builder()
        .label("Details")
        .xalign(0.0)
        .hexpand(true)
        .build();
    let close = gtk::Button::builder()
        .child(&icons::glyph(Glyph::Close, 12))
        .tooltip_text("Close details pane")
        .action_name("win.details-pane")
        .css_classes(["x"])
        .build();
    header.append(&title);
    header.append(&close);
    header
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

    fn property<'a>(content: &'a PaneContent, key: &str) -> Option<&'a str> {
        content
            .properties
            .iter()
            .find(|(name, _)| *name == key)
            .map(|(_, value)| value.as_str())
    }

    #[test]
    fn a_file_shows_its_type_size_date_and_containing_folder() {
        let mut entry = file_entry("Notes.txt");
        entry.size = Some(2048);
        let content = content_for(&[FileItem::new(entry.clone())], "file:///tmp/ox-test");
        let keys: Vec<&str> = content.properties.iter().map(|(key, _)| *key).collect();
        assert_eq!(keys, ["Type", "Size", "Modified", "Location"]);
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
