// SPDX-License-Identifier: AGPL-3.0-only
//! The details view: Name, Date modified, Type and Size columns.
//!
//! Matches the `.column-head` / `.file-row` grid in `desktop/ui/style.css`
//! and `renderRows` / `applyColumnLayout` in `desktop/ui/app.js`. Columns
//! start at the widths [`column_widths`] works out and sort by clicking
//! their headers; sizes and the Size title are right-aligned.
//! [`DetailsView`] is the widget; it keeps its titles' sort arrows in step
//! with the sort order and reports the column widths once a resize
//! settles.

use std::rc::Rc;
use std::time::Duration;

use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;
use ox_core::format;
use ox_core::settings::{ColumnWidth, ColumnWidths};

use crate::folder_view::cells::{self, CellLayout, CellOwners};
use crate::folder_view::column_titles;
use crate::folder_view::column_widths;
use crate::folder_view::item::FileItem;
use crate::folder_view::model::{self, FolderModel};
use crate::folder_view::sorting::{SortColumn, SortDirection, SortOrder};

/// Icon edge in details rows (`.name-cell svg{height:21px}`).
const ROW_ICON_SIZE: i32 = 21;

/// How long column widths must stay unchanged before they are saved, so a
/// drag saves once instead of on every pixel.
const RESIZE_SETTLE: Duration = Duration::from_millis(500);

/// The signal a [`DetailsView`] emits once the column widths have stayed
/// unchanged for [`RESIZE_SETTLE`].
const COLUMNS_RESIZED: &str = "columns-resized";

/// The text `column` shows for `item`. Folders and files of unknown size
/// have an empty Size cell.
fn cell_text(column: SortColumn, item: &FileItem) -> String {
    let entry = item.entry();
    match column {
        SortColumn::Name => entry.name.clone(),
        SortColumn::Modified => format::date_text(entry.modified),
        SortColumn::Type => entry.type_label.clone(),
        SortColumn::Size => item.file_size().map(format::pretty_bytes).unwrap_or_default(),
    }
}

/// The Name column's cells: the item's icon beside its name.
fn name_factory(owners: &Rc<CellOwners>) -> gtk::SignalListItemFactory {
    let factory = gtk::SignalListItemFactory::new();
    cells::connect_file_cells(&factory, CellLayout::DetailsRow, ROW_ICON_SIZE, owners);
    factory
}

/// The cells of the Date modified, Type or Size column: one dim label,
/// registered in `owners`, which dims the cells of cut items.
fn text_factory(column: SortColumn, owners: &Rc<CellOwners>) -> gtk::SignalListItemFactory {
    let factory = gtk::SignalListItemFactory::new();
    let setup_owners = Rc::clone(owners);
    factory.connect_setup(move |_, object| {
        let label = cells::dim_cell_label();
        if column == SortColumn::Size {
            // `.file-row .size-cell{text-align:right}`
            label.set_xalign(1.0);
        }
        let list_item = cells::as_list_item(object);
        list_item.set_child(Some(&label));
        setup_owners.register(&label, list_item);
    });
    let bind_owners = Rc::clone(owners);
    factory.connect_bind(move |_, object| {
        let list_item = cells::as_list_item(object);
        let label = list_item.child().and_downcast::<gtk::Label>();
        if let (Some(item), Some(label)) = (cells::bound_item(list_item), label) {
            label.set_text(&cell_text(column, &item));
            bind_owners.style_for_cut(&label, &item);
        }
    });
    factory
}

/// A resizable column showing `column`, sorted by its header.
fn new_view_column(column: SortColumn, owners: &Rc<CellOwners>) -> gtk::ColumnViewColumn {
    let factory = match column {
        SortColumn::Name => name_factory(owners),
        SortColumn::Modified | SortColumn::Type | SortColumn::Size => text_factory(column, owners),
    };
    let view_column = gtk::ColumnViewColumn::new(Some(column.label()), Some(factory));
    view_column.set_id(Some(column.as_str()));
    view_column.set_resizable(true);
    view_column.set_sorter(Some(&model::column_sorter(column)));
    view_column
}

/// The details columns a window has room for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DetailsColumns {
    /// Name, Date modified, Type and Size.
    All,
    /// Name and Size only, in a compact window (the 680-pixel rules in
    /// `style.css` hide Date modified and Type).
    NameAndSize,
}

impl DetailsColumns {
    /// True when `column` is one of these columns.
    const fn contains(self, column: SortColumn) -> bool {
        match self {
            DetailsColumns::All => true,
            DetailsColumns::NameAndSize => matches!(column, SortColumn::Name | SortColumn::Size),
        }
    }
}

mod imp {
    use std::cell::{OnceCell, RefCell};
    use std::sync::OnceLock;

    use gtk::glib;
    use gtk::glib::subclass::Signal;
    use gtk::prelude::*;
    use gtk::subclass::prelude::*;

    use super::COLUMNS_RESIZED;

    /// Private state of [`super::DetailsView`].
    #[derive(Debug, Default)]
    pub(crate) struct DetailsView {
        /// Scrolls the column view; the view's only child. The column view
        /// must be the scroller's direct child: GTK then builds rows only
        /// for the part of the list on screen.
        pub(super) scroller: gtk::ScrolledWindow,
        /// The rows and their column titles.
        pub(super) column_view: gtk::ColumnView,
        /// The pending report of settled column widths, restarted by every
        /// width change. A `RefCell`, as a `Cell` of a type that is not
        /// `Copy` cannot be debug-printed.
        pub(super) resize_timer: RefCell<Option<glib::SourceId>>,
        /// The column titles' sort arrows, in column order; set by
        /// [`super::DetailsView::new`].
        pub(super) carets: OnceCell<Vec<gtk::Image>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for DetailsView {
        const NAME: &'static str = "OxDetailsView";
        type Type = super::DetailsView;
        type ParentType = gtk::Widget;

        fn class_init(klass: &mut Self::Class) {
            // `GtkColumnView` cannot be subclassed, so the view wraps its
            // scroller and gives it all of its own size.
            klass.set_layout_manager_type::<gtk::BinLayout>();
        }
    }

    impl ObjectImpl for DetailsView {
        fn signals() -> &'static [Signal] {
            static SIGNALS: OnceLock<Vec<Signal>> = OnceLock::new();
            SIGNALS.get_or_init(|| vec![Signal::builder(COLUMNS_RESIZED).build()])
        }

        fn constructed(&self) {
            self.parent_constructed();
            let column_view = &self.column_view;
            column_view.add_css_class("files");
            column_view.set_enable_rubberband(true);
            column_view.set_show_row_separators(false);
            column_view.set_show_column_separators(false);
            column_view.set_reorderable(false);
            column_view.set_tab_behavior(gtk::ListTabBehavior::Item);
            self.scroller.set_child(Some(column_view));
            self.scroller.set_parent(&*self.obj());
        }

        fn dispose(&self) {
            self.obj().cancel_resize_report();
            self.scroller.unparent();
        }
    }

    impl WidgetImpl for DetailsView {}
}

glib::wrapper! {
    /// The details view: a column view of the folder's items in a
    /// scroller, sorted by its column titles.
    pub(crate) struct DetailsView(ObjectSubclass<imp::DetailsView>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl DetailsView {
    /// A details view over `model` whose cells are registered in
    /// `owners`. It completes the model's sorter and sorts by Name
    /// ascending, but shows no model until the window makes it the visible
    /// view.
    pub(crate) fn new(model: &FolderModel, owners: &Rc<CellOwners>) -> Self {
        let view: Self = glib::Object::new();
        let column_view = view.column_view();
        for column in SortColumn::ALL {
            column_view.append_column(&new_view_column(column, owners));
        }
        view.apply_column_widths(None);
        view.watch_column_widths();
        if let Some(sorter) = column_view.sorter() {
            model.attach_column_sorter(&sorter);
        }
        view.add_sort_carets();
        view.sort_by(SortOrder::DEFAULT);
        view
    }

    /// The column view, which holds the selection model, the sorter and
    /// the keyboard focus.
    pub(crate) fn column_view(&self) -> &gtk::ColumnView {
        &self.imp().column_view
    }

    /// The adjustment of the vertical scroll position.
    pub(crate) fn vadjustment(&self) -> gtk::Adjustment {
        self.imp().scroller.vadjustment()
    }

    /// The column view's column for `column`.
    pub(crate) fn column(&self, column: SortColumn) -> Option<gtk::ColumnViewColumn> {
        let columns = self.column_view().columns();
        (0..columns.n_items())
            .filter_map(|position| columns.item(position).and_downcast::<gtk::ColumnViewColumn>())
            .find(|candidate| candidate.id().as_deref() == Some(column.as_str()))
    }

    /// Each details column with the column view's column that shows it.
    fn view_columns(&self) -> impl Iterator<Item = (SortColumn, gtk::ColumnViewColumn)> + '_ {
        SortColumn::ALL
            .into_iter()
            .filter_map(|column| Some((column, self.column(column)?)))
    }

    /// Shows `columns` and hides the others.
    pub(crate) fn show_columns(&self, columns: DetailsColumns) {
        for (column, view_column) in self.view_columns() {
            view_column.set_visible(columns.contains(column));
        }
    }

    /// Applies saved column widths. Name keeps expanding until the user
    /// saved a width for it, as in `applyColumnLayout`.
    pub(crate) fn apply_column_widths(&self, saved: Option<&ColumnWidths>) {
        for (column, view_column) in self.view_columns() {
            let width = column_widths::start_width(column, saved);
            view_column.set_expand(width.is_none());
            view_column.set_fixed_width(column_widths::fixed_width(column, width));
        }
        // Applying widths is not a resize by the user: reporting it would
        // save widths the user never set.
        self.cancel_resize_report();
    }

    /// The widths the user set, in the form settings save them. Name
    /// counts only once it has a width of its own.
    fn widths_to_save(&self) -> Vec<ColumnWidth> {
        let widths = self.view_columns().filter_map(|(column, view_column)| {
            let pixels = column_widths::saved_width(column, view_column.fixed_width())?;
            let column = column_widths::settings_column(column);
            Some(ColumnWidth { column, pixels })
        });
        widths.collect()
    }

    /// Calls `on_resized` with every column width, in the form settings
    /// save them, once a resize has settled for [`RESIZE_SETTLE`].
    pub(crate) fn connect_columns_resized(
        &self,
        on_resized: impl Fn(Vec<ColumnWidth>) + 'static,
    ) -> glib::SignalHandlerId {
        self.connect_closure(
            COLUMNS_RESIZED,
            false,
            glib::closure_local!(move |view: DetailsView| on_resized(view.widths_to_save())),
        )
    }

    /// Restarts the wait for settled widths whenever a column's width
    /// changes, as it does while the user drags a title's edge.
    fn watch_column_widths(&self) {
        for (_, view_column) in self.view_columns() {
            view_column.connect_fixed_width_notify(glib::clone!(
                #[weak(rename_to = view)]
                self,
                move |_| view.restart_resize_timer()
            ));
        }
    }

    /// A column's width changed: reports the widths once they have stayed
    /// unchanged for [`RESIZE_SETTLE`].
    fn restart_resize_timer(&self) {
        self.cancel_resize_report();
        let report = glib::clone!(
            #[weak(rename_to = view)]
            self,
            move || view.report_settled_widths()
        );
        let timer = glib::timeout_add_local_once(RESIZE_SETTLE, report);
        self.imp().resize_timer.replace(Some(timer));
    }

    /// The widths have settled: tells the [`COLUMNS_RESIZED`] handlers.
    fn report_settled_widths(&self) {
        // The timer has fired; removing it again would be a GLib error.
        self.imp().resize_timer.take();
        self.emit_by_name::<()>(COLUMNS_RESIZED, &[]);
    }

    /// Drops the pending report of settled widths, if there is one.
    fn cancel_resize_report(&self) {
        if let Some(pending) = self.imp().resize_timer.take() {
            pending.remove();
        }
    }

    /// Sorts by `order`.
    pub(crate) fn sort_by(&self, order: SortOrder) {
        let column_view = self.column_view();
        let sort_type = order.direction.to_sort_type();
        if self.sort_order().column != order.column {
            // GTK updates the sort indicator only on the column it sorts by
            // now, so the previously sorted title would keep a stale
            // `ascending` or `descending` class. Clearing the sorter first
            // resets it.
            column_view.sort_by_column(None, sort_type);
        }
        column_view.sort_by_column(self.column(order.column).as_ref(), sort_type);
    }

    /// The column and direction the view sorts by (Name ascending while
    /// unsorted).
    pub(crate) fn sort_order(&self) -> SortOrder {
        self.primary_sort().unwrap_or(SortOrder::DEFAULT)
    }

    /// The column and direction the view sorts by, or `None` while
    /// unsorted.
    fn primary_sort(&self) -> Option<SortOrder> {
        let sorter = self.column_view().sorter();
        let sorter = sorter.and_downcast::<gtk::ColumnViewSorter>()?;
        let id = sorter.primary_sort_column()?.id()?;
        let column = SortColumn::from_key(&id)?;
        let direction = SortDirection::from_sort_type(sorter.primary_sort_order());
        Some(SortOrder { column, direction })
    }

    /// Gives the titles the current app's sort arrows, which follow the
    /// view's sorter from now on.
    fn add_sort_carets(&self) {
        let carets = column_titles::style_titles(self.column_view());
        self.imp()
            .carets
            .set(carets)
            .expect("DetailsView::new adds the carets once");
        self.show_sort_caret();
        let Some(sorter) = self.column_view().sorter() else {
            return;
        };
        sorter.connect_changed(glib::clone!(
            #[weak(rename_to = view)]
            self,
            move |_, _| view.show_sort_caret()
        ));
    }

    /// Shows the sorted column's arrow, pointing its way, and hides the
    /// others.
    fn show_sort_caret(&self) {
        if let Some(carets) = self.imp().carets.get() {
            column_titles::show_sort_caret(carets, self.primary_sort());
        }
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use ox_core::settings::Column;

    use super::*;
    use crate::test_support::harness::{wait_for, wait_until};

    /// A details view over an empty model, as a new window builds it.
    fn new_details_view() -> DetailsView {
        let model = FolderModel::new();
        DetailsView::new(&model, &CellOwners::new())
    }

    /// The last widths a view reported, `None` until it reports any.
    type ReportedWidths = Rc<RefCell<Option<Vec<ColumnWidth>>>>;

    /// The widths `view` reports once its next resize settles, filled in
    /// by its `columns-resized` handler.
    fn reported_widths(view: &DetailsView) -> ReportedWidths {
        let reported = ReportedWidths::default();
        let sink = Rc::clone(&reported);
        view.connect_columns_resized(move |widths| {
            sink.replace(Some(widths));
        });
        reported
    }

    #[test]
    fn a_compact_window_keeps_only_name_and_size() {
        let kept: Vec<SortColumn> = SortColumn::ALL
            .into_iter()
            .filter(|column| DetailsColumns::NameAndSize.contains(*column))
            .collect();
        assert_eq!(kept, [SortColumn::Name, SortColumn::Size]);
        let all_kept = SortColumn::ALL
            .into_iter()
            .all(|column| DetailsColumns::All.contains(column));
        assert!(all_kept, "a wider window shows every column");
    }

    /// parity: VIEW-014
    #[gtk::test]
    fn the_sort_order_reads_back_and_one_arrow_shows_it() {
        let view = new_details_view();
        assert_eq!(view.sort_order(), SortOrder::DEFAULT);
        let size_descending = SortOrder {
            column: SortColumn::Size,
            direction: SortDirection::Descending,
        };
        view.sort_by(size_descending);
        assert_eq!(view.sort_order(), size_descending);
        assert_eq!(
            column_titles::shown_carets(view.column_view()),
            [None, None, None, Some(SortDirection::Descending)]
        );
    }

    /// parity: VIEW-028
    #[gtk::test]
    fn a_settled_resize_reports_the_widths_settings_save() {
        let view = new_details_view();
        let reported = reported_widths(&view);
        let type_column = view.column(SortColumn::Type).expect("a Type column");
        type_column.set_fixed_width(200);
        wait_until("the resize to settle", || reported.borrow().is_some());
        let expected = [
            ColumnWidth {
                column: Column::Modified,
                pixels: 152.0,
            },
            ColumnWidth {
                column: Column::Type,
                pixels: 200.0,
            },
            ColumnWidth {
                column: Column::Size,
                pixels: 90.0,
            },
        ];
        assert_eq!(
            reported.take().as_deref(),
            Some(&expected[..]),
            "Name fills the space, so it has no width to save"
        );
    }

    /// parity: VIEW-028
    #[gtk::test]
    fn applying_saved_widths_is_not_reported_as_a_resize() {
        let view = new_details_view();
        let reported = reported_widths(&view);
        let saved = ColumnWidths {
            name: Some(300),
            ..ColumnWidths::default()
        };
        view.apply_column_widths(Some(&saved));
        wait_for(RESIZE_SETTLE * 2);
        assert_eq!(*reported.borrow(), None, "nothing was resized");
    }
}
