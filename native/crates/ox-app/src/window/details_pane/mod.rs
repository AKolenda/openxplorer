// SPDX-License-Identifier: AGPL-3.0-only
//! The details pane: the selection's properties.
//!
//! Ports `renderDetails` in `desktop/ui/app.js`, laid out as §4.8 of
//! `native/docs/ui-spec.md`: a header with a close button, a preview, the
//! name and type, an Open or "Pin to Quick access" button, a Properties
//! grid and a note. What the pane says is computed by [`pane_content`]
//! ([`content`]); this module draws it.

mod content;

use gtk::prelude::*;

use crate::icons::{self, ArtKind, Glyph};
use crate::theme::Appearance;

use super::appearance::ArtStyle;
use super::button_style::ButtonStyle;
use super::window_action::WindowAction;

pub(super) use content::{pane_content, PaneFacts};
use content::{PaneAction, PaneContent, Preview, Property};

/// Width of the pane (`.details` in `desktop/ui/style.css`).
pub(super) const PANE_WIDTH: i32 = 262;

/// Preview size in the pane: app.js draws `fileIcon(e, 84)` and
/// `.detail-preview svg` shows it at 83 pixels.
const PREVIEW_SIZE: i32 = 83;

/// The copy glyph that stands for several selected items.
const SEVERAL_ITEMS_GLYPH: i32 = 80;

/// The glyphs in the pane's buttons and note.
const SMALL_GLYPH: i32 = 14;

// The Properties grid follows `.detail-props{grid-template-columns:73px
// minmax(0,1fr);gap:15px 8px}` in `desktop/ui/style.css`.
/// The width of the Properties names column.
const PROPERTY_NAME_WIDTH: i32 = 73;
/// Pixels between Properties rows.
const PROPERTY_ROW_GAP: i32 = 15;
/// Pixels between a property's name and its value.
const PROPERTY_COLUMN_GAP: i32 = 8;
/// The Properties column of the names.
const NAME_COLUMN: i32 = 0;
/// The Properties column of the values.
const VALUE_COLUMN: i32 = 1;

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
    /// An empty pane; its placeholder art is drawn in `appearance` at the
    /// default scale until the first [`Self::show`].
    pub fn new(appearance: Appearance) -> Self {
        let inner = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .css_classes(["details-inner"])
            .build();
        let root = pane_scroller(&inner);
        inner.append(&header());
        let preview = icons::art_image(ArtKind::Folder, PREVIEW_SIZE, appearance, 1);
        inner.append(&preview_frame(&preview));
        let name = pane_label("dname");
        name.set_selectable(true);
        inner.append(&name);
        let kind = pane_label("dtype");
        inner.append(&kind);
        let open = pane_button("Open", Glyph::Share, WindowAction::Open);
        inner.append(&open);
        let pin_item = pane_button("Pin to Quick access", Glyph::Pin, WindowAction::PinSelected);
        inner.append(&pin_item);
        let pin_folder = pane_button("Pin to Quick access", Glyph::Pin, WindowAction::PinFolder);
        inner.append(&pin_folder);
        inner.append(&section_title("Properties"));
        let properties = gtk::Grid::builder()
            .row_spacing(PROPERTY_ROW_GAP)
            .column_spacing(PROPERTY_COLUMN_GAP)
            .build();
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

    /// Makes the pane `width` pixels wide ([`PANE_WIDTH`], or less in a
    /// narrower window).
    pub fn set_width(&self, width: i32) {
        self.root.set_width_request(width);
    }

    /// Shows `content`, drawing its art in `style`.
    pub fn show(&self, content: &PaneContent, style: ArtStyle) {
        match content.preview {
            Preview::Art(kind) => style.draw_into(&self.preview, kind, PREVIEW_SIZE),
            Preview::Several => icons::set_glyph(&self.preview, Glyph::Copy, SEVERAL_ITEMS_GLYPH),
        }
        self.name.set_text(&content.name);
        self.kind.set_text(&content.kind);
        self.show_buttons(content.action);
        self.show_properties(&content.properties);
        self.note.set_text(content.note);
    }

    /// Shows the buttons `action` offers and hides the others.
    fn show_buttons(&self, action: PaneAction) {
        let offers_open = matches!(action, PaneAction::Open { .. });
        let offers_item_pin = action == PaneAction::Open { can_pin: true };
        let offers_folder_pin = action == PaneAction::PinFolder;
        self.open.set_visible(offers_open);
        self.pin_item.set_visible(offers_item_pin);
        self.pin_folder.set_visible(offers_folder_pin);
    }

    fn show_properties(&self, properties: &[Property]) {
        while let Some(child) = self.properties.first_child() {
            self.properties.remove(&child);
        }
        for (row, property) in (0..).zip(properties) {
            self.properties
                .attach(&property_name(property.name), NAME_COLUMN, row, 1, 1);
            self.properties
                .attach(&property_value(&property.value), VALUE_COLUMN, row, 1, 1);
        }
    }

    /// The property rows shown, top to bottom, for tests.
    #[cfg(test)]
    pub fn shown_properties(&self) -> Vec<ShownProperty> {
        let mut shown = Vec::new();
        for row in 0.. {
            let name = self.property_text(NAME_COLUMN, row);
            let value = self.property_text(VALUE_COLUMN, row);
            let (Some(name), Some(value)) = (name, value) else {
                break;
            };
            shown.push(ShownProperty { name, value });
        }
        shown
    }

    /// The text in `column` of Properties row `row`, for tests.
    #[cfg(test)]
    fn property_text(&self, column: i32, row: i32) -> Option<String> {
        let label = self.properties.child_at(column, row);
        let label = label.and_downcast::<gtk::Label>()?;
        Some(label.text().to_string())
    }
}

/// A Properties row as the pane shows it, for tests.
#[cfg(test)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShownProperty {
    /// The name on the left, such as "Items".
    pub name: String,
    /// The value on the right.
    pub value: String,
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

/// A section title, such as "Properties".
fn section_title(text: &str) -> gtk::Label {
    let title = pane_label("dsection");
    title.set_text(text);
    title
}

/// A property's name in the left column.
fn property_name(name: &str) -> gtk::Label {
    let label = pane_label("dkey");
    label.set_text(name);
    label.set_width_request(PROPERTY_NAME_WIDTH);
    label.set_yalign(0.0);
    label
}

/// A property's value, which can be selected and copied.
fn property_value(value: &str) -> gtk::Label {
    let label = pane_label("dval");
    label.set_text(value);
    label.set_selectable(true);
    label.set_hexpand(true);
    label
}

/// A full-width pane button.
fn pane_button(label: &str, glyph: Glyph, action: WindowAction) -> gtk::Button {
    let child = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    child.set_halign(gtk::Align::Center);
    child.append(&icons::glyph(glyph, SMALL_GLYPH));
    child.append(&gtk::Label::new(Some(label)));
    gtk::Button::builder()
        .child(&child)
        .action_name(action.detailed_name())
        .css_classes(["dbutton", ButtonStyle::Bordered.css_class()])
        .build()
}

/// The pane around `inner`, scrolling when the window is short. A fixed
/// width: the property values expand within the pane, and without it the
/// pane would take half the window from the list.
fn pane_scroller(inner: &gtk::Box) -> gtk::ScrolledWindow {
    gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .width_request(PANE_WIDTH)
        .hexpand(false)
        .child(inner)
        .css_classes(["details"])
        .build()
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
    row.append(&icons::glyph(Glyph::Info, SMALL_GLYPH));
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
        .action_name(WindowAction::DetailsPane.detailed_name())
        .css_classes(["x"])
        .build();
    header.append(&title);
    header.append(&close);
    header
}
