// SPDX-License-Identifier: AGPL-3.0-only
//! The small pieces the category pages are built from: status cards,
//! notes, and the compact controls of a row.
//!
//! Ports the buttons, checkboxes and help paragraphs of
//! `renderSettingsPage` in `desktop/ui/app.js` in the look of the settings
//! mockup: a checkbox becomes a switch, a paragraph a short note, and a
//! section's summary a tinted status card. `resources/skin/settings.css`
//! styles the classes named here.

use gtk::prelude::*;

use super::row::PageWidth;
use crate::icons::{self, Icon};
use crate::window::{children, ButtonStyle};

/// The glyph in a status card's round badge.
const STATUS_GLYPH: i32 = 22;
/// The glyph before a note.
const NOTE_GLYPH: i32 = 16;
/// The chevron of a row that opens a page.
const CHEVRON_GLYPH: i32 = 12;
/// The glyph inside a button, before its label.
const BUTTON_GLYPH: i32 = 16;

/// The class of a status card.
const STATUS_CARD_CLASS: &str = "status-card";

/// What a status card says.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct StatusText<'a> {
    /// The glyph in the round badge.
    pub glyph: Icon,
    /// The one-line state, such as "Default file explorer".
    pub title: &'a str,
    /// The line under it.
    pub text: &'a str,
    /// The milestone that brings the card's actions, when the native
    /// preview lacks them.
    pub notice: Option<&'a str>,
}

/// A tinted card that sums up a category, with its main actions at the
/// right, as the mockup's `.hero`.
pub(crate) fn status_card(status: StatusText<'_>, actions: &[gtk::Widget]) -> gtk::Box {
    let card = gtk::Box::new(gtk::Orientation::Horizontal, 16);
    card.add_css_class(STATUS_CARD_CLASS);
    let badge = gtk::Box::builder()
        .valign(gtk::Align::Center)
        .halign(gtk::Align::Start)
        .css_classes(["status-badge"])
        .build();
    badge.append(&icons::image(status.glyph, STATUS_GLYPH));
    card.append(&badge);
    let texts = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .hexpand(true)
        .valign(gtk::Align::Center)
        .build();
    texts.append(&wrapped_label(status.title, "status-title"));
    texts.append(&wrapped_label(status.text, "status-text"));
    if let Some(notice) = status.notice {
        texts.append(&wrapped_label(notice, "setting-notice"));
    }
    card.append(&texts);
    let buttons = gtk::Box::builder()
        .spacing(8)
        .valign(gtk::Align::Center)
        .halign(gtk::Align::Start)
        .build();
    for action in actions {
        buttons.append(action);
    }
    card.append(&buttons);
    card
}

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

/// Lays `extra` out for a page `width` wide when it is a status card: its
/// parts side by side, or stacked in a narrow window.
pub(crate) fn fit_status_card(extra: &gtk::Widget, width: PageWidth) {
    let card = extra.downcast_ref::<gtk::Box>();
    let Some(card) = card.filter(|card| card.has_css_class(STATUS_CARD_CLASS)) else {
        return;
    };
    let orientation = match width {
        PageWidth::Roomy => gtk::Orientation::Horizontal,
        PageWidth::Narrow => gtk::Orientation::Vertical,
    };
    card.set_orientation(orientation);
}
