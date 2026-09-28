// SPDX-License-Identifier: AGPL-3.0-only
//! The details view: Name, Date modified, Type and Size columns.
//!
//! Matches the `.column-head` / `.file-row` grid in `desktop/ui/style.css`
//! and `renderRows` / `applyColumnLayout` in `desktop/ui/app.js`: Name
//! takes the remaining width until the user resizes it, the other columns
//! default to 152, 135 and 90 pixels, and saved widths are clamped to the
//! limits the Python app uses. Columns sort by clicking their headers;
//! sizes and the Size title are right-aligned.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use gtk::glib;
use gtk::prelude::*;
use ox_core::format;
use ox_core::settings::{Column, ColumnWidths};

use crate::folder_view::cells::{self, CellLayout, CellOwners, IconCells};
use crate::folder_view::column_titles;
use crate::folder_view::model::{self, FolderModel};
use crate::folder_view::sorting::{SortColumn, SortDirection};

/// Icon size in details rows.
const ROW_ICON: i32 = 21;

/// How long column widths must stay unchanged before they are saved, so a
/// drag saves once instead of on every pixel.
const RESIZE_SETTLE: Duration = Duration::from_millis(500);

/// The settings column of a details column.
pub(crate) const fn settings_column(column: SortColumn) -> Column {
    match column {
        SortColumn::Name => Column::Name,
        SortColumn::Modified => Column::Modified,
        SortColumn::Type => Column::Type,
        SortColumn::Size => Column::Size,
    }
}

/// The web list pads its column header and rows 14 pixels at both ends
/// (`.column-head{padding:0 14px}`), so its columns stop short of the
/// list's edges. GTK lays columns out across the whole column view, so
/// the first and last columns, Name and Size, hold those pixels: they are
/// this much wider than the widths saved in settings, and their titles and
/// cells pad for it (resources/style.css).
const EDGE_GUTTER: u32 = 14;

/// The part of `column`'s width that is the list's end padding.
const fn edge_gutter(column: SortColumn) -> u32 {
    match column {
        SortColumn::Name | SortColumn::Size => EDGE_GUTTER,
        SortColumn::Modified | SortColumn::Type => 0,
    }
}

/// Width of a column nobody resized (`columnDefaults` in app.js). Name has
/// none: it takes the remaining space.
const fn default_width(column: SortColumn) -> Option<u32> {
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
    cells::connect_file_cells(&factory, CellLayout::DetailsRow, ROW_ICON, icons, owners);
    factory
}

fn text_factory(column: SortColumn, owners: &Rc<CellOwners>) -> gtk::SignalListItemFactory {
    let factory = gtk::SignalListItemFactory::new();
    let registry = Rc::clone(owners);
    factory.connect_setup(move |_, object| {
        let label = cells::dim_cell_label();
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
/// The view shows no model until [`FolderModel::attach`] picks it.
pub(crate) fn build(model: &FolderModel, icons: &Rc<IconCells>, owners: &Rc<CellOwners>) -> gtk::ColumnView {
    let view = gtk::ColumnView::new(None::<gtk::MultiSelection>);
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
        view.append_column(&view_column);
    }
    apply_column_widths(&view, None);
    if let Some(sorter) = view.sorter() {
        model.attach_column_sorter(&sorter);
    }
    column_titles::style_titles(&view);
    sort_by(&view, SortColumn::Name, SortDirection::Ascending);
    view
}

/// The column view's column for `column`.
pub(crate) fn view_column(view: &gtk::ColumnView, column: SortColumn) -> Option<gtk::ColumnViewColumn> {
    let columns = view.columns();
    (0..columns.n_items())
        .filter_map(|index| columns.item(index).and_downcast::<gtk::ColumnViewColumn>())
        .find(|candidate| candidate.id().as_deref() == Some(column.key()))
}

/// The width `column` starts with: the saved width within the Python
/// app's limits, else its default. `None` lets Name fill the space.
fn start_width(column: SortColumn, saved: Option<&ColumnWidths>) -> Option<u32> {
    let limits = settings_column(column).width_range();
    let saved = saved.and_then(|widths| widths.get(settings_column(column)));
    let width = saved.or(default_width(column))?;
    Some(width.clamp(*limits.start(), *limits.end()))
}

/// Applies saved column widths. Name keeps expanding until the user saved
/// a width for it, as in `applyColumnLayout`.
pub(crate) fn apply_column_widths(view: &gtk::ColumnView, saved: Option<&ColumnWidths>) {
    for column in SortColumn::ALL {
        let Some(view_column) = view_column(view, column) else {
            continue;
        };
        let width = start_width(column, saved);
        view_column.set_expand(width.is_none());
        let with_gutter = width.map(|width| width + edge_gutter(column));
        let pixels = with_gutter
            .and_then(|width| i32::try_from(width).ok())
            .unwrap_or(-1);
        view_column.set_fixed_width(pixels);
    }
}

/// The width settings save for a column `fixed_width` pixels wide, or
/// `None` while it has no width of its own.
fn saved_width(column: SortColumn, fixed_width: i32) -> Option<f64> {
    let width = u32::try_from(fixed_width).ok().filter(|width| *width > 0)?;
    let without_gutter = width.saturating_sub(edge_gutter(column));
    Some(f64::from(without_gutter))
}

/// The widths the user set, in the form settings save them. Name counts
/// only once it has a width of its own.
fn column_widths(view: &gtk::ColumnView) -> Vec<(Column, f64)> {
    SortColumn::ALL
        .into_iter()
        .filter_map(|column| {
            let fixed_width = view_column(view, column)?.fixed_width();
            let width = saved_width(column, fixed_width)?;
            Some((settings_column(column), width))
        })
        .collect()
}

/// Calls `on_resized` with every column width once a resize settles.
pub(crate) fn connect_columns_resized(
    view: &gtk::ColumnView,
    on_resized: impl Fn(Vec<(Column, f64)>) + 'static,
) {
    let on_resized = Rc::new(on_resized);
    let timer: Rc<RefCell<Option<glib::SourceId>>> = Rc::new(RefCell::new(None));
    for column in SortColumn::ALL {
        let Some(view_column) = view_column(view, column) else {
            continue;
        };
        let weak_view = view.downgrade();
        let on_resized = Rc::clone(&on_resized);
        let timer = Rc::clone(&timer);
        view_column.connect_fixed_width_notify(move |_| {
            if let Some(pending) = timer.borrow_mut().take() {
                pending.remove();
            }
            let weak_view = weak_view.clone();
            let on_resized = Rc::clone(&on_resized);
            let slot = Rc::clone(&timer);
            let source = glib::timeout_add_local_once(RESIZE_SETTLE, move || {
                slot.borrow_mut().take();
                if let Some(view) = weak_view.upgrade() {
                    on_resized(column_widths(&view));
                }
            });
            timer.borrow_mut().replace(source);
        });
    }
}

/// Shows or hides the Date modified and Type columns, which a compact
/// window has no room for (the 680-pixel rules in `style.css`).
pub(crate) fn show_date_and_type(view: &gtk::ColumnView, shown: bool) {
    for column in [SortColumn::Modified, SortColumn::Type] {
        if let Some(view_column) = view_column(view, column) {
            view_column.set_visible(shown);
        }
    }
}

/// Sorts by `column` in `direction`.
pub(crate) fn sort_by(view: &gtk::ColumnView, column: SortColumn, direction: SortDirection) {
    let (current, _) = current_sort(view);
    if current != column {
        // GTK updates the sort indicator only on the column it sorts by
        // now, so the previously sorted title would keep a stale
        // `ascending` or `descending` class. Clearing the sorter first
        // resets it.
        view.sort_by_column(None, direction.to_sort_type());
    }
    view.sort_by_column(view_column(view, column).as_ref(), direction.to_sort_type());
}

/// The current sort column and direction (Name ascending when unsorted).
pub(crate) fn current_sort(view: &gtk::ColumnView) -> (SortColumn, SortDirection) {
    let sorter = view.sorter().and_downcast::<gtk::ColumnViewSorter>();
    let Some(sorter) = sorter else {
        return (SortColumn::Name, SortDirection::Ascending);
    };
    let column = sorter
        .primary_sort_column()
        .and_then(|column| column.id())
        .and_then(|id| SortColumn::from_key(&id))
        .unwrap_or(SortColumn::Name);
    let direction = SortDirection::from_sort_type(sorter.primary_sort_order());
    (column, direction)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unsaved_columns_start_at_the_python_defaults() {
        assert_eq!(start_width(SortColumn::Name, None), None);
        assert_eq!(start_width(SortColumn::Modified, None), Some(152));
        assert_eq!(start_width(SortColumn::Type, None), Some(135));
        assert_eq!(start_width(SortColumn::Size, None), Some(90));
    }

    #[test]
    fn saved_widths_are_used_within_their_limits() {
        let saved = ColumnWidths {
            name: Some(300),
            modified: Some(40),
            size: Some(5000),
            ..ColumnWidths::default()
        };
        assert_eq!(start_width(SortColumn::Name, Some(&saved)), Some(300));
        assert_eq!(start_width(SortColumn::Modified, Some(&saved)), Some(100));
        assert_eq!(start_width(SortColumn::Size, Some(&saved)), Some(600));
        assert_eq!(start_width(SortColumn::Type, Some(&saved)), Some(135));
    }

    #[test]
    fn saved_widths_leave_out_the_end_columns_padding() {
        assert_eq!(saved_width(SortColumn::Size, 104), Some(90.0));
        assert_eq!(saved_width(SortColumn::Name, 314), Some(300.0));
        assert_eq!(saved_width(SortColumn::Type, 135), Some(135.0));
        assert_eq!(
            saved_width(SortColumn::Name, -1),
            None,
            "Name still fills the space"
        );
    }
}
