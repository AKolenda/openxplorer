// SPDX-License-Identifier: AGPL-3.0-only
//! The details view: Name, Date modified, Type and Size columns.
//!
//! Matches the `.column-head` / `.file-row` grid in style.css (Name takes
//! the remaining width; 152, 135 and 78 pixel columns) and `renderRows` in
//! app.js. Columns are sortable by clicking their headers and resizable;
//! sizes are right-aligned.

use std::rc::Rc;

use gtk::prelude::*;
use ox_core::format;

use crate::folder_view::cells::{self, CellOwners, IconCells};
use crate::folder_view::model::{self, FolderModel};
use crate::folder_view::sorting::SortColumn;

/// Icon size in details rows.
const ROW_ICON: i32 = 21;

/// Fixed widths of the non-name columns, as in style.css.
fn fixed_width(column: SortColumn) -> Option<i32> {
    match column {
        SortColumn::Name => None,
        SortColumn::Modified => Some(152),
        SortColumn::Type => Some(135),
        SortColumn::Size => Some(90),
    }
}

/// The text of a non-name cell.
fn cell_text(column: SortColumn, entry: &ox_core::entry::Entry) -> String {
    match column {
        SortColumn::Name => entry.name.clone(),
        SortColumn::Modified => format::date_text(entry.modified),
        SortColumn::Type => entry.type_label.clone(),
        SortColumn::Size => match (entry.is_dir, entry.size) {
            (false, Some(size)) => format::pretty_bytes(size),
            _ => String::new(),
        },
    }
}

fn name_factory(icons: &Rc<IconCells>, owners: &Rc<CellOwners>) -> gtk::SignalListItemFactory {
    let factory = gtk::SignalListItemFactory::new();
    let registry = Rc::clone(owners);
    factory.connect_setup(move |_, object| {
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 11);
        let image = gtk::Image::new();
        image.set_pixel_size(ROW_ICON);
        let label = cells::cell_label(false);
        label.set_hexpand(true);
        cells::tooltip_when_truncated(&label);
        row.append(&image);
        row.append(&label);
        let list_item = cells::list_item(object);
        list_item.set_child(Some(&row));
        registry.register(&row, list_item);
    });
    let binder = Rc::clone(icons);
    factory.connect_bind(move |_, object| {
        let list_item = cells::list_item(object);
        let (Some(item), Some(row)) = (cells::bound_item(list_item), list_item.child()) else {
            return;
        };
        let image = row.first_child().and_downcast::<gtk::Image>();
        let label = image
            .as_ref()
            .and_then(|image| image.next_sibling())
            .and_downcast::<gtk::Label>();
        if let (Some(image), Some(label)) = (image, label) {
            binder.bind(&image, &item, ROW_ICON);
            label.set_text(&item.entry().name);
        }
    });
    let binder = Rc::clone(icons);
    factory.connect_unbind(move |_, object| {
        let image = cells::list_item(object)
            .child()
            .and_then(|row| row.first_child())
            .and_downcast::<gtk::Image>();
        if let Some(image) = image {
            binder.unbind(&image);
        }
    });
    factory
}

fn text_factory(column: SortColumn, owners: &Rc<CellOwners>) -> gtk::SignalListItemFactory {
    let factory = gtk::SignalListItemFactory::new();
    let registry = Rc::clone(owners);
    factory.connect_setup(move |_, object| {
        let label = cells::cell_label(true);
        if column == SortColumn::Size {
            label.set_xalign(1.0);
        }
        let list_item = cells::list_item(object);
        list_item.set_child(Some(&label));
        registry.register(&label, list_item);
    });
    factory.connect_bind(move |_, object| {
        let list_item = cells::list_item(object);
        let label = list_item.child().and_downcast::<gtk::Label>();
        if let (Some(item), Some(label)) = (cells::bound_item(list_item), label) {
            label.set_text(&cell_text(column, item.entry()));
        }
    });
    factory
}

/// Builds the column view over `model` and completes the model's sorter.
pub fn build(model: &FolderModel, icons: &Rc<IconCells>, owners: &Rc<CellOwners>) -> gtk::ColumnView {
    let view = gtk::ColumnView::new(Some(model.selection().clone()));
    view.add_css_class("files");
    view.set_enable_rubberband(true);
    view.set_show_row_separators(false);
    view.set_show_column_separators(false);
    view.set_reorderable(false);
    view.set_tab_behavior(gtk::ListTabBehavior::Item);
    for column in SortColumn::ALL {
        let factory = match column {
            SortColumn::Name => name_factory(icons, owners),
            other => text_factory(other, owners),
        };
        let view_column = gtk::ColumnViewColumn::new(Some(column.label()), Some(factory));
        view_column.set_id(Some(column.key()));
        view_column.set_resizable(true);
        view_column.set_sorter(Some(&model::column_sorter(column)));
        match fixed_width(column) {
            Some(width) => view_column.set_fixed_width(width),
            None => view_column.set_expand(true),
        }
        view.append_column(&view_column);
    }
    if let Some(sorter) = view.sorter() {
        model.attach_column_sorter(&sorter);
    }
    sort_by(&view, SortColumn::Name, false);
    view
}

/// The column view's column for `column`.
fn view_column(view: &gtk::ColumnView, column: SortColumn) -> Option<gtk::ColumnViewColumn> {
    let columns = view.columns();
    (0..columns.n_items())
        .filter_map(|index| columns.item(index).and_downcast::<gtk::ColumnViewColumn>())
        .find(|candidate| candidate.id().as_deref() == Some(column.key()))
}

/// Sorts by `column` in the given direction.
pub fn sort_by(view: &gtk::ColumnView, column: SortColumn, descending: bool) {
    let order = if descending {
        gtk::SortType::Descending
    } else {
        gtk::SortType::Ascending
    };
    view.sort_by_column(view_column(view, column).as_ref(), order);
}

/// The current sort column and direction (Name ascending when unsorted).
pub fn current_sort(view: &gtk::ColumnView) -> (SortColumn, bool) {
    let sorter = view.sorter().and_downcast::<gtk::ColumnViewSorter>();
    let Some(sorter) = sorter else {
        return (SortColumn::Name, false);
    };
    let column = sorter
        .primary_sort_column()
        .and_then(|column| column.id())
        .and_then(|id| SortColumn::from_key(&id))
        .unwrap_or(SortColumn::Name);
    let descending = sorter.primary_sort_order() == gtk::SortType::Descending;
    (column, descending)
}
