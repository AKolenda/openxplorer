// SPDX-License-Identifier: AGPL-3.0-only
//! The window's side of the folder tree: View > Folder tree and F7
//! (`win.folder-tree`), the options of its menu
//! (`win.folder-tree-option`), both saved for every window, and the tree
//! following the active tab.

use gtk::prelude::*;
use gtk::subclass::prelude::*;
use ox_core::settings::FolderTreeOptions;

use super::menu::toggled;
use super::FolderTree;
use crate::locations::Page;
use crate::window::actions::{text_action, toggle_action};
use crate::window::preferences::Preference;
use crate::window::window_action::WindowAction;
use crate::window::BrowserWindow;

impl BrowserWindow {
    /// The folder tree in the navigation pane.
    pub(in crate::window) fn folder_tree(&self) -> &FolderTree {
        self.sidebar().folder_tree()
    }

    /// Adds `win.folder-tree` and `win.folder-tree-option`, and shows the
    /// tree as the preferences say.
    pub(in crate::window) fn install_folder_tree(&self) {
        let options = self.context().settings_data().preferences.folder_tree;
        self.add_action_entries([
            toggle_action(WindowAction::FolderTree, options.shown, |window, shown| {
                window.change_folder_tree(FolderTreeOptions {
                    shown,
                    ..window.folder_tree().options()
                });
            }),
            text_action(WindowAction::FolderTreeOption, |window, name| {
                if let Some(options) = toggled(window.folder_tree().options(), name) {
                    window.change_folder_tree(options);
                }
            }),
        ]);
        self.apply_folder_tree(options);
    }

    /// Applies and saves the tree's `options`.
    fn change_folder_tree(&self, options: FolderTreeOptions) {
        self.apply_folder_tree(options);
        self.save_preference(Preference::FolderTree(options));
    }

    /// Shows the tree with `options` at the folder shown.
    fn apply_folder_tree(&self, options: FolderTreeOptions) {
        self.folder_tree().set_options(options);
        self.follow_with_folder_tree();
    }

    /// Has the tree follow the active tab's folder; on a page it selects
    /// nothing.
    pub(in crate::window) fn follow_with_folder_tree(&self) {
        let uri = self.current_uri().filter(|uri| Page::from_uri(uri).is_none());
        let home = self.imp().locations.borrow().home_uri();
        self.folder_tree().follow(uri.as_deref(), &home, |uri| {
            self.imp().locations.borrow().title_for(uri)
        });
    }

    /// Whether the clipboard's files can be pasted into the folder `uri`
    /// now.
    pub(in crate::window) fn can_paste_into(&self, uri: &str) -> bool {
        let facts = self.command_facts();
        facts.has_file_clipboard && !facts.is_busy && self.imp().locations.borrow().is_writable_location(uri)
    }
}
