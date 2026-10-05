// SPDX-License-Identifier: AGPL-3.0-only
//! The icon view: tiles with an icon and up to two lines of name, or the
//! compact list of small icons with their names beside them.
//!
//! Matches `.file-tile` in `v2.0.0:desktop/ui/style.css` ("Large icons": a 56
//! pixel icon in a 135 pixel cell) and the grid layout of `renderRows` in
//! `v2.0.0:desktop/ui/app.js`. The tiles zoom through the [`IconSize`] levels;
//! the window binds Explorer's four named sizes to Ctrl+Shift+1..4. The
//! compact layout is Dolphin's Compact view and Explorer's List layout
//! (VIEW-008): the same grid turned on its side, so the items fill columns
//! top to bottom and the view scrolls sideways. [`IconView`] is the widget;
//! it keeps its layout and the text size, and fits its lines to its size.

use std::rc::Rc;

use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;

use crate::folder_view::cells::{self, CellLayout, CellOwners};
use crate::folder_view::icon_size::compact_row;
pub(crate) use crate::folder_view::icon_size::{cell_size, IconSize};
use crate::text_size::TextSize;

/// The icon edge of the compact layout (Explorer's List uses small icons).
const COMPACT_ICON_SIZE: i32 = 16;

/// The CSS class of the compact layout.
const COMPACT_CLASS: &str = "compact";

/// How the icon view lays its items out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum GridLayout {
    /// Tiles of one icon size, in rows that scroll down.
    Icons(IconSize),
    /// Small icons with names beside them, in columns that scroll sideways.
    Compact,
}

/// The registry the views' cells share, which every factory the icon view
/// builds registers its cells in.
#[derive(Debug)]
struct TileRegistries {
    owners: Rc<CellOwners>,
}

impl TileRegistries {
    /// Cells for `layout`.
    fn factory(&self, layout: GridLayout) -> gtk::SignalListItemFactory {
        let factory = gtk::SignalListItemFactory::new();
        let (cell_layout, icon_pixels) = match layout {
            GridLayout::Icons(size) => (CellLayout::IconTile, size.pixels()),
            GridLayout::Compact => (CellLayout::CompactItem, COMPACT_ICON_SIZE),
        };
        cells::connect_file_cells(&factory, cell_layout, icon_pixels, &self.owners);
        factory
    }
}

mod imp {
    use std::cell::{Cell, OnceCell};

    use gtk::glib;
    use gtk::prelude::*;
    use gtk::subclass::prelude::*;

    use super::{GridLayout, IconSize, TileRegistries};
    use crate::text_size::TextSize;

    /// Private state of [`super::IconView`].
    #[derive(Debug)]
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
        let registries = TileRegistries {
            owners: Rc::clone(owners),
        };
        view.imp()
            .registries
            .set(registries)
            .expect("a new IconView has no registries yet");
        view.apply_layout(GridLayout::Icons(IconSize::LARGE));
        view
    }

    /// The grid of tiles, which holds the selection model and the keyboard
    /// focus.
    pub(crate) fn grid(&self) -> &gtk::GridView {
        &self.imp().grid
    }

    /// The adjustment of the scroll position: the horizontal one in the
    /// compact layout, which scrolls sideways, else the vertical one.
    pub(crate) fn scroll_adjustment(&self) -> gtk::Adjustment {
        let scroller = &self.imp().scroller;
        match self.layout() {
            GridLayout::Compact => scroller.hadjustment(),
            GridLayout::Icons(_) => scroller.vadjustment(),
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
    /// view is compact.
    pub(crate) fn icon_size(&self) -> IconSize {
        self.imp().icon_size.get()
    }

    /// Restores a saved icon size without leaving the compact layout.
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
    /// [`super::icon_size::CellSize::columns_in`]), or compact rows down its
    /// height.
    pub(crate) fn fit_lines(&self) {
        let imp = self.imp();
        let lines = match imp.layout.get() {
            GridLayout::Icons(size) => cell_size(size, imp.text_size.get()).columns_in(imp.scroller.width()),
            GridLayout::Compact => {
                let rows = imp.scroller.height() / compact_row(imp.text_size.get()).max(1);
                u32::try_from(rows.max(1)).unwrap_or(1)
            }
        };
        if imp.grid.max_columns() != lines {
            imp.grid.set_max_columns(lines);
        }
    }

    /// Gives the grid `layout`'s CSS classes, orientation and cells.
    fn apply_layout(&self, layout: GridLayout) {
        let imp = self.imp();
        imp.layout.set(layout);
        for class in IconSize::css_classes().chain([COMPACT_CLASS]) {
            imp.grid.remove_css_class(class);
        }
        let (class, orientation) = match layout {
            GridLayout::Icons(size) => {
                imp.icon_size.set(size);
                (size.css_class(), gtk::Orientation::Vertical)
            }
            GridLayout::Compact => (COMPACT_CLASS, gtk::Orientation::Horizontal),
        };
        imp.grid.add_css_class(class);
        imp.grid.set_orientation(orientation);
        let registries = imp
            .registries
            .get()
            .expect("IconView::new sets the registries first");
        imp.grid.set_factory(Some(&registries.factory(layout)));
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
}
