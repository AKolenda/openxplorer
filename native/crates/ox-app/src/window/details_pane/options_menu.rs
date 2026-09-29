// SPDX-License-Identifier: AGPL-3.0-only
//! The details pane's context menu (PROP-010), as Dolphin's Information
//! panel offers it: follow the item under the pointer, choose the fields
//! shown, condense dates, and let audio and video start by themselves.
//! Every change is saved at once and redraws the pane.

use gtk::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{gdk, glib};
use ox_core::settings::DetailsPaneOptions;

use super::DetailsPane;

/// The fields the menu can turn off, in the order the pane shows them.
pub(super) const FIELDS: [&str; 8] = [
    "Type",
    "Size",
    "Modified",
    "Location",
    "Dimensions",
    "Length",
    "Items",
    "Storage",
];

/// One choice of the menu: its label and how it reads and changes the
/// options.
struct Choice {
    label: String,
    is_on: fn(&DetailsPaneOptions, &str) -> bool,
    set: fn(&mut DetailsPaneOptions, &str, bool),
    /// The field a field choice is about; empty for the others.
    field: &'static str,
}

impl Choice {
    fn flag(
        label: &str,
        is_on: fn(&DetailsPaneOptions, &str) -> bool,
        set: fn(&mut DetailsPaneOptions, &str, bool),
    ) -> Self {
        Self {
            label: label.to_owned(),
            is_on,
            set,
            field: "",
        }
    }

    fn field(field: &'static str) -> Self {
        Self {
            label: field.to_owned(),
            is_on: |options, field| options.shows(field),
            set: |options, field, shown| {
                options.hidden_fields.retain(|hidden| hidden != field);
                if !shown {
                    options.hidden_fields.push(field.to_owned());
                }
            },
            field,
        }
    }
}

/// Every choice, top to bottom.
fn choices() -> Vec<Choice> {
    let mut choices = vec![Choice::flag(
        "Show the item under the pointer",
        |options, _| options.follow_hover,
        |options, _, on| options.follow_hover = on,
    )];
    choices.extend(FIELDS.into_iter().map(Choice::field));
    choices.push(Choice::flag(
        "Condensed dates",
        |options, _| options.condensed_dates,
        |options, _, on| options.condensed_dates = on,
    ));
    choices.push(Choice::flag(
        "Play audio and video automatically",
        |options, _| options.auto_play,
        |options, _, on| options.auto_play = on,
    ));
    choices
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
    pub(in crate::window) fn show_options_menu(&self, x: f64, y: f64) -> gtk::Popover {
        let column = gtk::Box::new(gtk::Orientation::Vertical, 2);
        let heading = gtk::Label::builder()
            .label("Details pane")
            .xalign(0.0)
            .css_classes(["detail-key"])
            .build();
        column.append(&heading);
        let options = self.imp().options.borrow().clone();
        for choice in choices() {
            let check = gtk::CheckButton::with_label(&choice.label);
            check.set_active((choice.is_on)(&options, choice.field));
            let pane = self.downgrade();
            check.connect_toggled(move |check| {
                let Some(pane) = pane.upgrade() else {
                    return;
                };
                let mut options = pane.imp().options.borrow().clone();
                (choice.set)(&mut options, choice.field, check.is_active());
                pane.change_options(options);
            });
            column.append(&check);
        }
        let popover = gtk::Popover::builder().child(&column).has_arrow(false).build();
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
}
