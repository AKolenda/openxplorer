// SPDX-License-Identifier: AGPL-3.0-only
//! Which details columns show, and in what order (VIEW-033, VIEW-034).
//!
//! The header's menu shows and hides columns, as in Dolphin and Windows
//! Explorer, and titles are dragged to move their columns. Name stays
//! first unless Choose Details moved it down, as Windows lets it; Folder
//! path follows it in a search, and Original location and Date deleted in
//! the Recycle Bin, wherever it is. Only a Name that is not first is listed
//! among the chosen columns (and saved as `name`), so the columns saved
//! for a Name in front are the keys they always were. The view reports
//! the order the user dragged the columns into ([`COLUMNS_CHOSEN`]) so the
//! window can save it with the shown columns.

use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;
use ox_core::settings::DEFAULT_DETAILS_COLUMNS;

use super::{DetailsView, COLUMNS_CHOSEN};
use crate::folder_view::sorting::SortColumn;

/// The columns a new installation shows after Name.
pub(super) fn default_chosen() -> Vec<SortColumn> {
    chosen_from_keys(DEFAULT_DETAILS_COLUMNS)
}

/// The choosable columns among the column `keys`, and Name where it is
/// not first, in their order.
pub(crate) fn chosen_from_keys(keys: impl IntoIterator<Item = impl AsRef<str>>) -> Vec<SortColumn> {
    let columns = keys
        .into_iter()
        .filter_map(|key| SortColumn::from_key(key.as_ref()));
    let chosen = columns
        .filter(|column| *column == SortColumn::Name || SortColumn::CHOOSABLE.contains(column))
        .collect();
    without_leading_name(chosen)
}

/// `chosen` without Name when it comes first, where Name goes anyway.
pub(crate) fn without_leading_name(mut chosen: Vec<SortColumn>) -> Vec<SortColumn> {
    if chosen.first() == Some(&SortColumn::Name) {
        chosen.remove(0);
    }
    chosen
}

/// The columns a listing always shows right after Name: Folder path in a
/// search, Original location and Date deleted in the Recycle Bin, as
/// Windows File Explorer shows them.
pub(crate) const FOLLOWING_NAME: [SortColumn; 3] = [
    SortColumn::FolderPath,
    SortColumn::OriginalLocation,
    SortColumn::Deleted,
];

/// Every column in the order the view lays them out: Name (first unless
/// `chosen` places it) with the listings' own columns after it, the
/// `chosen` ones, then the hidden ones.
fn column_layout(chosen: &[SortColumn]) -> Vec<SortColumn> {
    let mut layout = Vec::new();
    if !chosen.contains(&SortColumn::Name) {
        layout.push(SortColumn::Name);
        layout.extend(FOLLOWING_NAME);
    }
    for column in chosen {
        layout.push(*column);
        if *column == SortColumn::Name {
            layout.extend(FOLLOWING_NAME);
        }
    }
    let rest = SortColumn::ALL
        .into_iter()
        .filter(|column| !layout.contains(column));
    layout.extend(rest.collect::<Vec<_>>());
    layout
}

impl DetailsView {
    /// The columns shown, in their order, Name among them only where
    /// Choose Details moved it down.
    pub(crate) fn chosen_columns(&self) -> Vec<SortColumn> {
        self.imp().chosen.borrow().clone()
    }

    /// Shows the `chosen` columns in that order, after Name unless they
    /// place it, and hides the other choosable ones.
    pub(crate) fn show_chosen_columns(&self, chosen: Vec<SortColumn>) {
        let chosen = without_leading_name(chosen);
        if *self.imp().chosen.borrow() == chosen {
            return;
        }
        self.imp().chosen.replace(chosen);
        self.arrange_columns();
        self.show_fitting_columns();
    }

    /// Calls `on_chosen` with the shown columns' new order after the user
    /// dragged a title.
    pub(crate) fn connect_columns_chosen(
        &self,
        on_chosen: impl Fn(Vec<SortColumn>) + 'static,
    ) -> glib::SignalHandlerId {
        self.connect_closure(
            COLUMNS_CHOSEN,
            false,
            glib::closure_local!(move |view: DetailsView| on_chosen(view.chosen_columns())),
        )
    }

    /// The columns in the order the view lays them out now.
    pub(crate) fn column_order(&self) -> Vec<SortColumn> {
        let columns = self.column_view().columns();
        let ids = columns.iter::<gtk::ColumnViewColumn>().filter_map(Result::ok);
        ids.filter_map(|column| SortColumn::from_key(&column.id()?))
            .collect()
    }

    /// Puts the columns in the order of [`column_layout`].
    fn arrange_columns(&self) {
        let imp = self.imp();
        let was_arranging = imp.arranging.replace(true);
        let layout = column_layout(&imp.chosen.borrow());
        for (position, column) in (0_u32..).zip(layout) {
            let Some(view_column) = self.column(column) else {
                continue;
            };
            let current = self.column_order().iter().position(|shown| *shown == column);
            if current != Some(position as usize) {
                self.column_view().insert_column(position, &view_column);
            }
        }
        imp.arranging.set(was_arranging);
    }

    /// Follows the user dragging titles: the new order of the chosen
    /// columns is kept and reported, and Name returns to the front.
    pub(super) fn follow_column_drags(&self) {
        let columns = self.column_view().columns();
        columns.connect_items_changed(glib::clone!(
            #[weak(rename_to = view)]
            self,
            move |_, _, _, _| {
                if view.imp().arranging.get() {
                    return;
                }
                // GTK moves a column by removing and inserting it: the
                // order is read once both happened.
                glib::idle_add_local_once(glib::clone!(
                    #[weak]
                    view,
                    move || view.column_dragged()
                ));
            }
        ));
    }

    /// A title was dragged: keeps the chosen columns in their new order.
    /// A Name Choose Details placed keeps its place among them.
    fn column_dragged(&self) {
        let chosen = self.chosen_columns();
        let order = self.column_order();
        let dragged: Vec<SortColumn> = order
            .into_iter()
            .filter(|column| chosen.contains(column))
            .collect();
        let dragged = without_leading_name(dragged);
        self.arrange_columns_as(dragged.clone());
        if dragged != chosen {
            self.emit_by_name::<()>(COLUMNS_CHOSEN, &[]);
        }
    }

    /// Keeps `chosen` and lays the columns out for it.
    fn arrange_columns_as(&self, chosen: Vec<SortColumn>) {
        self.imp().chosen.replace(chosen);
        self.arrange_columns();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::folder_view::cells::CellOwners;
    use crate::folder_view::model::FolderModel;

    /// A dragged column keeps its new place, Name stays first, and the
    /// view reports the new order.
    ///
    /// parity: VIEW-034
    #[gtk::test]
    fn dragged_columns_keep_their_order_behind_name() {
        let view = DetailsView::new(&FolderModel::new(), &CellOwners::new());
        let reported = std::rc::Rc::new(std::cell::RefCell::new(None));
        let sink = std::rc::Rc::clone(&reported);
        view.connect_columns_chosen(move |chosen| {
            sink.replace(Some(chosen));
        });
        let size = view.column(SortColumn::Size).expect("a Size column");

        // As GTK does when a title is dropped at the front.
        view.column_view().insert_column(0, &size);
        crate::test_support::harness::wait_until("the drag to be read", || reported.borrow().is_some());

        let expected = [SortColumn::Size, SortColumn::Modified, SortColumn::Type];
        assert_eq!(reported.borrow().as_deref(), Some(&expected[..]));
        assert_eq!(
            view.column_order()[..5],
            [
                SortColumn::Name,
                SortColumn::FolderPath,
                SortColumn::OriginalLocation,
                SortColumn::Deleted,
                SortColumn::Size
            ]
        );
    }
}
