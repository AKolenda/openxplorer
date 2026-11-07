// SPDX-License-Identifier: AGPL-3.0-only
//! The heading above each group of a grouped listing (VIEW-022).
//!
//! Windows 11 heads each group with a chevron, its name and a line to the
//! right edge ("Today ———"; since 22H2 without the item count); the
//! chevron collapses the group under its heading and expands it again. The details view
//! lists the folder through a [`GroupedSelection`] in one column while it
//! is grouped, as Content does: each heading is a row of its own, which
//! the keyboard can rest on, and a collapsed group lists no rows. The
//! Name column's cell draws the chevron, the title and the start of the
//! line; the other columns' cells carry the line on. GTK's own section
//! headers take no keyboard focus, so they could not be collapsed from
//! the keyboard as Windows' headings are.

use std::rc::Rc;

use gtk::subclass::prelude::*;

use super::DetailsView;
use crate::folder_view::grid::GroupedSelection;
use crate::folder_view::item::FileItem;

/// The CSS class of a view that heads the folder's groups.
pub(crate) const GROUPED_CLASS: &str = "grouped";

/// Names the group an item is in, `None` while the items are not grouped.
pub(crate) type GroupTitle = Rc<dyn Fn(&FileItem) -> Option<String>>;

impl DetailsView {
    /// Shows the folder's `items`, or nothing.
    pub(crate) fn set_model(&self, items: Option<&gtk::MultiSelection>) {
        let imp = self.imp();
        let shown = self.column_view().model().is_some();
        if imp.items.borrow().as_ref() == items && shown == items.is_some() {
            return;
        }
        imp.items.replace(items.cloned());
        self.show_model();
    }

    /// Heads each group of the listing with its title, or shows no groups
    /// with `None`.
    pub(crate) fn set_group_titles(&self, title: Option<super::GroupTitle>) {
        let grouped_now = title.is_some();
        let was_grouped = self.imp().group_title.replace(title).is_some();
        match self.grouped() {
            // Grouped anew: the groups may have moved.
            Some(grouped) if grouped_now => grouped.regroup(),
            _ if was_grouped || grouped_now => self.show_model(),
            _ => {}
        }
    }

    /// Whether the groups are headed.
    pub(crate) fn heads_groups(&self) -> bool {
        self.imp().group_title.borrow().is_some()
    }

    /// The grouped model the rows show, while the folder is grouped.
    pub(crate) fn grouped(&self) -> Option<GroupedSelection> {
        self.imp().grouping.borrow().clone()
    }

    /// The listing position of the item the row at `row` shows.
    pub(crate) fn item_at(&self, row: u32) -> Option<u32> {
        match self.grouped() {
            Some(grouped) => grouped.item_at(row),
            None => Some(row),
        }
    }

    /// Moves to the item at the listing's `position` with `flags`,
    /// scrolling as `scroll` allows.
    pub(crate) fn scroll_to(
        &self,
        position: u32,
        flags: gtk::ListScrollFlags,
        scroll: Option<gtk::ScrollInfo>,
    ) {
        let row = match self.grouped() {
            Some(grouped) => grouped.slot_of(position),
            None => Some(position),
        };
        if let Some(row) = row {
            self.column_view().scroll_to(row, None, flags, scroll);
        }
    }

    /// Gives the column view the folder's items, grouped under headings
    /// while the folder is grouped.
    fn show_model(&self) {
        let imp = self.imp();
        let items = imp.items.borrow().clone();
        let title = imp.group_title.borrow().clone();
        let grouped = match (items.as_ref(), title) {
            (Some(items), Some(title)) => {
                let collapsed = Rc::clone(&imp.collapsed.borrow());
                Some(GroupedSelection::new(items, title, &collapsed, 1))
            }
            _ => None,
        };
        imp.grouping.replace(grouped.clone());
        let column_view = self.column_view();
        // The stylesheet spaces a grouped list's headings evenly from the
        // column titles down.
        crate::window::widget_tree::toggle_class(column_view, GROUPED_CLASS, grouped.is_some());
        match (grouped, items) {
            (Some(grouped), _) => column_view.set_model(Some(&grouped)),
            (None, items) => column_view.set_model(items.as_ref()),
        }
    }
}
