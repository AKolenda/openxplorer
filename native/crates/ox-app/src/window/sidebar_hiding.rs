// SPDX-License-Identifier: AGPL-3.0-only
//! Hiding places and sections of the sidebar, and showing them again
//! (SIDE-010).
//!
//! Dolphin's Places panel hides one place with "Hide" and a whole group
//! with "Hide Section", and "Show All Entries" lists what is hidden,
//! dimmed, so it can be shown again. Here "Unpin from Quick access" hides
//! a standard folder (SIDE-009), every row's menu ends with "Hide section"
//! for its group, saved for every window as `hiddenSidebarSections`, and
//! the empty-space menu's "Show all entries" (this window only, available
//! while anything is hidden) lists the hidden rows dimmed, whose menus
//! offer "Show" and "Show section".

use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;
use ox_core::location::same_location;
use ox_core::settings::{PreferencesUpdate, SettingsError};

use crate::settings_store::Change;

use super::actions::{text_action, toggle_action};
use super::sidebar::entries::{place_entry, Section, SidebarEntry};
use super::sidebar::HiddenRow;
use super::window_action::WindowAction;
use super::BrowserWindow;

impl BrowserWindow {
    /// Adds `win.sidebar-show-all`, `win.hide-section`,
    /// `win.show-section` and `win.show-place`.
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
        let show_all = self.imp().sidebar_show_all.get();
        let is_hidden = |section: Section| {
            section
                .hiding()
                .is_some_and(|(key, _)| hidden_sections.iter().any(|hidden| hidden == key))
        };
        let mut rows: Vec<(SidebarEntry, HiddenRow)> = entries
            .into_iter()
            .filter_map(|entry| match (is_hidden(entry.section), show_all) {
                (false, _) => Some((entry, HiddenRow::Shown)),
                (true, true) => Some((entry, HiddenRow::Section)),
                (true, false) => None,
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
        let anything_hidden = !hidden_sections.is_empty() || !hidden_folders.is_empty();
        (rows, anything_hidden)
    }

    /// Hides or shows the sidebar section saved as `key`, for every window.
    fn change_hidden_sections(&self, key: &str, hide: bool) {
        let mut keys = self.context().settings_data().preferences.hidden_sidebar_sections;
        keys.retain(|hidden| hidden != key);
        if hide {
            keys.push(key.to_owned());
        }
        let update = PreferencesUpdate {
            hidden_sidebar_sections: Some(keys),
            ..PreferencesUpdate::default()
        };
        let change: Change = Box::new(move |settings| settings.update_preferences(&update).map(|_| ()));
        self.save_sidebar_change(change);
    }

    /// Shows the hidden standard folder `uri` in Quick access again.
    fn show_hidden_place(&self, uri: &str) {
        let uri = uri.to_owned();
        let change: Change = Box::new(move |settings| settings.show_in_quick_access(&uri));
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
                        window.show_message(&format!("Could not save the sidebar: {error}"));
                    }
                }
            ),
        );
    }
}
