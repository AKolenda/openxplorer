// SPDX-License-Identifier: AGPL-3.0-only
//! One sidebar row as a widget: the `.side-entry` button of `renderSidebar`
//! in `desktop/ui/app.js`, styled by `.side-entry` in `style.css`.
//!
//! A row is an expander chevron (This PC, Network), a 20-pixel icon box, the
//! name and, on Quick access rows, the pin. The accent bar of the selected
//! row (`.side-entry.selected:before`) is an overlay at the row's left
//! edge, outside the padding, as the web page positions it.

use gtk::prelude::*;

use crate::icons::{self, Art, ArtImage, Icon};
use crate::window::window_action::WindowAction;

use super::entries::{RowLevel, RowTarget, Section, SectionEdges, SidebarEntry};

/// Glyph icons are 18 pixels (`.side-icon svg`), art 19 (`folderIcon(19)`).
const GLYPH_SIZE: i32 = 18;
const ART_SIZE: i32 = 19;

/// The expander chevron of This PC and Network (`icon('down')` at 9px).
const EXPANDER_SIZE: i32 = 9;

/// The pin of a Quick access row.
const PIN_SIZE: i32 = 11;

/// A row's icon at its size, with the class the skin spaces it by.
fn row_icon(icon: Art) -> ArtImage {
    let (size, class) = match icon {
        Art::Glyph(_) | Art::TintedGlyph(..) => (GLYPH_SIZE, "side-glyph"),
        Art::Folder | Art::ZipFolder | Art::File(_) | Art::Network(_) => (ART_SIZE, "side-art"),
    };
    let image = ArtImage::new(icon, size);
    image.add_css_class(class);
    image
}

fn name_label(text: &str) -> gtk::Label {
    gtk::Label::builder()
        .label(text)
        .xalign(0.0)
        .hexpand(true)
        .ellipsize(gtk::pango::EllipsizeMode::End)
        .css_classes(["name"])
        .build()
}

/// The chevron, icon, name and pin of `entry`.
fn row_content(entry: &SidebarEntry) -> gtk::Box {
    // The gaps are CSS margins on the parts (see `.side-entry` in
    // resources/skin/sidebar.css), so no box spacing.
    let content = gtk::Box::builder().css_classes(["side-entry"]).build();
    if entry.level == RowLevel::Group {
        let expander = icons::image(Icon::ChevronDown16, EXPANDER_SIZE);
        expander.add_css_class("expand");
        content.append(&expander);
    }
    content.append(&row_icon(entry.icon));
    content.append(&name_label(&entry.label));
    if entry.pinned {
        let pin = icons::image(Icon::Pin, PIN_SIZE);
        pin.add_css_class("pin");
        content.append(&pin);
    }
    content
}

/// The accent bar that marks the selected row.
fn selection_bar() -> gtk::Box {
    gtk::Box::builder()
        .halign(gtk::Align::Start)
        .valign(gtk::Align::Start)
        .can_target(false)
        .css_classes(["pill"])
        .build()
}

/// The CSS classes that place a row: a group head or indented inside a
/// group, and in the Quick access box with its first and last rows marked.
fn placement_classes(entry: &SidebarEntry, edges: SectionEdges) -> Vec<&'static str> {
    let mut classes = Vec::new();
    match entry.level {
        RowLevel::Place => {}
        RowLevel::Group => classes.push("group"),
        RowLevel::Child => classes.push("indent"),
    }
    if entry.section == Section::QuickAccess {
        classes.push("quick-access");
        if edges.first {
            classes.push("section-start");
        }
        if edges.last {
            classes.push("section-end");
        }
    }
    classes
}

/// The row for `entry`, which runs `win.go-to` or `win.mount-volume`.
pub(super) fn sidebar_row(entry: &SidebarEntry, edges: SectionEdges) -> gtk::ListBoxRow {
    let overlay = gtk::Overlay::builder().child(&row_content(entry)).build();
    overlay.add_overlay(&selection_bar());
    let row = gtk::ListBoxRow::builder()
        .child(&overlay)
        .tooltip_text(&entry.tooltip)
        .css_classes(placement_classes(entry, edges))
        .build();
    // The visible label names the row; the address is its description.
    row.update_property(&[
        gtk::accessible::Property::Label(&entry.label),
        gtk::accessible::Property::Description(&entry.tooltip),
    ]);
    let (action, target) = match &entry.target {
        RowTarget::Location(uri) => (WindowAction::GoTo, uri),
        RowTarget::MountVolume(id) => (WindowAction::MountVolume, id),
    };
    row.set_action_name(Some(&action.detailed_name()));
    row.set_action_target_value(Some(&target.to_variant()));
    row
}
