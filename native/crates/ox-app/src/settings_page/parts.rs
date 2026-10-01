// SPDX-License-Identifier: AGPL-3.0-only
//! The small pieces the category pages are built from: notes, paragraphs
//! and the compact controls of a row.
//!
//! Ports the buttons, checkboxes and help paragraphs of
//! `renderSettingsPage` in `v2.0.0:desktop/ui/app.js` in the look of the settings
//! mockup: a checkbox becomes a switch and a paragraph a short note; a
//! section's summary is a [`super::status_card::StatusCard`].
//! `resources/skin/settings.css` styles the classes named here.

use gtk::prelude::*;

use crate::icons::{self, Icon};
use crate::window::{children, ButtonStyle};

/// The glyph before a note.
const NOTE_GLYPH: i32 = 16;
/// The chevron of a row that opens a page.
const CHEVRON_GLYPH: i32 = 12;
/// The glyph inside a button, before its label.
const BUTTON_GLYPH: i32 = 16;

/// A short muted paragraph after a group, with `glyph` before it, for
/// what a row's one line cannot say.
pub(crate) fn note(glyph: Icon, text: &str) -> gtk::Box {
    let note = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    note.add_css_class("settings-note");
    let image = icons::image(glyph, NOTE_GLYPH);
    image.set_valign(gtk::Align::Start);
    note.append(&image);
    note.append(&wrapped_label(text, "note-text"));
    note
}

/// A label that starts at the left and wraps, with `css_class`.
pub(crate) fn wrapped_label(text: &str, css_class: &str) -> gtk::Label {
    gtk::Label::builder()
        .label(text)
        .xalign(0.0)
        .wrap(true)
        .hexpand(true)
        .css_classes([css_class])
        .build()
}

/// A standalone button labelled `label`, in `style`.
pub(crate) fn button(label: &str, style: ButtonStyle) -> gtk::Button {
    gtk::Button::builder()
        .label(label)
        .valign(gtk::Align::Center)
        .css_classes(["setting-button", style.css_class()])
        .build()
}

/// A bordered button with `glyph` before `label`, such as "Reset".
pub(crate) fn button_with_glyph(label: &str, glyph: Icon) -> gtk::Button {
    let content = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    content.append(&icons::image(glyph, BUTTON_GLYPH));
    content.append(&gtk::Label::new(Some(label)));
    let button = gtk::Button::builder()
        .child(&content)
        .valign(gtk::Align::Center)
        .css_classes(["setting-button", ButtonStyle::Bordered.css_class()])
        .build();
    button.update_property(&[gtk::accessible::Property::Label(label)]);
    button
}

/// A bordered button that opens a page of its own: `label` and a chevron,
/// such as "Manage…".
pub(crate) fn page_button(label: &str) -> gtk::Button {
    let content = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    content.append(&gtk::Label::new(Some(label)));
    content.append(&icons::image(Icon::ChevronRight16, CHEVRON_GLYPH));
    let button = gtk::Button::builder()
        .child(&content)
        .valign(gtk::Align::Center)
        .css_classes(["setting-button", ButtonStyle::Bordered.css_class()])
        .build();
    button.update_property(&[gtk::accessible::Property::Label(label)]);
    button
}

/// A flat chevron at the end of a row that opens a page of its own, as the
/// mockup's `.iconbtn`; screen readers and the tooltip call it `name`.
pub(crate) fn chevron_button(name: &str) -> gtk::Button {
    let button = gtk::Button::builder()
        .child(&icons::image(Icon::ChevronRight16, CHEVRON_GLYPH))
        .tooltip_text(name)
        .valign(gtk::Align::Center)
        .css_classes(["page-chevron"])
        .build();
    button.update_property(&[gtk::accessible::Property::Label(name)]);
    button
}

/// An on/off switch, centred on its row. GTK draws on and off shapes in
/// it with icons from the desktop theme, which the app does not use
/// (`native/README.md`, Icons), so they stay hidden; which side the knob is
/// on shows the state, as in Windows 11.
pub(crate) fn switch() -> gtk::Switch {
    let switch = gtk::Switch::builder().valign(gtk::Align::Center).build();
    for shape in children(&switch).filter(ObjectExt::is::<gtk::Image>) {
        shape.set_visible(false);
    }
    switch
}

/// A muted value at the right of a row, such as the app that opens
/// folders; a value too long for the row ends in "…".
pub(crate) fn value_label(text: &str) -> gtk::Label {
    gtk::Label::builder()
        .label(text)
        .xalign(1.0)
        .ellipsize(gtk::pango::EllipsizeMode::End)
        .css_classes(["setting-value"])
        .build()
}

/// A paragraph in the text colour, such as a step of a guide.
pub(crate) fn paragraph(text: &str) -> gtk::Label {
    wrapped_label(text, "settings-paragraph")
}

/// A bordered menu button with `glyph` before `label`, such as "Open
/// windows…"; its own chevron stays out, as the menu opens under it.
pub(crate) fn menu_button_with_glyph(label: &str, glyph: Icon) -> gtk::MenuButton {
    let content = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    content.append(&icons::image(glyph, BUTTON_GLYPH));
    content.append(&gtk::Label::new(Some(label)));
    let button = gtk::MenuButton::builder()
        .child(&content)
        .valign(gtk::Align::Center)
        .css_classes(["setting-button", ButtonStyle::Bordered.css_class()])
        .build();
    button.update_property(&[gtk::accessible::Property::Label(label)]);
    button
}
