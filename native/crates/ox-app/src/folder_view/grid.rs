// SPDX-License-Identifier: AGPL-3.0-only
//! The icon view: tiles with an icon and up to two lines of name, the
//! compact list of small icons with their names beside them, or Windows
//! 11's Tiles and Content.
//!
//! Matches `.file-tile` in `v2.0.0:desktop/ui/style.css` ("Large icons": a 56
//! pixel icon in a 135 pixel cell) and the grid layout of `renderRows` in
//! `v2.0.0:desktop/ui/app.js`. The tiles zoom through the [`IconSize`] levels;
//! the window binds Explorer's four named sizes to Ctrl+Shift+1..4. The
//! compact layout is Dolphin's Compact view and Explorer's List layout
//! (VIEW-008): the same grid turned on its side, so the items fill columns
//! top to bottom and the view scrolls sideways. Tiles and Content
//! (VIEW-071) are the same grid with other cells: Tiles in as many columns
//! of 256-pixel tiles as fit, Content in one column of rows that span the
//! width, both scrolling down. A grouped folder's Tiles and Content keep
//! its groups, each under a heading as in Details ([`GroupedSelection`]).
//! [`IconView`] is the widget; it keeps its layout and the text size, and
//! fits its lines to its size.

mod grouped;

use std::cell::RefCell;
use std::rc::Rc;

use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;

pub(crate) use grouped::{CollapsedGroups, GroupSlot, GroupedSelection, SlotKind};

use crate::folder_view::cells::{self, CellLayout, CellOwners, PositionMap};
use crate::folder_view::details::GroupTitle;
use crate::folder_view::group_slots::RowHeights;
pub(crate) use crate::folder_view::icon_size::{cell_size, IconSize};
use crate::folder_view::icon_size::{
    compact_density_heading_row, compact_density_tiles_row, compact_row, content_row, group_heading_row,
    tiles_cell, CONTENT_ICON, TILES_ICON,
};
use crate::text_size::TextSize;

/// The icon edge of the compact layout (Explorer's List uses small icons).
const COMPACT_ICON_SIZE: i32 = 16;

/// The CSS class of the compact layout.
const COMPACT_CLASS: &str = "compact";

/// The CSS class of the Tiles layout.
const TILES_CLASS: &str = "tiles";

/// The CSS class of the Content layout.
const CONTENT_CLASS: &str = "content";

/// The CSS class of a grouped grid of icon tiles (Gallery's months,
/// HOME-019), whose heading rows are a line of text tall.
const ICON_GROUPS_CLASS: &str = "icon-groups";

/// How the icon view lays its items out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum GridLayout {
    /// Tiles of one icon size, in rows that scroll down.
    Icons(IconSize),
    /// Small icons with names beside them, in columns that scroll sideways.
    Compact,
    /// Explorer's Tiles: a 48-pixel icon with its name, type and size
    /// beside it, in rows that scroll down.
    Tiles,
    /// Explorer's Content: one row per item across the width, its name and
    /// type beside its icon and its date modified and size at the right.
    Content,
}

impl GridLayout {
    /// Whether the layout keeps a grouped folder's groups: Tiles and
    /// Content do, as in Windows 11 (VIEW-071).
    const fn shows_groups(self) -> bool {
        matches!(self, GridLayout::Tiles | GridLayout::Content)
    }
}

/// The grouped model the grid shows, if any, shared with the cells'
/// [`PositionMap`].
type SharedGrouping = Rc<RefCell<Option<GroupedSelection>>>;

/// The registry the views' cells share, which every factory the icon view
/// builds registers its cells in.
struct TileRegistries {
    owners: Rc<CellOwners>,
    /// Maps the grid's positions to the listing's.
    positions: PositionMap,
}

impl std::fmt::Debug for TileRegistries {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.debug_struct("TileRegistries").finish_non_exhaustive()
    }
}

impl TileRegistries {
    /// Cells for `layout`.
    fn factory(&self, layout: GridLayout) -> gtk::SignalListItemFactory {
        let factory = gtk::SignalListItemFactory::new();
        let (cell_layout, icon_pixels) = match layout {
            GridLayout::Icons(size) => (CellLayout::IconTile, size.pixels()),
            GridLayout::Compact => (CellLayout::CompactItem, COMPACT_ICON_SIZE),
            GridLayout::Tiles => (CellLayout::Tile, TILES_ICON),
            GridLayout::Content => (CellLayout::ContentRow, CONTENT_ICON),
        };
        let positions = Some(Rc::clone(&self.positions));
        cells::connect_mapped_file_cells(&factory, cell_layout, icon_pixels, &self.owners, positions);
        factory
    }
}

mod imp {
    use std::cell::{Cell, OnceCell, RefCell};

    use gtk::glib;
    use gtk::prelude::*;
    use gtk::subclass::prelude::*;

    use super::{GridLayout, IconSize, SharedGrouping, TileRegistries};
    use crate::folder_view::details::GroupTitle;
    use crate::text_size::TextSize;

    /// Private state of [`super::IconView`].
    pub(crate) struct IconView {
        /// Scrolls the grid; the view's only child. The grid must be the
        /// scroller's direct child: GTK then builds tiles only for the rows
        /// on screen.
        pub(super) scroller: gtk::ScrolledWindow,
        /// The tiles.
        pub(super) grid: gtk::GridView,
        /// Set by [`super::IconView::new`].
        pub(super) registries: OnceCell<TileRegistries>,
        /// How the items are laid out.
        pub(super) layout: Cell<GridLayout>,
        /// The last icon size, retained while the compact layout is shown.
        pub(super) icon_size: Cell<IconSize>,
        /// The text size, which sizes the cells too.
        pub(super) text_size: Cell<TextSize>,
        /// The folder's selection model, while the view shows it.
        pub(super) items: RefCell<Option<gtk::MultiSelection>>,
        /// Names each item's group while the folder is grouped.
        pub(super) group_title: RefCell<Option<GroupTitle>>,
        /// The grid's model while it shows grouped Tiles or Content.
        pub(super) grouping: SharedGrouping,
        /// The groups collapsed under their headings, which the folder
        /// model shares.
        pub(super) collapsed: RefCell<super::CollapsedGroups>,
        /// The icon sizes keep the groups too: Gallery's months
        /// (HOME-019).
        pub(super) icon_groups: Cell<bool>,
    }

    impl std::fmt::Debug for IconView {
        fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            formatter
                .debug_struct("IconView")
                .field("layout", &self.layout)
                .field("icon_size", &self.icon_size)
                .field("text_size", &self.text_size)
                .field("grouped", &self.group_title.borrow().is_some())
                .finish_non_exhaustive()
        }
    }

    impl Default for IconView {
        /// Large icons at the default text size, as a new window starts.
        fn default() -> Self {
            Self {
                scroller: gtk::ScrolledWindow::default(),
                grid: gtk::GridView::default(),
                registries: OnceCell::new(),
                layout: Cell::new(GridLayout::Icons(IconSize::LARGE)),
                icon_size: Cell::new(IconSize::LARGE),
                text_size: Cell::new(TextSize::DEFAULT),
                items: RefCell::default(),
                group_title: RefCell::default(),
                grouping: SharedGrouping::default(),
                collapsed: RefCell::default(),
                icon_groups: Cell::new(false),
            }
        }
    }

    #[glib::object_subclass]
    impl ObjectSubclass for IconView {
        const NAME: &'static str = "OxIconView";
        type Type = super::IconView;
        type ParentType = gtk::Widget;

        fn class_init(klass: &mut Self::Class) {
            // `GtkGridView` cannot be subclassed, so the view wraps its
            // scroller and gives it all of its own size.
            klass.set_layout_manager_type::<gtk::BinLayout>();
        }
    }

    impl ObjectImpl for IconView {
        fn constructed(&self) {
            self.parent_constructed();
            self.grid.add_css_class("files");
            self.grid.set_tab_behavior(gtk::ListTabBehavior::Item);
            super::cells::label_view(self.grid.upcast_ref());
            self.scroller.set_child(Some(&self.grid));
            self.scroller.set_parent(&*self.obj());
            self.obj().fit_lines_to_size();
        }

        fn dispose(&self) {
            self.scroller.unparent();
        }
    }

    impl WidgetImpl for IconView {}
}

glib::wrapper! {
    /// The icon view: tiles of one [`IconSize`], or the compact list, in a
    /// scroller, in as many lines as its size holds.
    pub(crate) struct IconView(ObjectSubclass<imp::IconView>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl IconView {
    /// An icon view of Large icons whose tiles are registered in
    /// `owners`. It shows no model until the window makes it the visible
    /// view.
    pub(crate) fn new(owners: &Rc<CellOwners>) -> Self {
        let view: Self = glib::Object::new();
        let grouping = Rc::clone(&view.imp().grouping);
        let registries = TileRegistries {
            owners: Rc::clone(owners),
            positions: Rc::new(move |slot| match grouping.borrow().as_ref() {
                Some(grouped) => grouped.item_at(slot),
                None => Some(slot),
            }),
        };
        view.imp()
            .registries
            .set(registries)
            .expect("a new IconView has no registries yet");
        view.apply_layout(GridLayout::Icons(IconSize::LARGE));
        view
    }

    /// Collapses the groups `collapsed` names, which the folder model
    /// keeps for every view.
    pub(crate) fn share_collapsed_groups(&self, collapsed: &CollapsedGroups) {
        self.imp().collapsed.replace(Rc::clone(collapsed));
    }

    /// The grid of tiles, which holds the selection model and the keyboard
    /// focus.
    pub(crate) fn grid(&self) -> &gtk::GridView {
        &self.imp().grid
    }

    /// Shows the folder's `items`, or nothing.
    pub(crate) fn set_model(&self, items: Option<&gtk::MultiSelection>) {
        let imp = self.imp();
        if imp.items.borrow().as_ref() == items && imp.grid.model().is_some() == items.is_some() {
            return;
        }
        imp.items.replace(items.cloned());
        self.show_model();
    }

    /// Keeps a grouped folder's groups in Tiles and Content, each under a
    /// heading `title` names, or shows no groups with `None` (VIEW-022).
    pub(crate) fn set_group_titles(&self, title: Option<GroupTitle>) {
        let grouped_now = title.is_some();
        let was_grouped = self.imp().group_title.replace(title).is_some();
        match self.grouped() {
            // Grouped anew: the groups may have moved.
            Some(grouped) if grouped_now => grouped.regroup(),
            _ if was_grouped || grouped_now => self.show_model(),
            _ => {}
        }
    }

    /// Has the icon sizes keep a grouped folder's groups too, each under a
    /// heading, as Gallery shows its months (HOME-019); other folders show
    /// them only in Tiles and Content.
    pub(crate) fn set_icon_groups(&self, kept: bool) {
        if self.imp().icon_groups.replace(kept) == kept {
            return;
        }
        if matches!(self.layout(), GridLayout::Icons(_)) && self.imp().group_title.borrow().is_some() {
            self.show_model();
            self.fit_lines();
        }
    }

    /// Whether `layout` shows a grouped folder's groups.
    fn keeps_groups(&self, layout: GridLayout) -> bool {
        layout.shows_groups() || (matches!(layout, GridLayout::Icons(_)) && self.imp().icon_groups.get())
    }

    /// The grouped model the grid shows, while it shows grouped Tiles or
    /// Content.
    pub(crate) fn grouped(&self) -> Option<GroupedSelection> {
        self.imp().grouping.borrow().clone()
    }

    /// The listing position of the item the grid's `slot` shows.
    pub(crate) fn item_at(&self, slot: u32) -> Option<u32> {
        match self.grouped() {
            Some(grouped) => grouped.item_at(slot),
            None => Some(slot),
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
        let slot = match self.grouped() {
            Some(grouped) => grouped.slot_of(position),
            None => Some(position),
        };
        if let Some(slot) = slot {
            self.imp().grid.scroll_to(slot, flags, scroll);
        }
    }

    /// How tall a page of the view and its rows are, for Page Up and Page
    /// Down in grouped Tiles and Content.
    /// In Compact view (VIEW-067) the rows are as close as its stylesheet
    /// draws them (`theme/fonts.rs`).
    pub(crate) fn row_heights(&self) -> RowHeights {
        let text_size = self.imp().text_size.get();
        let content = self.layout() == GridLayout::Content;
        if let GridLayout::Icons(size) = self.layout() {
            let heading = if self.in_compact_density() {
                compact_density_heading_row(text_size, false)
            } else {
                group_heading_row(text_size)
            };
            return RowHeights {
                page: self.scroll_adjustment().page_size(),
                item: cell_size(size, text_size).height,
                heading,
            };
        }
        let (item, heading) = match (self.in_compact_density(), content) {
            (false, true) => (content_row(text_size), group_heading_row(text_size)),
            (false, false) => (tiles_cell(text_size).height, group_heading_row(text_size)),
            (true, true) => (
                content_row(text_size),
                compact_density_heading_row(text_size, true),
            ),
            (true, false) => (
                compact_density_tiles_row(text_size),
                compact_density_heading_row(text_size, false),
            ),
        };
        RowHeights {
            page: self.scroll_adjustment().page_size(),
            item,
            heading,
        }
    }

    /// Whether the window showing the view is in Compact view: it carries
    /// the class `window/compact_density.rs` sets.
    fn in_compact_density(&self) -> bool {
        self.root()
            .is_some_and(|root| root.has_css_class(crate::window::COMPACT_DENSITY_CLASS))
    }

    /// Gives the grid the folder's items, grouped in Tiles and Content
    /// while the folder is grouped.
    fn show_model(&self) {
        let imp = self.imp();
        let items = imp.items.borrow().clone();
        let title = imp.group_title.borrow().clone();
        let grouped = match (items.as_ref(), title) {
            (Some(items), Some(title)) if self.keeps_groups(self.layout()) => {
                let collapsed = imp.collapsed.borrow().clone();
                Some(GroupedSelection::new(
                    items,
                    title,
                    &collapsed,
                    imp.grid.max_columns(),
                ))
            }
            _ => None,
        };
        imp.grouping.replace(grouped.clone());
        crate::window::widget_tree::toggle_class(
            &imp.grid,
            crate::folder_view::details::GROUPED_CLASS,
            grouped.is_some(),
        );
        crate::window::widget_tree::toggle_class(
            &imp.grid,
            ICON_GROUPS_CLASS,
            grouped.is_some() && matches!(self.layout(), GridLayout::Icons(_)),
        );
        match (grouped, items) {
            (Some(grouped), _) => imp.grid.set_model(Some(&grouped)),
            (None, items) => imp.grid.set_model(items.as_ref()),
        }
        self.fit_lines();
    }

    /// The adjustment of the scroll position: the horizontal one in the
    /// compact layout, which scrolls sideways, else the vertical one.
    pub(crate) fn scroll_adjustment(&self) -> gtk::Adjustment {
        let scroller = &self.imp().scroller;
        match self.layout() {
            GridLayout::Compact => scroller.hadjustment(),
            GridLayout::Icons(_) | GridLayout::Tiles | GridLayout::Content => scroller.vadjustment(),
        }
    }

    /// Both adjustments of the scroller, down and sideways: the compact
    /// layout scrolls along the one, the icons along the other.
    pub(crate) fn both_scroll_adjustments(&self) -> [gtk::Adjustment; 2] {
        let scroller = &self.imp().scroller;
        [scroller.vadjustment(), scroller.hadjustment()]
    }

    /// How the items are laid out.
    pub(crate) fn layout(&self) -> GridLayout {
        self.imp().layout.get()
    }

    /// The icon size of the tiles, or of the last tiles shown while the
    /// view is compact, Tiles or Content.
    pub(crate) fn icon_size(&self) -> IconSize {
        self.imp().icon_size.get()
    }

    /// Restores a saved icon size without leaving the compact, Tiles or
    /// Content layout.
    pub(crate) fn set_icon_size(&self, size: IconSize) {
        self.imp().icon_size.set(size);
        if matches!(self.layout(), GridLayout::Icons(_)) {
            self.set_layout(GridLayout::Icons(size));
        }
    }

    /// Lays the items out as `layout`.
    pub(crate) fn set_layout(&self, layout: GridLayout) {
        if self.layout() == layout {
            return;
        }
        self.apply_layout(layout);
        self.fit_lines();
    }

    /// Sizes the cells for text of `size`.
    pub(crate) fn set_text_size(&self, size: TextSize) {
        self.imp().text_size.set(size);
        self.fit_lines();
    }

    /// Gives the grid the lines its size holds at the current layout and
    /// text size: columns of tiles across its width (see
    /// [`super::icon_size::CellSize::columns_in`]), compact rows down its
    /// height, columns of Tiles across its width, or Content's one column.
    pub(crate) fn fit_lines(&self) {
        let imp = self.imp();
        let text_size = imp.text_size.get();
        let width = imp.scroller.width();
        let lines = match imp.layout.get() {
            GridLayout::Icons(size) => cell_size(size, text_size).columns_in(width),
            GridLayout::Compact => {
                let rows = imp.scroller.height() / compact_row(text_size).max(1);
                u32::try_from(rows.max(1)).unwrap_or(1)
            }
            GridLayout::Tiles => tiles_cell(text_size).columns_in(width),
            GridLayout::Content => 1,
        };
        if imp.grid.max_columns() != lines {
            imp.grid.set_max_columns(lines);
        }
        // Grouped, every row holds exactly as many slots as the groups are
        // laid out for.
        let grouped = self.grouped();
        let min_columns = if grouped.is_some() { lines } else { 1 };
        if imp.grid.min_columns() != min_columns {
            imp.grid.set_min_columns(min_columns);
        }
        if let Some(grouped) = grouped {
            grouped.set_columns(lines);
        }
    }

    /// Gives the grid `layout`'s CSS classes, orientation and cells.
    fn apply_layout(&self, layout: GridLayout) {
        let imp = self.imp();
        let regroups = self.keeps_groups(imp.layout.replace(layout)) != self.keeps_groups(layout);
        for class in IconSize::css_classes().chain([COMPACT_CLASS, TILES_CLASS, CONTENT_CLASS]) {
            imp.grid.remove_css_class(class);
        }
        let (class, orientation) = match layout {
            GridLayout::Icons(size) => {
                imp.icon_size.set(size);
                (size.css_class(), gtk::Orientation::Vertical)
            }
            GridLayout::Compact => (COMPACT_CLASS, gtk::Orientation::Horizontal),
            GridLayout::Tiles => (TILES_CLASS, gtk::Orientation::Vertical),
            GridLayout::Content => (CONTENT_CLASS, gtk::Orientation::Vertical),
        };
        imp.grid.add_css_class(class);
        imp.grid.set_orientation(orientation);
        let registries = imp
            .registries
            .get()
            .expect("IconView::new sets the registries first");
        imp.grid.set_factory(Some(&registries.factory(layout)));
        if regroups && imp.group_title.borrow().is_some() && imp.items.borrow().is_some() {
            self.show_model();
        }
    }

    /// Keeps the lines at what the view's size holds.
    fn fit_lines_to_size(&self) {
        let scroller = &self.imp().scroller;
        for adjustment in [scroller.hadjustment(), scroller.vadjustment()] {
            adjustment.connect_page_size_notify(glib::clone!(
                #[weak(rename_to = view)]
                self,
                move |_| view.fit_lines_when_allocated()
            ));
        }
    }

    /// Fits the lines once GTK has finished allocating the view. The page
    /// size changes while GTK allocates the grid, and GTK ignores a resize
    /// the grid queues then, so a window that opened in the icon view kept
    /// one column.
    fn fit_lines_when_allocated(&self) {
        glib::idle_add_local_once(glib::clone!(
            #[weak(rename_to = view)]
            self,
            move || view.fit_lines()
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[gtk::test]
    fn switching_the_icon_size_restyles_the_tiles() {
        let view = IconView::new(&CellOwners::new());
        let grid = view.grid();
        assert_eq!(view.icon_size(), IconSize::LARGE, "a new view shows Large icons");
        assert!(grid.has_css_class(IconSize::LARGE.css_class()));
        view.set_layout(GridLayout::Icons(IconSize::SMALL));
        assert_eq!(view.icon_size(), IconSize::SMALL);
        assert!(grid.has_css_class(IconSize::SMALL.css_class()));
        assert!(!grid.has_css_class(IconSize::LARGE.css_class()));
        view.set_layout(GridLayout::Compact);
        assert_eq!(
            view.icon_size(),
            IconSize::SMALL,
            "List keeps the previous icon size"
        );
        view.set_icon_size(IconSize::EXTRA_LARGE);
        assert_eq!(view.layout(), GridLayout::Compact);
        assert_eq!(view.icon_size(), IconSize::EXTRA_LARGE);
    }

    /// The compact layout fills columns top to bottom and scrolls sideways.
    ///
    /// parity: VIEW-008
    #[gtk::test]
    fn the_compact_layout_fills_columns_and_scrolls_sideways() {
        let view = IconView::new(&CellOwners::new());
        view.set_layout(GridLayout::Compact);
        let grid = view.grid();
        assert_eq!(grid.orientation(), gtk::Orientation::Horizontal);
        assert!(grid.has_css_class(COMPACT_CLASS));
        assert!(!grid.has_css_class(IconSize::LARGE.css_class()));
        assert_eq!(view.scroll_adjustment(), view.imp().scroller.hadjustment());
        view.set_layout(GridLayout::Icons(IconSize::LARGE));
        assert_eq!(grid.orientation(), gtk::Orientation::Vertical);
    }

    /// Tiles and Content scroll down, each with its own class, and keep the
    /// icon size the icons had; Content is one column.
    ///
    /// parity: VIEW-071
    #[gtk::test]
    fn tiles_and_content_scroll_down_and_keep_the_icon_size() {
        let view = IconView::new(&CellOwners::new());
        view.set_layout(GridLayout::Icons(IconSize::MEDIUM));
        let grid = view.grid();
        for (layout, class) in [
            (GridLayout::Tiles, TILES_CLASS),
            (GridLayout::Content, CONTENT_CLASS),
        ] {
            view.set_layout(layout);
            assert_eq!(grid.orientation(), gtk::Orientation::Vertical);
            assert!(grid.has_css_class(class));
            assert!(!grid.has_css_class(IconSize::MEDIUM.css_class()));
            assert_eq!(view.scroll_adjustment(), view.imp().scroller.vadjustment());
            assert_eq!(view.icon_size(), IconSize::MEDIUM);
        }
        assert_eq!(grid.max_columns(), 1, "Content is one column");
    }
}
