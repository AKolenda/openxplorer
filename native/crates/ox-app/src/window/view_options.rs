// SPDX-License-Identifier: AGPL-3.0-only
//! The folder views' options in a window: previews (VIEW-057, VIEW-058)
//! and item counts (VIEW-037) for the folder shown, and the details
//! columns the user chose from the titles' menu (VIEW-033) and dragged
//! into order (VIEW-034).
//!
//! Previews and item counts follow the folder: a network folder gets
//! neither unless Settings allow previews there, as Dolphin skips remote
//! files. Every window follows a change made in Settings or another
//! window, as all windows share the preferences.

use gtk::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{gdk, glib};
use ox_core::location::{location_kind, LocationKind};

use super::actions::text_action;
use super::menu_popover::{ItemCheck, MenuEntry, MenuItem, MenuPopover};
use super::window_action::WindowAction;
use super::BrowserWindow;
use crate::folder_view::sorting::SortColumn;
use crate::icons::Icon;
use crate::thumbnails::PreviewPolicy;

/// The glyph of each column in the titles' menu.
fn column_glyph(column: SortColumn) -> Icon {
    match column {
        SortColumn::Modified | SortColumn::Created => Icon::Clock,
        SortColumn::Size => Icon::HardDrive,
        SortColumn::Owner => Icon::Organization,
        SortColumn::Permissions => Icon::ShieldLock,
        _ => Icon::Document,
    }
}

/// The titles' menu: every column that can be shown or hidden, checked
/// while it shows. Name cannot be hidden, so it is not offered.
fn column_menu_entries(chosen: &[SortColumn]) -> Vec<MenuEntry> {
    let item = |column: SortColumn| {
        let item = MenuItem::with_text_target(
            column.label(),
            column_glyph(column),
            WindowAction::DetailsColumn,
            column.as_str(),
        );
        MenuEntry::Item(MenuItem {
            check: ItemCheck::Fixed(chosen.contains(&column)),
            ..item
        })
    };
    SortColumn::CHOOSABLE.into_iter().map(item).collect()
}

/// `chosen` with `column` shown at the end, or hidden.
fn toggled(mut chosen: Vec<SortColumn>, column: SortColumn) -> Vec<SortColumn> {
    if chosen.contains(&column) {
        chosen.retain(|shown| *shown != column);
    } else {
        chosen.push(column);
    }
    chosen
}

impl BrowserWindow {
    /// Adds `win.details-column`, opens the titles' menu on a secondary
    /// click and saves the columns' order after a drag.
    pub(super) fn install_view_option_actions(&self) {
        let toggle = text_action(WindowAction::DetailsColumn, |window, key| {
            if let Some(column) = SortColumn::from_key(key) {
                window.toggle_details_column(column);
            }
        });
        self.add_action_entries([toggle]);
        for pane in self.folder_panes() {
            let details = pane.details();
            details.connect_columns_chosen(glib::clone!(
                #[weak(rename_to = window)]
                self,
                #[weak]
                pane,
                move |_| window.remember_pane_style(&pane)
            ));
            let Some(header) = details.header() else { continue };
            let click = gtk::GestureClick::new();
            click.set_button(gdk::BUTTON_SECONDARY);
            click.connect_pressed(glib::clone!(
                #[weak(rename_to = window)]
                self,
                #[weak]
                header,
                move |gesture, _, x, y| {
                    gesture.set_state(gtk::EventSequenceState::Claimed);
                    if let Some(side) = window.side_holding(&header) {
                        window.activate_pane(side);
                    }
                    window.show_column_menu(&header, x, y);
                }
            ));
            header.add_controller(click);
        }
    }

    /// The titles' menu at (`x`, `y`) in `header`.
    pub(super) fn show_column_menu(&self, header: &gtk::Widget, x: f64, y: f64) -> MenuPopover {
        let chosen = self.folder_pane().details().chosen_columns();
        let popover = MenuPopover::new(column_menu_entries(&chosen));
        popover.set_parent(header);
        #[expect(clippy::cast_possible_truncation, reason = "a pointer position fits in i32")]
        popover.set_pointing_to(Some(&gdk::Rectangle::new(x as i32, y as i32, 1, 1)));
        popover.connect_closed(|popover| {
            let popover = popover.clone();
            glib::idle_add_local_once(move || popover.unparent());
        });
        popover.popup();
        popover
    }

    /// Shows or hides `column` and saves the choice.
    fn toggle_details_column(&self, column: SortColumn) {
        let details = self.folder_pane().details();
        let chosen = toggled(details.chosen_columns(), column);
        details.show_chosen_columns(chosen.clone());
        self.remember_style();
    }

    /// Applies the shared view options to the folder shown: which items
    /// show previews, whether folders count their items, and the details
    /// columns.
    pub(super) fn apply_view_options(&self) {
        for (side, uri) in self.shown_panes() {
            let mut options = self.context().settings_data().preferences.view_options;
            let pane = self.pane_on(side);
            options.show_previews = pane.previews_enabled();
            let is_remote = self.is_remote_folder(&uri);
            pane.owners()
                .set_previews(PreviewPolicy::for_folder(&options, is_remote));
            pane.owners()
                .set_counts_items(options.count_folder_items && !is_remote);
        }
    }

    /// Whether `uri` is reached over a network or a device link: a
    /// server, a share mounted by the kernel, a phone or camera.
    fn is_remote_folder(&self, uri: &str) -> bool {
        match location_kind(uri) {
            LocationKind::Local => self.imp().locations.borrow().is_network_location(uri),
            LocationKind::Smb | LocationKind::Remote | LocationKind::Device => true,
            LocationKind::Other => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use ox_core::settings::ViewOptions;

    use super::*;
    use crate::folder_view::details::chosen_from_keys;

    /// The titles' menu offers every column but Name, checked while it
    /// shows; choosing one adds it at the end or hides it.
    ///
    /// parity: VIEW-033
    #[test]
    fn the_titles_menu_shows_and_hides_columns() {
        let chosen = chosen_from_keys(&ViewOptions::default().details_columns);
        let entries = column_menu_entries(&chosen);
        let checked: Vec<(String, bool)> = entries
            .iter()
            .filter_map(|entry| match entry {
                MenuEntry::Item(item) => Some((item.label.clone(), item.check == ItemCheck::Fixed(true))),
                MenuEntry::Divider => None,
            })
            .collect();
        assert_eq!(checked.len(), SortColumn::CHOOSABLE.len());
        assert!(!checked.iter().any(|(label, _)| label == "Name"));
        assert!(checked.contains(&("Type".to_owned(), true)));
        assert!(checked.contains(&("Owner".to_owned(), false)));

        let with_owner = toggled(chosen.clone(), SortColumn::Owner);
        assert_eq!(with_owner.last(), Some(&SortColumn::Owner));
        assert!(!toggled(chosen, SortColumn::Type).contains(&SortColumn::Type));
    }
}
