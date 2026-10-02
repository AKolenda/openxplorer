// SPDX-License-Identifier: AGPL-3.0-only
//! The sorters of the folder model: folders first, a further sort key
//! (VIEW-019), the details column, names as the tie-break, and the groups
//! (VIEW-022) the items are sorted into first.

use std::cell::Cell;
use std::cmp::Ordering;
use std::rc::Rc;

use gtk::glib;
use gtk::prelude::*;

use crate::folder_view::groups::{self, GroupClock};
use crate::folder_view::item::FileItem;
use crate::folder_view::sort_roles::{SortRole, SortState};
use crate::folder_view::sorting::{self, SortColumn, SortDirection};

/// The item a folder model hands to its filter or sorters.
pub(super) fn as_item(object: &glib::Object) -> &FileItem {
    object
        .downcast_ref::<FileItem>()
        .expect("folder models hold FileItems")
}

/// Compares two items by one column, without folders-first or tie-breaks.
/// Sizes that are not known, and folders never measured, count as 0, as
/// in app.js.
pub(super) fn compare_column(column: SortColumn, a: &FileItem, b: &FileItem) -> Ordering {
    match column {
        SortColumn::Name => a.sort_name().key.natural_cmp(b.sort_name().key),
        SortColumn::Modified => a.entry().modified.cmp(&b.entry().modified),
        SortColumn::FolderPath => a.folder_path().key.natural_cmp(&b.folder_path().key),
        SortColumn::OriginalLocation => {
            let (a, b) = (a.original_location(), b.original_location());
            a.key.natural_cmp(&b.key)
        }
        SortColumn::Deleted => a.entry().trash_deletion_date.cmp(&b.entry().trash_deletion_date),
        SortColumn::Type => a.type_sort_key().natural_cmp(b.type_sort_key()),
        SortColumn::Size => a.sort_size().cmp(&b.sort_size()),
        SortColumn::Created => SortRole::Created.compare(a, b),
        SortColumn::Extension => SortRole::Extension.compare(a, b),
        SortColumn::Owner => SortRole::Owner.compare(a, b),
        SortColumn::Permissions => SortRole::Permissions.compare(a, b),
    }
}

/// The sorter a details column uses; the column view applies the
/// direction.
pub(crate) fn column_sorter(column: SortColumn) -> gtk::CustomSorter {
    gtk::CustomSorter::new(move |a, b| {
        let order = compare_column(column, as_item(a), as_item(b));
        order.into()
    })
}

/// How the model sorts beyond the details view's column: folders first or
/// not, a further sort key (VIEW-019) and groups (VIEW-022).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct SortOptions {
    pub(super) folders_first: bool,
    pub(super) hidden_last: bool,
    pub(super) role: Option<(SortRole, SortDirection)>,
    pub(super) grouping: Option<(SortState, GroupClock)>,
}

impl Default for SortOptions {
    fn default() -> Self {
        Self {
            folders_first: true,
            hidden_last: false,
            role: None,
            grouping: None,
        }
    }
}

/// Shared with the sorters' callbacks, which GTK calls with no access to
/// the model.
pub(super) type SharedOptions = Rc<Cell<SortOptions>>;

/// Sorts folders before files, whichever way the column sorts, while
/// folders come first.
pub(super) fn folders_first(options: &SharedOptions) -> gtk::CustomSorter {
    let options = Rc::clone(options);
    gtk::CustomSorter::new(move |a, b| {
        let options = options.get();
        let (a, b) = (as_item(a).entry(), as_item(b).entry());
        let folders = if options.folders_first {
            b.is_dir.cmp(&a.is_dir)
        } else {
            Ordering::Equal
        };
        let hidden = if options.hidden_last {
            a.is_hidden.cmp(&b.is_hidden)
        } else {
            Ordering::Equal
        };
        folders.then(hidden).into()
    })
}

/// Sorts by the further key while one is chosen; the column view is then
/// unsorted.
pub(super) fn role_sorter(options: &SharedOptions) -> gtk::CustomSorter {
    let options = Rc::clone(options);
    gtk::CustomSorter::new(move |a, b| {
        let Some((role, direction)) = options.get().role else {
            return gtk::Ordering::Equal;
        };
        let order = role.compare(as_item(a), as_item(b));
        directed(order, direction).into()
    })
}

/// Sorts the items into their groups, in the order the key sorts.
pub(super) fn group_sorter(options: &SharedOptions) -> gtk::CustomSorter {
    let options = Rc::clone(options);
    gtk::CustomSorter::new(move |a, b| {
        let Some((state, clock)) = options.get().grouping else {
            return gtk::Ordering::Equal;
        };
        let a = groups::group_of(state.by, as_item(a), &clock);
        let b = groups::group_of(state.by, as_item(b), &clock);
        directed(a.compare(&b), state.direction).into()
    })
}

/// `order` for an ascending sort, reversed for a descending one.
fn directed(order: Ordering, direction: SortDirection) -> Ordering {
    match direction {
        SortDirection::Ascending => order,
        SortDirection::Descending => order.reverse(),
    }
}

/// Breaks ties by name, always ascending.
pub(super) fn names_ascending() -> gtk::CustomSorter {
    gtk::CustomSorter::new(|a, b| {
        let order = sorting::compare_names(as_item(a).sort_name(), as_item(b).sort_name());
        order.into()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::file_entry;

    /// Sizes sort by value; folders never measured and files of unknown
    /// size count as 0.
    ///
    /// parity: VIEW-015
    #[gtk::test]
    fn unmeasured_folders_and_unknown_sizes_sort_as_nothing() {
        let mut big = file_entry("big.bin");
        big.size = Some(10);
        let big = FileItem::new(big);
        let folder = FileItem::new(crate::test_support::folder_entry("Photos"));
        let unknown = FileItem::new(file_entry("unknown.bin"));
        assert_eq!(compare_column(SortColumn::Size, &folder, &big), Ordering::Less);
        assert_eq!(compare_column(SortColumn::Size, &unknown, &big), Ordering::Less);
        assert_eq!(
            compare_column(SortColumn::Size, &folder, &unknown),
            Ordering::Equal
        );
    }

    /// Hidden-last remains independent of the selected sort direction.
    ///
    /// parity: VIEW-017, VIEW-021
    #[gtk::test]
    fn hidden_items_sort_after_visible_items_when_requested() {
        let options = SharedOptions::new(Cell::new(SortOptions {
            hidden_last: true,
            ..SortOptions::default()
        }));
        let sorter = folders_first(&options);
        let mut hidden = file_entry(".hidden");
        hidden.is_hidden = true;
        let hidden = FileItem::new(hidden);
        let visible = FileItem::new(file_entry("visible"));
        assert_eq!(sorter.compare(&hidden, &visible), gtk::Ordering::Larger);
    }
}
