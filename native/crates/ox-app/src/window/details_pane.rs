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

/// Width of the pane (`.details` in `desktop/ui/style.css`).
pub(super) const PANE_WIDTH: i32 = 262;

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
    /// The picture at the top.
    pub preview: Preview,
    /// The item's or folder's name, or "N items selected".
    pub name: String,
    /// The type line under the name.
    pub kind: String,
    /// The buttons under the name.
    pub action: PaneAction,
    /// Property names and values, in display order.
    pub properties: Vec<(&'static str, String)>,
    /// The note at the bottom.
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

/// The picture, name and type line when no single item is selected.
fn folder_heading(facts: &PaneFacts<'_>) -> (Preview, String, &'static str) {
    let selected = facts.selection.len();
    if selected > 1 {
        let name = format!("{selected} items selected");
        return (Preview::Several, name, "Multiple items");
    }
    let title = facts.locations.title_for(facts.folder_uri);
    (Preview::Art(ArtKind::Folder), title, "Folder")
}

/// Where a folder's items are stored, as the Storage row says.
fn storage_of(uri: &str) -> &'static str {
    if is_network(uri) {
        "Network share"
    } else {
        "This computer"
    }
}

fn folder_content(facts: &PaneFacts<'_>) -> PaneContent {
    let uri = facts.folder_uri;
    let (preview, name, kind) = folder_heading(facts);
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
        preview,
        name,
        kind: kind.to_owned(),
        action,
        properties: vec![
            ("Items", items),
            ("Location", facts.locations.display_location(uri)),
            ("Storage", storage_of(uri).to_owned()),
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

/// A wrapping label. Its natural width is a few words, so a long name or
/// path wraps inside the pane instead of widening it.
fn pane_label(css_class: &str) -> gtk::Label {
    gtk::Label::builder()
        .xalign(0.0)
        .wrap(true)
        .wrap_mode(gtk::pango::WrapMode::WordChar)
        .max_width_chars(10)
        .css_classes([css_class])
        .build()
}

/// The pane's widgets.
#[derive(Debug)]
pub(super) struct DetailsPane {
    /// The pane, shown beside the folder pane. It scrolls when the window
    /// is too short for it (`.details{overflow:auto}`), so its wrapped
    /// properties never set the window's height.
    pub root: gtk::ScrolledWindow,
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
    /// An empty pane drawing art in `appearance`.
    pub fn new(appearance: Appearance) -> Self {
        let inner = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(0)
            .css_classes(["details-inner"])
            .build();
        // A fixed width: the property values expand within the pane, and
        // without this the pane would take half the window from the list.
        let root = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .width_request(PANE_WIDTH)
            .hexpand(false)
            .child(&inner)
            .css_classes(["details"])
            .build();
        inner.append(&header());
        let preview = icons::art_image(ArtKind::Folder, PREVIEW_SIZE, appearance, 1);
        inner.append(&preview_frame(&preview));
        let name = pane_label("dname");
        name.set_selectable(true);
        inner.append(&name);
        let kind = pane_label("dtype");
        inner.append(&kind);
        let open = pane_button("Open", Glyph::Share, "win.open");
        inner.append(&open);
        let pin_item = pane_button("Pin to Quick access", Glyph::Pin, "win.pin-selected");
        inner.append(&pin_item);
        let pin_folder = pane_button("Pin to Quick access", Glyph::Pin, "win.pin-folder");
        inner.append(&pin_folder);
        let section = pane_label("dsection");
        section.set_text("Properties");
        inner.append(&section);
        let properties = gtk::Grid::builder().row_spacing(15).column_spacing(8).build();
        inner.append(&properties);
        let note = pane_label("note-text");
        inner.append(&note_row(&note));
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

/// The frame that centres the preview art.
fn preview_frame(preview: &gtk::Image) -> gtk::CenterBox {
    let frame = gtk::CenterBox::new();
    frame.add_css_class("preview");
    frame.set_center_widget(Some(preview));
    frame
}

/// The note at the bottom of the pane, with its info glyph.
fn note_row(note: &gtk::Label) -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 5);
    row.add_css_class("note");
    row.append(&icons::glyph(Glyph::Info, 14));
    row.append(note);
    row
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
