// SPDX-License-Identifier: AGPL-3.0-only
//! The card the window's notices pop up at the top right, under the
//! command bar: the update notice (UPD-001) and the GitHub star prompt
//! (UPD-023). A glyph, a title over a line of text, an accent button and
//! a bordered one, and a close button at the top right.

use gtk::prelude::*;

use crate::icons::{self, Icon};

/// The glyph beside the title.
const NOTICE_GLYPH: i32 = 20;

/// The glyph of the close button.
const CLOSE_GLYPH: i32 = 16;

/// The widest the text gets before it wraps, in characters.
const MAX_WIDTH_CHARS: i32 = 34;

/// The parts of a built card its notice fills in and listens to.
pub(super) struct NoticeCard {
    /// The card itself, a child of the notice.
    pub(super) surface: gtk::Box,
    /// The line under the title.
    pub(super) text: gtk::Label,
    /// The accent button.
    pub(super) primary: gtk::Button,
    /// The bordered button.
    pub(super) secondary: gtk::Button,
    /// The close button.
    pub(super) close: gtk::Button,
}

/// Builds the card inside `notice`, which it hides and lays out at the top
/// right, named `title` for screen readers.
pub(super) fn build(notice: &impl IsA<gtk::Widget>, glyph: Icon, title: &str) -> NoticeCard {
    let notice = notice.upcast_ref::<gtk::Widget>();
    notice.set_halign(gtk::Align::End);
    notice.set_valign(gtk::Align::Start);
    notice.set_visible(false);
    notice.update_property(&[gtk::accessible::Property::Label(title)]);

    let glyph = icons::image(glyph, NOTICE_GLYPH);
    glyph.set_valign(gtk::Align::Start);
    glyph.add_css_class("notice-card-glyph");

    let title = gtk::Label::builder()
        .label(title)
        .xalign(0.0)
        .css_classes(["notice-card-title"])
        .build();
    let text = gtk::Label::builder()
        .xalign(0.0)
        .wrap(true)
        .max_width_chars(MAX_WIDTH_CHARS)
        .accessible_role(gtk::AccessibleRole::Status)
        .css_classes(["notice-card-text"])
        .build();
    let primary = gtk::Button::builder().css_classes(["accent"]).build();
    let secondary = gtk::Button::builder().css_classes(["bordered"]).build();
    let buttons = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    buttons.add_css_class("notice-card-buttons");
    buttons.append(&primary);
    buttons.append(&secondary);

    let column = gtk::Box::new(gtk::Orientation::Vertical, 4);
    column.set_hexpand(true);
    column.append(&title);
    column.append(&text);
    column.append(&buttons);

    let dismiss = ox_core::i18n::gettext("Dismiss");
    let close = gtk::Button::builder()
        .child(&icons::image(Icon::Dismiss, CLOSE_GLYPH))
        .valign(gtk::Align::Start)
        .tooltip_text(&dismiss)
        .css_classes(["notice-card-close"])
        .build();
    close.update_property(&[gtk::accessible::Property::Label(&dismiss)]);

    let surface = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(14)
        .css_classes(["notice-card"])
        .build();
    surface.append(&glyph);
    surface.append(&column);
    surface.append(&close);
    surface.set_parent(notice);
    NoticeCard {
        surface,
        text,
        primary,
        secondary,
        close,
    }
}
