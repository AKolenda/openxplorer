// SPDX-License-Identifier: AGPL-3.0-only
//! Windows 11's search layout, the default (SRCH-046): while a search
//! shows, the command bar offers Search options (All subfolders or
//! Current folder; Date modified, Kind and Size; File contents) and Close
//! search in place of the search filter bar, and the words searched for
//! are marked in the names. Settings > Search > "Show the search filter
//! bar" shows the filter bar instead, as before.
//!
//! Windows 11's Search options also lists System files, Zipped
//! (compressed) folders and Change indexed locations. The app does not
//! search inside ZIP files or keep system files apart from hidden ones,
//! and indexes folders from Settings > Search, so it leaves them out.

use gtk::prelude::*;
use gtk::subclass::prelude::*;
use ox_core::search::{is_wildcard, DateFacet, KindFacet, SearchFacets, SearchIn, SizeFacet};

use super::actions::{choice_action, plain_action, toggle_action};
use super::window_action::WindowAction;
use super::BrowserWindow;

/// Search options > All subfolders.
pub(super) const ALL_SUBFOLDERS: &str = "all";
/// Search options > Current folder.
pub(super) const CURRENT_FOLDER: &str = "current";

/// The key of the option at `position` in a list of options, as the
/// actions of Search options carry it.
pub(super) fn option_key(position: usize) -> String {
    position.to_string()
}

/// The option of `options` whose key is `key`.
fn option_of<T: Copy>(options: &[T], key: &str) -> Option<T> {
    key.parse::<usize>()
        .ok()
        .and_then(|position| options.get(position).copied())
}

/// The key of `option` in `options`; the first option's when it is not
/// listed, as a typed size is not.
fn key_of<T: PartialEq>(options: &[T], option: &T) -> String {
    option_key(options.iter().position(|listed| listed == option).unwrap_or(0))
}

impl BrowserWindow {
    /// Registers Search options' choices and Close search.
    pub(super) fn install_search_layout_actions(&self) {
        let depth = choice_action(WindowAction::SearchDepth, ALL_SUBFOLDERS, |window, key| {
            window.search_current_folder_only(key == CURRENT_FOLDER);
            true
        });
        let date = choice_action(WindowAction::SearchDate, &option_key(0), |window, key| {
            option_of(&DateFacet::ALL, key).is_some_and(|date| {
                window.change_search_facets(|facets| facets.date = date);
                true
            })
        });
        let kind = choice_action(WindowAction::SearchKind, &option_key(0), |window, key| {
            option_of(&KindFacet::ALL, key).is_some_and(|kind| {
                window.change_search_facets(|facets| facets.kind = kind);
                true
            })
        });
        let size = choice_action(WindowAction::SearchSize, &option_key(0), |window, key| {
            option_of(&SizeFacet::ALL, key).is_some_and(|size| {
                window.change_search_facets(|facets| facets.size = size);
                true
            })
        });
        let contents = toggle_action(WindowAction::SearchContents, false, |window, on| {
            let search_in = if on {
                SearchIn::NamesAndContents
            } else {
                SearchIn::Names
            };
            let facets = window.imp().search.borrow().facets();
            window.change_search_options(search_in, facets);
        });
        let close = plain_action(WindowAction::CloseSearch, |window| window.search_box().clear());
        self.add_action_entries([depth, date, kind, size, contents, close]);
    }

    /// Whether a search shows the search filter bar, as Settings > Search
    /// asks, rather than Windows 11's Search options and Close search.
    pub(super) fn shows_search_filter_bar(&self) -> bool {
        self.context().settings_data().preferences.show_search_filter_bar
    }

    /// Follows Settings > Search > "Show the search filter bar": a search
    /// shown swaps its options and titles at once.
    pub(super) fn follow_search_filter_bar(&self, shown: bool) {
        if self.imp().shows_search_filter_bar.replace(Some(shown)) == Some(shown) {
            return;
        }
        if self.is_searching() {
            self.show_search_state();
            self.render_location();
        }
    }

    /// Opens Search options, as a click on it does, while it shows; for
    /// the snapshot hook's scenes.
    pub(crate) fn open_search_options(&self) -> bool {
        let button = self.command_bar().search_options_button();
        button
            .filter(WidgetExt::is_visible)
            .inspect(gtk::MenuButton::popup)
            .is_some()
    }

    /// Opens the submenu labelled `label` of Search options while it is
    /// open; for the snapshot hook's scenes.
    pub(crate) fn open_search_options_submenu(&self, label: &str) -> bool {
        let menu = self
            .command_bar()
            .search_options_button()
            .and_then(|button| button.popover())
            .and_downcast::<super::menu_popover::MenuPopover>();
        menu.is_some_and(|menu| menu.is_visible() && menu.open_submenu_labelled(label))
    }

    /// Searches the folder alone, or with its subfolders, and runs the
    /// search again.
    fn search_current_folder_only(&self, only: bool) {
        let changed = self.imp().search.borrow().is_current_folder_only() != only;
        if changed {
            self.imp().search.borrow_mut().set_current_folder_only(only);
            self.run_search();
        }
    }

    /// Changes the date, kind or size option as `change` does.
    fn change_search_facets(&self, change: impl FnOnce(&mut SearchFacets)) {
        let (search_in, mut facets) = {
            let search = self.imp().search.borrow();
            (search.search_in(), search.facets())
        };
        change(&mut facets);
        self.change_search_options(search_in, facets);
    }

    /// Shows the search's options in Search options and the command bar's
    /// search commands, or the filter bar instead; and marks the words
    /// searched for in the names while Search options shows.
    pub(super) fn show_search_layout(&self) {
        let filter_bar = self.shows_search_filter_bar();
        let (is_active, depth, facets, search_in, words) = {
            let search = self.imp().search.borrow();
            let depth = if search.is_current_folder_only() {
                CURRENT_FOLDER
            } else {
                ALL_SUBFOLDERS
            };
            (
                search.is_active(),
                depth,
                search.facets(),
                search.search_in(),
                marked_words(search.words()),
            )
        };
        let windows_layout = is_active && !filter_bar;
        if !filter_bar {
            self.search_strip().set_visible(false);
        }
        self.command_bar().show_search_commands(windows_layout);
        self.set_action_state(WindowAction::SearchDepth, &depth.to_variant());
        let date = key_of(&DateFacet::ALL, &facets.date);
        self.set_action_state(WindowAction::SearchDate, &date.to_variant());
        let kind = key_of(&KindFacet::ALL, &facets.kind);
        self.set_action_state(WindowAction::SearchKind, &kind.to_variant());
        let size = key_of(&SizeFacet::ALL, &facets.size);
        self.set_action_state(WindowAction::SearchSize, &size.to_variant());
        let contents = search_in == SearchIn::NamesAndContents;
        self.set_action_state(WindowAction::SearchContents, &contents.to_variant());
        let words = if windows_layout { words } else { Vec::new() };
        for pane in self.folder_panes() {
            let marks = if self.is_active_pane(pane) {
                words.clone()
            } else {
                Vec::new()
            };
            pane.owners().set_marked_words(marks);
        }
    }
}

/// The words of `text` a result's name is marked at: each word once,
/// lower-cased, without wildcard patterns, which match whole names.
fn marked_words(text: &str) -> Vec<String> {
    let mut words: Vec<String> = Vec::new();
    for word in text.to_lowercase().split_whitespace() {
        if !is_wildcard(word) && !words.iter().any(|known| known == word) {
            words.push(word.to_owned());
        }
    }
    words
}
