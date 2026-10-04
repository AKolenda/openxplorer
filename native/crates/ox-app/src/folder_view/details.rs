// SPDX-License-Identifier: AGPL-3.0-only
//! The details view: Name, Date modified, Type and Size columns, with
//! Folder path in place of Date modified while searching, and the columns
//! the user adds from the header's menu ([`column_choice`]).
//!
//! Matches the `.column-head` / `.file-row` grid in `v2.0.0:desktop/ui/style.css`
//! and `renderRows` / `applyColumnLayout` in `v2.0.0:desktop/ui/app.js`. Columns
//! start at the widths [`column_widths`] works out and sort by clicking
//! their headers; sizes and the Size title are right-aligned.
//! [`DetailsView`] is the widget; it keeps its titles' sort arrows in step
//! with the sort order and reports the column widths once a resize
//! settles. [`column_fit`] fits and nudges columns from their titles;
//! [`column_text`] says what each column shows.

mod column_choice;
mod column_fit;
pub(crate) mod column_text;
mod group_headers;

pub(crate) use group_headers::GroupTitle;

use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;

use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;
use ox_core::format::DateStyle;
use ox_core::settings::{ColumnWidth, ColumnWidths};

pub(crate) use column_choice::chosen_from_keys;
pub(crate) use column_text::cell_text;

use crate::folder_view::cells::{self, CellLayout, CellOwners};
use crate::folder_view::column_keys;
use crate::folder_view::column_titles;
use crate::folder_view::column_widths;
use crate::folder_view::item::FileItem;
use crate::folder_view::model::{self, FolderModel};
use crate::folder_view::sorting::{SortColumn, SortDirection, SortOrder};
use crate::folder_view::tree::FolderTree;

/// Icon edge in details rows (`.name-cell svg{height:21px}`).
const ROW_ICON_SIZE: i32 = 21;

/// How long column widths must stay unchanged before they are saved, so a
/// drag saves once instead of on every pixel.
const RESIZE_SETTLE: Duration = Duration::from_millis(500);

/// The signal a [`DetailsView`] emits once the column widths have stayed
/// unchanged for [`RESIZE_SETTLE`].
const COLUMNS_RESIZED: &str = "columns-resized";

/// The signal a [`DetailsView`] emits once the user dragged a column to
/// another place.
const COLUMNS_CHOSEN: &str = "columns-chosen";

/// The Name column's cells: the item's icon beside its name, after the
/// arrow of a folder that expands in `tree`.
fn name_factory(owners: &Rc<CellOwners>, tree: &FolderTree) -> gtk::SignalListItemFactory {
    let factory = gtk::SignalListItemFactory::new();
    cells::connect_file_cells(&factory, CellLayout::DetailsRow, ROW_ICON_SIZE, owners);
    cells::connect_expanders(&factory, tree);
    factory
}

/// The cells of the Date modified, Folder path, Type or Size column: one
/// dim label, registered in `owners`, which dims the cells of cut items.
/// Dates are written in the style `dates` holds when a cell is bound.
fn text_factory(
    column: SortColumn,
    owners: &Rc<CellOwners>,
    dates: &Rc<Cell<DateStyle>>,
) -> gtk::SignalListItemFactory {
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
        cells::show_row_tooltip(&label, &setup_owners, column_text::cell_tooltip(column));
    });
    let bind_owners = Rc::clone(owners);
    let dates = Rc::clone(dates);
    factory.connect_bind(move |_, object| {
        let list_item = cells::as_list_item(object);
        let label = list_item.child().and_downcast::<gtk::Label>();
        if let (Some(item), Some(label)) = (cells::bound_item(list_item), label) {
            label.set_text(&cell_text(column, &item, dates.get()));
            bind_owners.style_cell(&label, &item);
            if column == SortColumn::Size {
                column_text::request_item_count(&label, &item, &bind_owners);
            }
        }
    });
    factory
}

/// What the columns' cells share: the registry they are kept in, how dates
/// are written, and the folders that expand.
#[derive(Debug)]
struct CellContext {
    owners: Rc<CellOwners>,
    dates: Rc<Cell<DateStyle>>,
    tree: FolderTree,
}

impl CellContext {
    /// The factory of `column`'s cells.
    fn factory(&self, column: SortColumn) -> gtk::SignalListItemFactory {
        match column {
            SortColumn::Name => name_factory(&self.owners, &self.tree),
            _ => text_factory(column, &self.owners, &self.dates),
        }
    }
}

/// A resizable column showing `column`, sorted by its header.
fn new_view_column(column: SortColumn, cells: &CellContext) -> gtk::ColumnViewColumn {
    let factory = cells.factory(column);
    let view_column = gtk::ColumnViewColumn::new(Some(column.label()), Some(factory));
    view_column.set_id(Some(column.as_str()));
    view_column.set_resizable(true);
    view_column.set_sorter(Some(&model::column_sorter(column)));
    view_column
}

/// The details columns a window has room for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum DetailsColumns {
    /// Name, Date modified (or Folder path), Type and Size.
    #[default]
    All,
    /// Name and Size only, in a compact window (the 680-pixel rules in
    /// `style.css` hide Date modified and Type); a search keeps Name and
    /// Folder path.
    NameAndSize,
}

/// What the view lists.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum DetailsListing {
    /// A folder's items.
    #[default]
    Folder,
    /// The results of a search, which show Folder path in place of Date
    /// modified (`columnFields` in app.js, VIEW-042).
    SearchResults,
    /// The Recycle Bin's items, which show Original location and Date
    /// deleted in place of Date modified (VIEW-062).
    RecycleBin,
}

/// Whether `listing` has `column` at all, whatever the room.
const fn lists_column(listing: DetailsListing, column: SortColumn) -> bool {
    match column {
        SortColumn::Modified => matches!(listing, DetailsListing::Folder),
        SortColumn::FolderPath => matches!(listing, DetailsListing::SearchResults),
        SortColumn::OriginalLocation | SortColumn::Deleted => matches!(listing, DetailsListing::RecycleBin),
        _ => true,
    }
}

/// Whether settings save `column`'s width while the view lists
/// `listing`: of the columns sharing a saved width, the one the listing
/// has, else the first.
fn saves_width(column: SortColumn, listing: DetailsListing) -> bool {
    let shared = column_widths::settings_column(column);
    let sharing = || {
        SortColumn::ALL
            .into_iter()
            .filter(move |other| column_widths::settings_column(*other) == shared)
    };
    let owner = sharing()
        .find(|other| lists_column(listing, *other))
        .or_else(|| sharing().next());
    owner == Some(column)
}

/// Whether `column` is shown in a window with room for `columns` while it
/// lists `listing` (the `.searching` rules of `style.css` included), when
/// the user chose to show the columns in `chosen`.
fn is_column_shown(
    column: SortColumn,
    columns: DetailsColumns,
    listing: DetailsListing,
    chosen: &[SortColumn],
) -> bool {
    let is_roomy = matches!(columns, DetailsColumns::All);
    let is_search = matches!(listing, DetailsListing::SearchResults);
    lists_column(listing, column)
        && match column {
            SortColumn::Name | SortColumn::FolderPath | SortColumn::OriginalLocation => true,
            SortColumn::Deleted => is_roomy,
            _ if !chosen.contains(&column) => false,
            SortColumn::Size => is_roomy || !is_search,
            _ => is_roomy,
        }
}

mod imp {
    use std::cell::{Cell, OnceCell, RefCell};
    use std::sync::OnceLock;

    use gtk::glib;
    use gtk::glib::subclass::Signal;
    use gtk::prelude::*;
    use gtk::subclass::prelude::*;

    use super::{DetailsColumns, DetailsListing, COLUMNS_CHOSEN, COLUMNS_RESIZED};
    use crate::folder_view::sorting::SortColumn;

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
        /// The columns the window has room for.
        pub(super) columns: Cell<DetailsColumns>,
        /// Whether the view lists a folder or search results.
        pub(super) listing: Cell<DetailsListing>,
        /// Counts the scroll positions the window restored, so a grouped
        /// list filled from empty does not override one restored
        /// meanwhile.
        pub(super) scroll_restores: Cell<u64>,
        /// The columns the user chose to show after Name, in their order.
        pub(super) chosen: RefCell<Vec<SortColumn>>,
        /// Set while the view puts its own columns in order, which is not
        /// the user reordering them.
        pub(super) arranging: Cell<bool>,
        /// What the cells share, set by [`super::DetailsView::new`].
        pub(super) cells: OnceCell<super::CellContext>,
        /// The group headers on screen, whose counts follow the model.
        pub(super) headers: RefCell<Vec<glib::WeakRef<gtk::ListHeader>>>,
        /// Set while a recount of the headers waits for the main loop.
        pub(super) recount_pending: Cell<bool>,
        /// Names the group of an item while the groups are headed.
        pub(super) header_title: RefCell<super::group_headers::HeaderTitle>,
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
            SIGNALS.get_or_init(|| {
                vec![
                    Signal::builder(COLUMNS_RESIZED).build(),
                    Signal::builder(COLUMNS_CHOSEN).build(),
                ]
            })
        }

        fn constructed(&self) {
            self.parent_constructed();
            let column_view = &self.column_view;
            column_view.add_css_class("files");
            column_view.set_show_row_separators(false);
            column_view.set_show_column_separators(false);
            // Titles are dragged to reorder the columns (VIEW-034).
            column_view.set_reorderable(true);
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
        let cells = CellContext {
            owners: Rc::clone(owners),
            dates: Rc::default(),
            tree: model.tree().clone(),
        };
        for column in SortColumn::ALL {
            column_view.append_column(&new_view_column(column, &cells));
        }
        view.imp().chosen.replace(column_choice::default_chosen());
        view.imp()
            .cells
            .set(cells)
            .expect("DetailsView::new sets the cells' context once");
        view.apply_column_widths(None);
        view.show_fitting_columns();
        view.watch_column_widths();
        if let Some(sorter) = column_view.sorter() {
            model.attach_column_sorter(&sorter);
        }
        view.add_sort_carets();
        view.install_column_fit();
        view.follow_column_drags();
        column_keys::make_titles_keyboard_operable(&view);
        view.describe_rows(model);
        view.start_grouped_lists_at_the_top(model);
        view.follow_group_counts(model);
        view.sort_by(SortOrder::DEFAULT);
        view
    }

    /// Names each row after its item and tells screen readers how many
    /// rows the folder has, drawn or not (`aria-label` and
    /// `aria-rowcount` in `renderRows`).
    fn describe_rows(&self, model: &FolderModel) {
        let column_view = self.column_view();
        cells::label_view(column_view.upcast_ref());
        let rows = gtk::SignalListItemFactory::new();
        rows.connect_bind(|_, object| {
            if let Some(row) = object.downcast_ref::<gtk::ColumnViewRow>() {
                if let Some(item) = row.item().and_downcast::<FileItem>() {
                    row.set_accessible_label(&item.entry().name);
                }
            }
        });
        column_view.set_row_factory(Some(&rows));
        model.selection().connect_items_changed(glib::clone!(
            #[weak]
            column_view,
            move |selection, _, _, _| {
                let count = i32::try_from(selection.n_items()).unwrap_or(i32::MAX);
                let columns = column_view
                    .columns()
                    .iter::<gtk::ColumnViewColumn>()
                    .filter_map(Result::ok)
                    .filter(gtk::ColumnViewColumn::is_visible)
                    .count();
                column_view.update_relation(&[
                    gtk::accessible::Relation::RowCount(count),
                    gtk::accessible::Relation::ColCount(i32::try_from(columns).unwrap_or(i32::MAX)),
                ]);
            }
        ));
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

    /// What `column` shows for the first `limit` items the view lists.
    pub(crate) fn cell_texts(&self, column: SortColumn, limit: u32) -> Vec<String> {
        let Some(items) = self.column_view().model() else {
            return Vec::new();
        };
        (0..items.n_items().min(limit))
            .filter_map(|position| items.item(position).and_downcast::<FileItem>())
            .map(|item| cell_text(column, &item, self.date_style()))
            .collect()
    }

    /// The cells' shared context, which `new` sets.
    fn cells(&self) -> &CellContext {
        self.imp()
            .cells
            .get()
            .expect("DetailsView::new sets the cells' context")
    }

    /// How dates are written.
    pub(crate) fn date_style(&self) -> DateStyle {
        self.cells().dates.get()
    }

    /// Writes dates in `style` (VIEW-004), redrawing the date cells
    /// shown.
    pub(crate) fn set_date_style(&self, style: DateStyle) {
        if self.cells().dates.replace(style) != style {
            for column in [SortColumn::Modified, SortColumn::Created, SortColumn::Deleted] {
                self.redraw_column(column);
            }
        }
    }

    /// Binds every shown cell of `column` again, through a new factory.
    pub(crate) fn redraw_column(&self, column: SortColumn) {
        if let Some(view_column) = self.column(column) {
            view_column.set_factory(Some(&self.cells().factory(column)));
        }
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

    /// Shows the columns a window with room for `columns` has.
    pub(crate) fn show_columns(&self, columns: DetailsColumns) {
        self.imp().columns.set(columns);
        self.show_fitting_columns();
    }

    /// Shows the columns of `listing`: Folder path in place of Date
    /// modified for search results, Original location and Date deleted in
    /// the Recycle Bin. A view sorted by a column only another listing
    /// has, such as Folder path in a folder, goes back to sorting by name;
    /// the Sort menu's columns sort every listing.
    pub(crate) fn show_listing(&self, listing: DetailsListing) {
        let earlier = self.imp().listing.replace(listing);
        if earlier != listing {
            self.carry_shared_widths(earlier, listing);
        }
        self.show_fitting_columns();
        let column = self.sort_order().column;
        if !lists_column(listing, column) && !SortColumn::IN_SORT_MENU.contains(&column) {
            self.sort_by(SortOrder::DEFAULT);
        }
    }

    /// Gives each column `listing` brings in the width of the column it
    /// shares a saved width with in `earlier`, so a width the user set in
    /// one listing holds in the other (Date deleted and Date modified).
    fn carry_shared_widths(&self, earlier: DetailsListing, listing: DetailsListing) {
        for (column, view_column) in self.view_columns() {
            if !lists_column(listing, column) || lists_column(earlier, column) {
                continue;
            }
            let shared = column_widths::settings_column(column);
            let partner = SortColumn::ALL.into_iter().find(|other| {
                *other != column
                    && lists_column(earlier, *other)
                    && column_widths::settings_column(*other) == shared
            });
            if let Some(partner) = partner.and_then(|partner| self.column(partner)) {
                view_column.set_fixed_width(partner.fixed_width());
            }
        }
    }

    /// Shows the columns the room and the listing call for, and hides the
    /// others.
    fn show_fitting_columns(&self) {
        let columns = self.imp().columns.get();
        let listing = self.imp().listing.get();
        let chosen = self.imp().chosen.borrow().clone();
        let changes: Vec<(gtk::ColumnViewColumn, bool)> = self
            .view_columns()
            .map(|(column, view_column)| {
                let shown = is_column_shown(column, columns, listing, &chosen);
                (view_column, shown)
            })
            .filter(|(view_column, shown)| view_column.is_visible() != *shown)
            .collect();
        if changes.is_empty() {
            return;
        }
        self.change_columns_without_headers(|| {
            for (view_column, shown) in changes {
                view_column.set_visible(shown);
            }
        });
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
    pub(crate) fn widths_to_save(&self) -> Vec<ColumnWidth> {
        let listing = self.imp().listing.get();
        let saved = self
            .view_columns()
            .filter(|(column, _)| saves_width(*column, listing));
        let widths = saved.filter_map(|(column, view_column)| {
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
    pub(crate) fn primary_sort(&self) -> Option<SortOrder> {
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

    /// The columns shown with room for `columns` while listing `listing`.
    fn shown_columns(columns: DetailsColumns, listing: DetailsListing) -> Vec<SortColumn> {
        let chosen = column_choice::default_chosen();
        let shown = SortColumn::ALL
            .into_iter()
            .filter(|column| is_column_shown(*column, columns, listing, &chosen));
        shown.collect()
    }

    #[test]
    fn a_compact_window_keeps_only_name_and_size() {
        let compact = shown_columns(DetailsColumns::NameAndSize, DetailsListing::Folder);
        let roomy = shown_columns(DetailsColumns::All, DetailsListing::Folder);

        assert_eq!(compact, [SortColumn::Name, SortColumn::Size]);
        assert_eq!(
            roomy,
            [
                SortColumn::Name,
                SortColumn::Modified,
                SortColumn::Type,
                SortColumn::Size
            ]
        );
    }

    /// parity: VIEW-042
    #[test]
    fn search_results_show_folder_path_in_place_of_date_modified() {
        let roomy = shown_columns(DetailsColumns::All, DetailsListing::SearchResults);
        let compact = shown_columns(DetailsColumns::NameAndSize, DetailsListing::SearchResults);

        assert_eq!(
            roomy,
            [
                SortColumn::Name,
                SortColumn::FolderPath,
                SortColumn::Type,
                SortColumn::Size
            ]
        );
        assert_eq!(compact, [SortColumn::Name, SortColumn::FolderPath]);
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

    /// Folder path has a saved width of its own (`parentUri`).
    ///
    /// parity: VIEW-028, VIEW-042
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
                pixels: 176.0,
            },
            ColumnWidth {
                column: Column::ParentUri,
                pixels: 330.0,
            },
            ColumnWidth {
                column: Column::Type,
                pixels: 200.0,
            },
            ColumnWidth {
                column: Column::Size,
                pixels: 90.0,
            },
            ColumnWidth {
                column: Column::Created,
                pixels: 176.0,
            },
            ColumnWidth {
                column: Column::Extension,
                pixels: 90.0,
            },
            ColumnWidth {
                column: Column::Owner,
                pixels: 110.0,
            },
            ColumnWidth {
                column: Column::Permissions,
                pixels: 110.0,
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
