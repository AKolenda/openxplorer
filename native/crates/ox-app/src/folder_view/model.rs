// SPDX-License-Identifier: AGPL-3.0-only
//! The list models behind both views: filter, sort and selection.
//!
//! Each tab owns a `gio::ListStore` of [`FileItem`]s; the folder view shows
//! the active tab's store through one filter model (hidden items and the
//! search box), one sort model (folders first, then the chosen column in
//! natural order, ties by name ascending, as `filtered()` in app.js) and one
//! multi-selection shared by the details and icon views.

use std::cell::RefCell;
use std::cmp::Ordering;
use std::rc::Rc;

use gtk::prelude::*;
use gtk::{gio, glib};

use crate::folder_view::filter::FilterState;
use crate::folder_view::item::FileItem;
use crate::folder_view::sorting::{self, SortColumn};

fn as_item(object: &glib::Object) -> &FileItem {
    object
        .downcast_ref::<FileItem>()
        .expect("folder models hold FileItems")
}

fn to_gtk(order: Ordering) -> gtk::Ordering {
    match order {
        Ordering::Less => gtk::Ordering::Smaller,
        Ordering::Equal => gtk::Ordering::Equal,
        Ordering::Greater => gtk::Ordering::Larger,
    }
}

/// Compares two items by one column, without folders-first or tie-breaks.
pub fn compare_column(column: SortColumn, a: &FileItem, b: &FileItem) -> Ordering {
    let (x, y) = (a.entry(), b.entry());
    match column {
        SortColumn::Name => sorting::natural_cmp(a.name_key(), b.name_key()),
        SortColumn::Modified => x.modified.cmp(&y.modified),
        SortColumn::Type => sorting::natural_cmp(a.type_key(), b.type_key()),
        SortColumn::Size => x.size.unwrap_or(0).cmp(&y.size.unwrap_or(0)),
    }
}

/// The sorter a details column uses.
pub fn column_sorter(column: SortColumn) -> gtk::CustomSorter {
    gtk::CustomSorter::new(move |a, b| to_gtk(compare_column(column, as_item(a), as_item(b))))
}

fn folders_first() -> gtk::CustomSorter {
    gtk::CustomSorter::new(|a, b| {
        let (x, y) = (as_item(a).entry(), as_item(b).entry());
        // `true` sorts first.
        to_gtk(y.is_dir.cmp(&x.is_dir))
    })
}

fn names_ascending() -> gtk::CustomSorter {
    gtk::CustomSorter::new(|a, b| {
        let (x, y) = (as_item(a), as_item(b));
        to_gtk(sorting::compare_names(
            x.name_key(),
            &x.entry().name,
            y.name_key(),
            &y.entry().name,
        ))
    })
}

/// Counts for the status bar.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SelectionSummary {
    /// Selected items.
    pub count: u32,
    /// Total size of the selected files (folders count as 0).
    pub bytes: u64,
    /// At least one selected item is a file with a known size.
    pub has_files: bool,
}

/// Filter, sort and selection over the active tab's store.
pub struct FolderModel {
    filter_state: Rc<RefCell<FilterState>>,
    filter: gtk::CustomFilter,
    filter_model: gtk::FilterListModel,
    sort_model: gtk::SortListModel,
    selection: gtk::MultiSelection,
}

impl FolderModel {
    /// Empty models; [`FolderModel::set_store`] shows a tab's items.
    pub fn new() -> Self {
        let filter_state = Rc::new(RefCell::new(FilterState::default()));
        let state = Rc::clone(&filter_state);
        let filter = gtk::CustomFilter::new(move |object| {
            let item = as_item(object);
            state.borrow().accepts(item.lower_name(), item.entry().is_hidden)
        });
        let filter_model = gtk::FilterListModel::new(None::<gio::ListStore>, Some(filter.clone()));
        let sort_model = gtk::SortListModel::new(Some(filter_model.clone()), None::<gtk::Sorter>);
        let selection = gtk::MultiSelection::new(Some(sort_model.clone()));
        Self {
            filter_state,
            filter,
            filter_model,
            sort_model,
            selection,
        }
    }

    /// Completes the sorter once the details view exists: folders first,
    /// then `column_sorter` (the column view's sorter, which applies the
    /// chosen direction), then names ascending.
    pub fn attach_column_sorter(&self, column_sorter: &gtk::Sorter) {
        let sorter = gtk::MultiSorter::new();
        sorter.append(folders_first());
        sorter.append(column_sorter.clone());
        sorter.append(names_ascending());
        self.sort_model.set_sorter(Some(&sorter));
    }

    /// The selection model both views display.
    pub fn selection(&self) -> &gtk::MultiSelection {
        &self.selection
    }

    /// The sorted, filtered items in display order.
    pub fn sorted(&self) -> &gtk::SortListModel {
        &self.sort_model
    }

    /// Shows another tab's items.
    pub fn set_store(&self, store: Option<&gio::ListStore>) {
        self.filter_model.set_model(store);
    }

    /// Items shown (after filtering).
    pub fn n_items(&self) -> u32 {
        self.sort_model.n_items()
    }

    /// The item at a display position.
    pub fn item(&self, position: u32) -> Option<FileItem> {
        self.sort_model.item(position).and_downcast::<FileItem>()
    }

    /// The display name at a position ("" past the end).
    pub fn name_at(&self, position: u32) -> String {
        self.item(position)
            .map(|item| item.entry().name.clone())
            .unwrap_or_default()
    }

    /// Sets the search text; returns true when the shown items changed.
    pub fn set_query(&self, query: &str) -> bool {
        let changed = self.filter_state.borrow_mut().set_query(query);
        if changed {
            self.filter.changed(gtk::FilterChange::Different);
        }
        changed
    }

    /// True while the search box filters the folder.
    pub fn is_searching(&self) -> bool {
        self.filter_state.borrow().is_searching()
    }

    /// Shows or hides hidden items; returns true when that changed.
    pub fn set_show_hidden(&self, show: bool) -> bool {
        let changed = self.filter_state.borrow_mut().set_show_hidden(show);
        if changed {
            self.filter.changed(gtk::FilterChange::Different);
        }
        changed
    }

    /// Display positions of the selected items, ascending.
    pub fn selected_positions(&self) -> Vec<u32> {
        let bitset = self.selection.selection();
        let count = bitset.size().min(u64::from(u32::MAX));
        (0..count as u32).map(|nth| bitset.nth(nth)).collect()
    }

    /// The selected items in display order.
    pub fn selected_items(&self) -> Vec<FileItem> {
        self.selected_positions()
            .into_iter()
            .filter_map(|position| self.item(position))
            .collect()
    }

    /// The first selected position, if any.
    pub fn first_selected(&self) -> Option<u32> {
        let bitset = self.selection.selection();
        (!bitset.is_empty()).then(|| bitset.minimum())
    }

    /// Count and size of the selection.
    pub fn summary(&self) -> SelectionSummary {
        let mut summary = SelectionSummary::default();
        for item in self.selected_items() {
            summary.count += 1;
            if let (false, Some(size)) = (item.entry().is_dir, item.entry().size) {
                summary.bytes += size;
                summary.has_files = true;
            }
        }
        summary
    }

    /// Selects only `position`.
    pub fn select_only(&self, position: u32) {
        self.selection.select_item(position, true);
    }

    /// Selects every shown item.
    pub fn select_all(&self) {
        self.selection.select_all();
    }

    /// Clears the selection.
    pub fn select_none(&self) {
        self.selection.unselect_all();
    }

    /// Selects exactly the items that were not selected.
    pub fn invert_selection(&self) {
        let count = self.n_items();
        let everything = gtk::Bitset::new_range(0, count);
        let inverted = gtk::Bitset::new_range(0, count);
        inverted.subtract(&self.selection.selection());
        self.selection.set_selection(&inverted, &everything);
    }

    /// Selects the items whose URIs are in `uris` (used after a refresh).
    pub fn select_uris(&self, uris: &[String]) {
        if uris.is_empty() {
            self.select_none();
            return;
        }
        let count = self.n_items();
        let wanted = gtk::Bitset::new_empty();
        for position in 0..count {
            let matches = self
                .item(position)
                .is_some_and(|item| uris.iter().any(|uri| *uri == item.entry().uri));
            if matches {
                wanted.add(position);
            }
        }
        self.selection
            .set_selection(&wanted, &gtk::Bitset::new_range(0, count));
    }

    /// Display position of the item with `uri`.
    pub fn position_of(&self, uri: &str) -> Option<u32> {
        (0..self.n_items()).find(|position| self.item(*position).is_some_and(|item| item.entry().uri == uri))
    }
}

impl Default for FolderModel {
    fn default() -> Self {
        Self::new()
    }
}
