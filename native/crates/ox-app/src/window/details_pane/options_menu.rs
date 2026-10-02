// SPDX-License-Identifier: AGPL-3.0-only
//! The details pane's context menu (PROP-010), as Dolphin's Information
//! panel offers it: follow the item under the pointer, choose the fields
//! shown, condense dates, and let audio and video start by themselves.
//! It is the app's usual menu with check marks; each choice runs
//! `win.details-pane-option` with its name, is saved at once and redraws
//! the pane.

use gtk::prelude::*;
use gtk::{gdk, glib};
use ox_core::settings::DetailsPaneOptions;

use super::DetailsPane;
use crate::icons::Icon;
use crate::window::menu_popover::{ItemCheck, MenuEntry, MenuItem, MenuPopover};
use crate::window::window_action::WindowAction;

/// The fields the menu can turn off, in the order the pane shows them,
/// with their glyphs.
pub(super) const FIELDS: [(&str, Icon); 8] = [
    ("Type", Icon::Document),
    ("Size", Icon::HardDrive),
    ("Modified", Icon::Clock),
    ("Location", Icon::Folder),
    ("Dimensions", Icon::Image),
    ("Length", Icon::Video),
    ("Items", Icon::TextBulletList),
    ("Storage", Icon::HardDrive),
];

/// The option names of the choices that are not fields.
const FOLLOW_HOVER: &str = "follow-hover";
const CONDENSED_DATES: &str = "condensed-dates";
const AUTO_PLAY: &str = "auto-play";

/// `options` with the choice `name` (an option name or a field) switched.
fn toggled(mut options: DetailsPaneOptions, name: &str) -> DetailsPaneOptions {
    match name {
        FOLLOW_HOVER => options.follow_hover = !options.follow_hover,
        CONDENSED_DATES => options.condensed_dates = !options.condensed_dates,
        AUTO_PLAY => options.auto_play = !options.auto_play,
        field => {
            let shown = options.shows(field);
            options.hidden_fields.retain(|hidden| hidden != field);
            if shown {
                options.hidden_fields.push(field.to_owned());
            }
        }
    }
    options
}

/// A checkable item for the choice `name`.
fn choice(label: &str, glyph: Icon, name: &str, is_on: bool) -> MenuEntry {
    let item = MenuItem::with_text_target(label, glyph, WindowAction::DetailsPaneOption, name);
    MenuEntry::Item(MenuItem {
        check: ItemCheck::Fixed(is_on),
        ..item
    })
}

/// The menu's entries for `options`, top to bottom.
fn entries(options: &DetailsPaneOptions) -> Vec<MenuEntry> {
    let mut entries = vec![
        choice(
            ox_core::i18n::gettext_static("Show the item under the pointer"),
            Icon::Eye,
            FOLLOW_HOVER,
            options.follow_hover,
        ),
        MenuEntry::Divider,
    ];
    for (field, glyph) in FIELDS {
        entries.push(choice(field, glyph, field, options.shows(field)));
    }
    entries.push(MenuEntry::Divider);
    entries.push(choice(
        ox_core::i18n::gettext_static("Condensed dates"),
        Icon::Clock,
        CONDENSED_DATES,
        options.condensed_dates,
    ));
    entries.push(choice(
        ox_core::i18n::gettext_static("Play audio and video automatically"),
        Icon::MusicNote,
        AUTO_PLAY,
        options.auto_play,
    ));
    entries
}

impl DetailsPane {
    /// Opens the menu on a secondary click anywhere in the pane.
    pub(super) fn attach_options_menu(&self) {
        let click = gtk::GestureClick::new();
        click.set_button(gdk::BUTTON_SECONDARY);
        click.connect_pressed(glib::clone!(
            #[weak(rename_to = pane)]
            self,
            move |_, _, x, y| {
                pane.show_options_menu(x, y);
            }
        ));
        self.add_controller(click);
    }

    /// The menu as a popover at (`x`, `y`).
    pub(in crate::window) fn show_options_menu(&self, x: f64, y: f64) -> MenuPopover {
        let popover = MenuPopover::new(entries(&self.options()));
        popover.set_parent(self);
        #[expect(clippy::cast_possible_truncation, reason = "a pointer position fits in i32")]
        popover.set_pointing_to(Some(&gdk::Rectangle::new(x as i32, y as i32, 1, 1)));
        popover.connect_closed(|popover| {
            let popover = popover.clone();
            glib::idle_add_local_once(move || popover.unparent());
        });
        popover.popup();
        popover
    }

    /// Switches the menu's choice `name`, as `win.details-pane-option`
    /// asks.
    pub(in crate::window) fn toggle_option(&self, name: &str) {
        self.change_options(toggled(self.options(), name));
    }
}
