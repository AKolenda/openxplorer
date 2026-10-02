// SPDX-License-Identifier: AGPL-3.0-only
//! Hiding places and sections of the sidebar, and showing them again
//! (SIDE-010).
//!
//! Dolphin's Places panel hides one place with "Hide" and a whole group
//! with "Hide Section", and "Show All Entries" lists what is hidden,
//! dimmed, so it can be shown again. Here "Unpin from Quick access" hides
//! a standard folder (SIDE-009), "Hide" hides any other place (a drive, a
//! network location, Recent files, the Recycle Bin), saved for every
//! window as `hiddenSidebarPlaces`, every row's menu ends with "Hide
//! section" for its group, saved as `hiddenSidebarSections`, and the
//! empty-space menu's "Show all entries" (this window only, available
//! while anything is hidden) lists the hidden rows dimmed, whose menus
//! offer "Show" and "Show section".

use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;
use ox_core::location::same_location;
use ox_core::settings::SettingsError;

use crate::settings_store::Change;

use super::actions::{text_action, toggle_action};
use super::sidebar::entries::{place_entry, RowTarget, Section, SidebarEntry};
use super::sidebar::HiddenRow;
use super::window_action::WindowAction;
use super::BrowserWindow;

impl BrowserWindow {
    /// Adds `win.sidebar-show-all`, `win.hide-section`,
    /// `win.show-section`, `win.hide-place` and `win.show-place`.
    pub(super) fn install_sidebar_hiding(&self) {
        self.add_action_entries([
            toggle_action(WindowAction::SidebarShowAll, false, |window, on| {
                window.imp().sidebar_show_all.set(on);
                window.render_places();
            }),
            text_action(WindowAction::HideSection, |window, key| {
                window.change_hidden_sections(key, true);
            }),
            text_action(WindowAction::ShowSection, |window, key| {
                window.change_hidden_sections(key, false);
            }),
            text_action(WindowAction::HidePlace, BrowserWindow::hide_place),
            text_action(WindowAction::ShowPlace, BrowserWindow::show_hidden_place),
        ]);
    }

    /// `entries` as the sidebar shows them: rows of hidden sections left
    /// out, or, while "Show all entries" is on, kept and joined by the
    /// hidden standard folders, each marked; and whether anything is
    /// hidden.
    pub(super) fn shown_sidebar_rows(
        &self,
        entries: Vec<SidebarEntry>,
    ) -> (Vec<(SidebarEntry, HiddenRow)>, bool) {
        let settings = self.context().settings_data();
        let hidden_sections = &settings.preferences.hidden_sidebar_sections;
        let hidden_places = &settings.preferences.hidden_sidebar_places;
        let show_all = self.imp().sidebar_show_all.get();
        let is_hidden = |section: Section| {
            section
                .hiding()
                .is_some_and(|(key, _)| hidden_sections.iter().any(|hidden| hidden == key))
        };
        let mut rows: Vec<(SidebarEntry, HiddenRow)> = entries
            .into_iter()
            .filter_map(|entry| {
                let state = if is_hidden(entry.section) {
                    HiddenRow::Section
                } else if is_hidden_place(&entry, hidden_places) {
                    HiddenRow::Place
                } else {
                    HiddenRow::Shown
                };
                (state == HiddenRow::Shown || show_all).then_some((entry, state))
            })
            .collect();
        let hidden_folders: Vec<_> = self
            .context()
            .known_folders()
            .into_iter()
            .filter(|place| {
                settings
                    .hidden_quick
                    .iter()
                    .any(|uri| same_location(uri, &place.uri))
            })
            .collect();
        if show_all {
            let after_quick_access = rows
                .iter()
                .rposition(|(entry, _)| entry.section == Section::QuickAccess)
                .map_or(1.min(rows.len()), |last| last + 1);
            let locations = self.imp().locations.borrow();
            for (offset, place) in hidden_folders.iter().enumerate() {
                let row = (place_entry(place, &locations), HiddenRow::Place);
                rows.insert(after_quick_access + offset, row);
            }
        }
        let anything_hidden =
            !hidden_sections.is_empty() || !hidden_places.is_empty() || !hidden_folders.is_empty();
        (rows, anything_hidden)
    }

    /// Hides or shows the sidebar section saved as `key`, for every window.
    fn change_hidden_sections(&self, key: &str, hide: bool) {
        let key = key.to_owned();
        let change: Change = Box::new(move |settings| settings.set_section_hidden(&key, hide).map(|_| ()));
        self.save_sidebar_change(change);
    }

    /// Hides the place `uri` from the sidebar, for every window.
    fn hide_place(&self, uri: &str) {
        let uri = uri.to_owned();
        let change: Change = Box::new(move |settings| settings.set_place_hidden(&uri, true).map(|_| ()));
        self.save_sidebar_change(change);
    }

    /// Shows the hidden place `uri` again: a place hidden with Hide, or a
    /// standard folder in Quick access.
    fn show_hidden_place(&self, uri: &str) {
        let uri = uri.to_owned();
        let hidden_places = self.context().settings_data().preferences.hidden_sidebar_places;
        let change: Change = if hidden_places.iter().any(|place| same_location(place, &uri)) {
            Box::new(move |settings| settings.set_place_hidden(&uri, false).map(|_| ()))
        } else {
            Box::new(move |settings| settings.show_in_quick_access(&uri))
        };
        self.save_sidebar_change(change);
    }

    /// Saves `change`; every window redraws its places once it is saved.
    fn save_sidebar_change(&self, change: Change) {
        self.context().change_settings(
            change,
            glib::clone!(
                #[weak(rename_to = window)]
                self,
                move |result: Result<(), SettingsError>| {
                    if let Err(error) = result {
                        window.show_message(&ox_core::i18n::format_message(
                            "Could not save the sidebar: {error}",
                            &[("error", &error.to_string())],
                        ));
                    }
                }
            ),
        );
    }
}

/// Whether `entry` is a place hidden with Hide: one of `hidden_places`.
/// Pins and standard folders are hidden from Quick access instead.
fn is_hidden_place(entry: &SidebarEntry, hidden_places: &[String]) -> bool {
    match &entry.target {
        RowTarget::Location(uri) if !entry.pinned => {
            hidden_places.iter().any(|place| same_location(place, uri))
        }
        _ => false,
    }
}
