// SPDX-License-Identifier: AGPL-3.0-only
//! Drops on the folder tree (DND-011, SIDE-028): the folder under the
//! pointer takes them, as a folder of Windows Explorer's navigation pane
//! does, with the drop outline the navigation pane's places get, and a
//! collapsed folder held under a drag opens its subfolders after a
//! moment, as the navigation pane's This PC does (DND-027). The window's
//! drop target ([`crate::window::file_drop`]) decides what the drop does.

use std::time::Duration;

use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;

use super::{model, FolderTree};

/// The class of the row a drop would go into.
const DROP_CLASS: &str = "file-drop-active";

/// How long a drag rests on a collapsed folder before its subfolders
/// show: the navigation pane's wait (DND-027).
const DRAG_EXPAND_DELAY: Duration = Duration::from_millis(750);

impl FolderTree {
    /// The list of rows, which takes the window's drops.
    pub(in crate::window) fn rows_list(&self) -> &gtk::ListView {
        self.imp().view.get().expect("constructed builds the view")
    }

    /// The widget that shows the row at `position`, while it is shown.
    fn row_widget(&self, position: u32) -> Option<gtk::Widget> {
        self.imp()
            .bound
            .borrow()
            .iter()
            .filter_map(|bound| bound.item.upgrade())
            .find(|item| item.position() == position)
            .and_then(|item| item.child())
            .and_then(|expander| expander.parent())
    }

    /// The position and location of the folder whose row is at `y` of the
    /// list, while one is shown there.
    pub(in crate::window) fn folder_at(&self, y: f64) -> Option<(u32, String)> {
        let list = self.rows_list();
        let position = self
            .imp()
            .bound
            .borrow()
            .iter()
            .filter_map(|bound| bound.item.upgrade())
            .find(|item| {
                let bounds = item
                    .child()
                    .and_then(|expander| expander.parent())
                    .and_then(|row| row.compute_bounds(list));
                bounds.is_some_and(|bounds| {
                    let (top, height) = (f64::from(bounds.y()), f64::from(bounds.height()));
                    (top..top + height).contains(&y)
                })
            })
            .map(|item| item.position())?;
        Some((position, self.uri_at(position)?))
    }

    /// Outlines the row at `position` as where a drop would go, or no row
    /// with `None`.
    pub(in crate::window) fn show_drop_row(&self, position: Option<u32>) {
        let rows: Vec<gtk::Widget> = self
            .imp()
            .bound
            .borrow()
            .iter()
            .filter_map(|bound| bound.item.upgrade())
            .filter_map(|item| item.child())
            .filter_map(|expander| expander.parent())
            .collect();
        for row in rows {
            row.remove_css_class(DROP_CLASS);
        }
        if let Some(row) = position.and_then(|position| self.row_widget(position)) {
            row.add_css_class(DROP_CLASS);
        }
    }

    /// Opens the subfolders of the collapsed folder at `position` once a
    /// drag has rested on it for [`DRAG_EXPAND_DELAY`]; `None`, or another
    /// folder, calls a waiting one off.
    pub(in crate::window) fn expand_after_drag_hover(&self, position: Option<u32>) {
        let row = position
            .and_then(|position| self.row(position))
            .filter(|row| row.is_expandable() && !row.is_expanded());
        let waiting = self
            .imp()
            .drag_expand
            .borrow()
            .as_ref()
            .map(|(waiting, _)| waiting.clone());
        if waiting == row {
            return;
        }
        if let Some((_, timer)) = self.imp().drag_expand.take() {
            timer.remove();
        }
        let Some(row) = row else {
            return;
        };
        let timer = glib::timeout_add_local_once(
            DRAG_EXPAND_DELAY,
            glib::clone!(
                #[weak(rename_to = tree)]
                self,
                #[weak]
                row,
                move || {
                    tree.imp().drag_expand.replace(None);
                    model::load_children(&row);
                    row.set_expanded(true);
                }
            ),
        );
        self.imp().drag_expand.replace(Some((row, timer)));
    }

    /// Whether the row at `position` is outlined as where a drop would
    /// go, for tests.
    #[cfg(test)]
    pub(in crate::window) fn is_drop_row(&self, position: u32) -> bool {
        self.row_widget(position)
            .is_some_and(|row| row.has_css_class(DROP_CLASS))
    }
}
