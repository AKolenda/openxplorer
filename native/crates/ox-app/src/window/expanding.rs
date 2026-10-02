// SPDX-License-Identifier: AGPL-3.0-only
//! When folders expand in place, and the keys that expand them (VIEW-035).
//!
//! Dolphin's details view expands folders while `ExpandableFolders` is on;
//! the native app does so in the details view of a folder, not while it
//! searches or shows groups. Right expands the current folder and the
//! selected ones, Left collapses them, or goes to the folder an item is in,
//! as in Dolphin; Back and Forward return to a folder with the same folders
//! expanded.

use gtk::subclass::prelude::*;
use gtk::{gdk, glib};

use super::folder_pane::FolderView;
use super::session::TabId;
use super::BrowserWindow;
use crate::folder_view::sorting::SortColumn;

impl BrowserWindow {
    /// Lets folders expand when the preference, the view, the search and
    /// the groups allow it, and redraws the arrows when that changes.
    pub(super) fn update_expandability(&self) {
        let pane = self.folder_pane();
        let expandable = self.context().settings_data().preferences.expandable_folders
            && pane.view() == FolderView::Details
            && !self.imp().search.borrow().is_active()
            && pane.model().grouping().is_none();
        let tree = pane.model().tree();
        if tree.is_expandable() != expandable {
            tree.set_expandable(expandable);
            pane.details().redraw_column(SortColumn::Name);
        }
    }

    /// Expands again the folders tab `id` had expanded when Back or
    /// Forward left it, now that it is listed.
    pub(super) fn restore_expanded_after_listing(&self, id: TabId) {
        let expanded = {
            let mut session = self.imp().session.borrow_mut();
            session
                .tab_mut(id)
                .map(|tab| std::mem::take(&mut tab.expand_after_listing))
        };
        if let Some(expanded) = expanded.filter(|expanded| !expanded.is_empty()) {
            self.folder_pane().model().tree().expand_when_listed(expanded);
        }
    }

    /// Right and Left in the details view: expand or collapse the current
    /// folder and the selected ones; Left on an item inside an expanded
    /// folder goes to that folder. `None` for every other key, or when
    /// folders do not expand.
    pub(super) fn tree_key(&self, key: gdk::Key, modifiers: gdk::ModifierType) -> Option<glib::Propagation> {
        let expand = match key {
            gdk::Key::Right | gdk::Key::KP_Right => true,
            gdk::Key::Left | gdk::Key::KP_Left => false,
            _ => return None,
        };
        let pane = self.folder_pane();
        let tree = pane.model().tree();
        if !modifiers.is_empty() || !tree.is_expandable() {
            return None;
        }
        let current = pane.focused_position()?;
        let current_row = tree.row(current)?;
        let mut positions = pane.model().selected_positions();
        if !positions.contains(&current) {
            positions.push(current);
        }
        let rows: Vec<gtk::TreeListRow> = positions
            .into_iter()
            .filter_map(|position| tree.row(position))
            .collect();
        let changes = rows
            .iter()
            .any(|row| row.is_expandable() && row.is_expanded() != expand);
        if changes {
            for row in &rows {
                tree.set_expanded(row, expand);
            }
        } else if let Some(parent) = current_row.parent().filter(|_| !expand) {
            pane.select_and_reveal(parent.position());
        }
        self.reset_typeahead();
        Some(glib::Propagation::Stop)
    }
}
