// SPDX-License-Identifier: AGPL-3.0-only
//! One sidebar row as a widget: the `.side-entry` button of `renderSidebar`
//! in `v2.0.0:desktop/ui/app.js`, styled by `.side-entry` in `style.css`.
//!
//! A row is an expander chevron (This PC, Network), a 20-pixel icon box, the
//! name (with a capacity bar under a drive's) and, on Quick access rows, the
//! pin. The accent bar of the selected
//! row (`.side-entry.selected:before`) is an overlay at the row's left
//! edge, outside the padding, as the web page positions it.

use gtk::prelude::*;

use crate::icons::{self, Art, ArtImage, Icon};
use crate::window::landing;
use crate::window::place_menus::{removal_action, PlaceMenu};
use crate::window::widget_tree::descendants;
use crate::window::window_action::WindowAction;

use super::super::saved_search::saved_search_target;
use super::entries::{EjectButton, RowLevel, RowTarget, Section, SectionEdges, SidebarEntry};

/// Glyph icons are 18 pixels (`.side-icon svg`), art 19 (`folderIcon(19)`).
const GLYPH_SIZE: i32 = 18;
const ART_SIZE: i32 = 19;

/// The expander chevron of This PC and Network (`icon('down')` at 9px).
const EXPANDER_SIZE: i32 = 9;

/// The class of the chevron of This PC and Network.
const CHEVRON_CLASS: &str = "side-expander";

/// The pin of a Quick access row.
const PIN_SIZE: i32 = 11;

/// The eject glyph of a removable drive's row.
const EJECT_SIZE: i32 = 14;

/// The class of the dashed drop tail of an empty Quick access.
const PIN_DROP_TAIL_CLASS: &str = "quick-drop-tail";

/// A row's icon at its size, with the class the skin spaces it by: the
/// automatic size for `chosen_size` 0, else the size chosen in pixels
/// (SIDE-012).
fn row_icon(icon: Art, chosen_size: u32) -> ArtImage {
    let (size, class) = match icon {
        Art::Glyph(_) | Art::TintedGlyph(..) => (GLYPH_SIZE, "side-glyph"),
        Art::Folder | Art::ZipFolder | Art::File(_) | Art::Network(_) => (ART_SIZE, "side-art"),
    };
    let size = i32::try_from(chosen_size)
        .ok()
        .filter(|chosen| *chosen > 0)
        .unwrap_or(size);
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

/// The name of `entry`, with a thin capacity bar under it for a mounted
/// drive, as Dolphin's Places panel shows (SIDE-018); the bar turns red
/// when the drive is nearly full and its tooltip says how much is free.
fn name_and_capacity(entry: &SidebarEntry) -> gtk::Widget {
    let name = name_label(&entry.label);
    let Some(PlaceMenu::Drive { uri, .. }) = &entry.menu else {
        return name.upcast();
    };
    let texts = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .valign(gtk::Align::Center)
        .hexpand(true)
        .build();
    texts.append(&name);
    landing::show_capacity(&texts, uri, false);
    texts.upcast()
}

/// The chevron, icon, name and pin of `entry`, its icon `icon_size`
/// pixels or automatic for 0, and the chevron as `collapsed` says.
fn row_content(entry: &SidebarEntry, icon_size: u32, collapsed: bool) -> gtk::Box {
    // The gaps are CSS margins on the parts (see `.side-entry` in
    // resources/skin/sidebar.css), so no box spacing.
    let content = gtk::Box::builder().css_classes(["side-entry"]).build();
    if entry.level == RowLevel::Group {
        if let Some(chevron) = section_chevron(entry.section, !collapsed) {
            content.append(&chevron);
        }
    }
    let icon = row_icon(entry.icon, icon_size);
    if matches!(entry.icon, Art::Network(_)) {
        // The network pipe says what it means (`.side-icon.shared`).
        icon.set_tooltip_text(Some(&ox_core::i18n::gettext("Network share")));
    }
    content.append(&icon);
    content.append(&name_and_capacity(entry));
    if entry.pinned {
        let pin = icons::image(Icon::Pin, PIN_SIZE);
        pin.add_css_class("pin");
        content.append(&pin);
    }
    if let Some(eject) = &entry.eject {
        content.append(&eject_button(eject));
    }
    content
}

/// The eject button of a removable drive: runs Eject, or Disconnect for a
/// drive that can only be unmounted.
fn eject_button(eject: &EjectButton) -> gtk::Button {
    let button = gtk::Button::builder()
        .child(&icons::image(Icon::ArrowEject, EJECT_SIZE))
        .tooltip_text(eject.label)
        .valign(gtk::Align::Center)
        .css_classes(["side-eject"])
        .build();
    button.update_property(&[gtk::accessible::Property::Label(eject.label)]);
    removal_action(eject.removal).assign_with_target_to(&button, &eject.uri.to_variant());
    button
}

/// The chevron of This PC or Network, `expanded` or collapsed: a button
/// of its own that collapses and expands the section, as in Windows
/// Explorer's navigation pane, with its own highlight; clicking the name
/// still opens the place (SIDE-033). It takes no focus: Left and Right on
/// the row collapse and expand the section from the keyboard
/// (`collapsing.rs`).
fn section_chevron(section: Section, expanded: bool) -> Option<gtk::Button> {
    let (key, _) = section.hiding()?;
    let button = gtk::Button::builder()
        .css_classes([CHEVRON_CLASS])
        .valign(gtk::Align::Center)
        .focus_on_click(false)
        .can_focus(false)
        .build();
    WindowAction::ToggleSidebarSection.assign_with_target_to(&button, &key.to_variant());
    draw_chevron(&button, section, expanded);
    Some(button)
}

/// Points `chevron` down while `section` is expanded and right while it
/// is collapsed, and names what a click on it does.
fn draw_chevron(chevron: &gtk::Button, section: Section, expanded: bool) {
    let name = section.hiding().map_or("", |(_, name)| name);
    let (glyph, label) = if expanded {
        (
            Icon::ChevronDown16,
            ox_core::i18n::format_message("Collapse {section}", &[("section", name)]),
        )
    } else {
        (
            Icon::ChevronRight16,
            ox_core::i18n::format_message("Expand {section}", &[("section", name)]),
        )
    };
    chevron.set_child(Some(&icons::image(glyph, EXPANDER_SIZE)));
    chevron.set_tooltip_text(Some(&label));
    chevron.update_property(&[gtk::accessible::Property::Label(&label)]);
}

/// Shows the head `row` of `section` expanded or collapsed: its chevron,
/// and the state screen readers announce.
pub(super) fn show_expanded(row: &gtk::ListBoxRow, section: Section, expanded: bool) {
    let chevron = descendants::<gtk::Button>(row)
        .into_iter()
        .find(|button| button.has_css_class(CHEVRON_CLASS));
    if let Some(chevron) = chevron {
        draw_chevron(&chevron, section, expanded);
    }
    row.update_state(&[gtk::accessible::State::Expanded(Some(expanded))]);
}

/// Whether `widget` is a row's chevron or inside one.
pub(super) fn is_on_chevron(widget: &gtk::Widget) -> bool {
    std::iter::successors(Some(widget.clone()), WidgetExt::parent)
        .take_while(|widget| !widget.is::<gtk::ListBoxRow>())
        .any(|widget| widget.has_css_class(CHEVRON_CLASS))
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
    if matches!(entry.section, Section::QuickAccess | Section::SavedSearches) {
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

/// The row for `entry`, which runs `win.go-to` or `win.mount-volume`,
/// drawn for its section `collapsed` or expanded: a head shows it, and
/// the rows inside a collapsed section are hidden (SIDE-033).
pub(super) fn sidebar_row(
    entry: &SidebarEntry,
    edges: SectionEdges,
    icon_size: u32,
    collapsed: bool,
) -> gtk::ListBoxRow {
    let overlay = gtk::Overlay::builder()
        .child(&row_content(entry, icon_size, collapsed))
        .build();
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
    match entry.level {
        RowLevel::Place => {}
        RowLevel::Group => row.update_state(&[gtk::accessible::State::Expanded(Some(!collapsed))]),
        RowLevel::Child => row.set_visible(!collapsed),
    }
    let (action, target) = match &entry.target {
        RowTarget::Location(uri) => (WindowAction::GoTo, uri.to_variant()),
        RowTarget::MountVolume(id) => (WindowAction::MountVolume, id.to_variant()),
        RowTarget::SavedSearch(search) => (WindowAction::OpenSavedSearch, saved_search_target(search)),
        RowTarget::PinDropTail => {
            row.set_activatable(false);
            row.set_selectable(false);
            row.add_css_class(PIN_DROP_TAIL_CLASS);
            return row;
        }
    };
    row.set_action_name(Some(&action.detailed_name()));
    row.set_action_target_value(Some(&target));
    row
}
