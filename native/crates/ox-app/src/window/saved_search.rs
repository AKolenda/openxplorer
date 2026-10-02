// SPDX-License-Identifier: AGPL-3.0-only
//! Saving the search shown to the sidebar and running a saved one again
//! (SRCH-038).
//!
//! From Dolphin's search box, whose save button adds the query as a place
//! named "Search for <text> in <folder>"; opening it opens the folder and
//! runs the search there again. The search strip's "Save search" runs
//! `win.save-search`; a saved search's row runs `win.open-saved-search`,
//! and its menu can remove it with `win.forget-saved-search`. Both take
//! the search's `(folder, text)` as their target.

use gtk::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{gio, glib};
use ox_core::search::SavedSearch;

use super::actions::plain_action;
use super::menu_popover::{MenuEntry, MenuItem};
use super::window_action::WindowAction;
use super::BrowserWindow;
use crate::icons::Icon;
use crate::search::SearchScope;

/// What the window says once a search was saved.
const SEARCH_SAVED: &str = crate::i18n::message_id("Search saved to the navigation pane");

/// The `(folder, text)` target of the actions on `search`.
pub(super) fn saved_search_target(search: &SavedSearch) -> glib::Variant {
    (search.folder.as_str(), search.text.as_str()).to_variant()
}

/// The menu of a saved search's row.
pub(super) fn saved_search_menu(search: &SavedSearch) -> Vec<MenuEntry> {
    let target = saved_search_target(search);
    vec![
        MenuItem::with_target(
            &ox_core::i18n::gettext("Open"),
            Icon::Search,
            WindowAction::OpenSavedSearch,
            target.clone(),
        )
        .into(),
        MenuEntry::Divider,
        MenuItem::with_target(
            &ox_core::i18n::gettext("Remove from navigation pane"),
            Icon::Dismiss,
            WindowAction::ForgetSavedSearch,
            target,
        )
        .into(),
    ]
}

/// An action whose target is a saved search's `(folder, text)`.
fn saved_search_action(
    window_action: WindowAction,
    run: impl Fn(&BrowserWindow, &str, &str) + 'static,
) -> gio::ActionEntry<BrowserWindow> {
    gio::ActionEntry::builder(window_action.name())
        .parameter_type(Some(&<(String, String)>::static_variant_type()))
        .activate(move |window: &BrowserWindow, _, target| {
            let search = target.and_then(<(String, String)>::from_variant);
            if let Some((folder, text)) = search {
                run(window, &folder, &text);
            }
        })
        .build()
}

impl BrowserWindow {
    /// Adds Save search and the actions of a saved search's row.
    pub(super) fn install_saved_search_actions(&self) {
        self.add_action_entries([
            plain_action(WindowAction::SaveSearch, BrowserWindow::save_search),
            saved_search_action(WindowAction::OpenSavedSearch, BrowserWindow::open_saved_search),
            saved_search_action(WindowAction::ForgetSavedSearch, |window, folder, text| {
                let saved = window.context().saved_searches();
                let search = saved
                    .into_iter()
                    .find(|search| search.folder == folder && search.text == text);
                if let Some(search) = search {
                    window.context().forget_search(search);
                }
            }),
        ]);
    }

    /// Saves the search shown to the sidebar, named after the folder.
    fn save_search(&self) {
        let Some(folder) = self.current_uri() else {
            return;
        };
        let (text, search_in, scope) = {
            let search = self.imp().search.borrow();
            (search.query().to_owned(), search.search_in(), search.scope())
        };
        let folder_name = self.imp().locations.borrow().title_for(&folder);
        let search = match SavedSearch::new(&folder, &text, &folder_name) {
            Ok(search) => SavedSearch {
                search_in,
                all_cached_folders: scope == SearchScope::AllCachedFolders,
                ..search
            },
            Err(error) => {
                self.show_message(&error.to_string());
                return;
            }
        };
        let window = self.downgrade();
        self.context().save_search(search, move |result| {
            if let Some(window) = window.upgrade() {
                let message = result.map_or_else(
                    |error| error.to_string(),
                    |()| ox_core::i18n::gettext_static(SEARCH_SAVED).to_owned(),
                );
                window.show_message(&message);
            }
        });
    }

    /// Opens `folder` and searches it for `text` again, with the options
    /// it was saved with.
    fn open_saved_search(&self, folder: &str, text: &str) {
        let saved = self.context().saved_searches().into_iter();
        let options = saved
            .filter(|search| search.folder == folder && search.text == text)
            .map(|search| (search.search_in, search.all_cached_folders))
            .next_back();
        if let Err(error) = self.navigate(folder) {
            self.show_message(&error.to_string());
            return;
        }
        // After the navigation has ended the search of the folder left.
        let text = text.to_owned();
        glib::idle_add_local_once(glib::clone!(
            #[weak(rename_to = window)]
            self,
            move || {
                if let Some((search_in, all_cached_folders)) = options {
                    let mut search = window.imp().search.borrow_mut();
                    search.set_search_in(search_in);
                    search.set_scope(if all_cached_folders {
                        SearchScope::AllCachedFolders
                    } else {
                        SearchScope::ThisFolder
                    });
                }
                window.search_box().set_query(&text);
            }
        ));
    }
}
