// SPDX-License-Identifier: AGPL-3.0-only
//! The icon view: tiles with a large icon and up to two lines of name.
//!
//! Matches `.file-tile` in `desktop/ui/style.css` ("Large icons": a 56
//! pixel icon in a 135 pixel cell) and the grid layout of `renderRows` in
//! `desktop/ui/app.js`. Explorer's other icon layouts use the same tiles
//! with a different icon size; the window binds them to Ctrl+Shift+1..4
//! as Explorer does. [`IconView`] is the widget; it keeps its icon size
//! and the text size, and fits its columns to its width.

use std::rc::Rc;

use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;

use crate::folder_view::cells::{self, CellLayout, CellOwners};
use crate::text_size::{self, TextSize};

/// A large tile's width beyond its icon: the 135-pixel `gridWidth` less
/// the 56-pixel icon of Large icons. Larger icons widen the tile by as
/// much as the icon grows.
const TILE_WIDTH_BEYOND_ICON: i32 = 79;

/// Icon sizes of Explorer's icon layouts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum IconSize {
    /// Explorer's "Extra large icons" (Ctrl+Shift+1).
    ExtraLarge,
    /// "Large icons" (Ctrl+Shift+2), the Python app's only icon view.
    Large,
    /// "Medium icons" (Ctrl+Shift+3).
    Medium,
    /// "Small icons" (Ctrl+Shift+4).
    Small,
}

impl IconSize {
    /// Every size, largest first.
    pub(crate) const ALL: [IconSize; 4] = [
        IconSize::ExtraLarge,
        IconSize::Large,
        IconSize::Medium,
        IconSize::Small,
    ];

    /// Icon edge in logical pixels.
    pub(crate) const fn pixels(self) -> i32 {
        match self {
            IconSize::ExtraLarge => 96,
            IconSize::Large => 56,
            IconSize::Medium => 40,
            IconSize::Small => 28,
        }
    }

    /// Menu label.
    pub(crate) const fn label(self) -> &'static str {
        match self {
            IconSize::ExtraLarge => "Extra large icons",
            IconSize::Large => "Large icons",
            IconSize::Medium => "Medium icons",
            IconSize::Small => "Small icons",
        }
    }

    /// Action-state key.
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            IconSize::ExtraLarge => "extra-large",
            IconSize::Large => "large",
            IconSize::Medium => "medium",
            IconSize::Small => "small",
        }
    }

    /// The size for an action-state key.
    pub(crate) fn from_key(key: &str) -> Option<IconSize> {
        Self::ALL.into_iter().find(|size| size.as_str() == key)
    }

    /// CSS class that widens tiles for large icons.
    pub(crate) const fn css_class(self) -> &'static str {
        match self {
            IconSize::ExtraLarge => "icons-extra-large",
            IconSize::Large => "icons-large",
            IconSize::Medium => "icons-medium",
            IconSize::Small => "icons-small",
        }
    }
}

/// An icon-view cell in pixels: a tile plus the gap to its neighbours.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct CellSize {
    /// The narrowest a cell may be.
    pub width: i32,
    /// The row pitch.
    pub height: i32,
}

impl CellSize {
    /// How many columns of these cells a pane `pane_width` pixels wide
    /// shows: `max(1, floor(clientWidth / gridWidth))`, as `renderRows` in
    /// app.js counts them. The tiles then share the pane's width less its
    /// 20 pixels of inset, so they can be a little narrower than a cell.
    ///
    /// GTK keeps tiles for about thirty rows of `max-columns` alive, so the
    /// window sets exactly this many columns rather than a generous cap,
    /// which made every listing build thousands of tiles.
    pub(crate) fn columns_in(self, pane_width: i32) -> u32 {
        let columns = pane_width / self.width.max(1);
        u32::try_from(columns.max(1)).unwrap_or(1)
    }
}

/// The cell of tiles of `size` at `text_size`: `gridWidth` ×
/// `gridRow` from `metrics()` in text-size.js for large icons (135 × 130
/// at 100%), widened and heightened with the icon for the other sizes,
/// which the Python app does not have.
pub(crate) fn cell_size(size: IconSize, text_size: TextSize) -> CellSize {
    let metrics = text_size::metrics(text_size);
    let icon_growth = size.pixels() - IconSize::Large.pixels();
    let width_for_icon = size.pixels() + TILE_WIDTH_BEYOND_ICON;
    CellSize {
        width: metrics.grid_width.max(width_for_icon),
        height: metrics.grid_row + icon_growth,
    }
}

/// The registry the views' cells share, which every tile factory the
/// icon view builds registers its tiles in.
#[derive(Debug)]
struct TileRegistries {
    owners: Rc<CellOwners>,
}

impl TileRegistries {
    /// Tiles of `size` icons above their names.
    fn tile_factory(&self, size: IconSize) -> gtk::SignalListItemFactory {
        let factory = gtk::SignalListItemFactory::new();
        let icon_pixels = size.pixels();
        cells::connect_file_cells(&factory, CellLayout::IconTile, icon_pixels, &self.owners);
        factory
    }
}

mod imp {
    use std::cell::{Cell, OnceCell};

    use gtk::glib;
    use gtk::prelude::*;
    use gtk::subclass::prelude::*;

    use super::{IconSize, TileRegistries};
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
        /// The icon size the tiles show.
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
                icon_size: Cell::new(IconSize::Large),
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
            self.grid.set_enable_rubberband(true);
            self.grid.set_tab_behavior(gtk::ListTabBehavior::Item);
            self.scroller.set_child(Some(&self.grid));
            self.scroller.set_parent(&*self.obj());
            self.obj().fit_columns_to_width();
        }

        fn dispose(&self) {
            self.scroller.unparent();
        }
    }

    impl WidgetImpl for IconView {}
}

glib::wrapper! {
    /// The icon view: tiles of one [`IconSize`] in a scroller, in as many
    /// columns as its width holds.
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
        view.apply_icon_size(IconSize::Large);
        view
    }

    /// The grid of tiles, which holds the selection model and the keyboard
    /// focus.
    pub(crate) fn grid(&self) -> &gtk::GridView {
        &self.imp().grid
    }

    /// The adjustment of the vertical scroll position.
    pub(crate) fn vadjustment(&self) -> gtk::Adjustment {
        self.imp().scroller.vadjustment()
    }

    /// The icon size the tiles show.
    pub(crate) fn icon_size(&self) -> IconSize {
        self.imp().icon_size.get()
    }

    /// Switches the tiles to `size` icons.
    pub(crate) fn set_icon_size(&self, size: IconSize) {
        if self.icon_size() == size {
            return;
        }
        self.apply_icon_size(size);
        self.fit_columns();
    }

    /// Sizes the cells for text of `size`.
    pub(crate) fn set_text_size(&self, size: TextSize) {
        self.imp().text_size.set(size);
        self.fit_columns();
    }

    /// Gives the grid the columns its width holds at the current icon and
    /// text size (see [`CellSize::columns_in`]).
    pub(crate) fn fit_columns(&self) {
        let imp = self.imp();
        let cell = cell_size(imp.icon_size.get(), imp.text_size.get());
        let columns = cell.columns_in(imp.scroller.width());
        if imp.grid.max_columns() != columns {
            imp.grid.set_max_columns(columns);
        }
    }

    /// Gives the grid tiles of `size` icons: their CSS class, which widens
    /// the tiles, and a factory that draws them.
    fn apply_icon_size(&self, size: IconSize) {
        let imp = self.imp();
        imp.icon_size.set(size);
        for other in IconSize::ALL {
            imp.grid.remove_css_class(other.css_class());
        }
        imp.grid.add_css_class(size.css_class());
        let registries = imp
            .registries
            .get()
            .expect("IconView::new sets the registries first");
        imp.grid.set_factory(Some(&registries.tile_factory(size)));
    }

    /// Keeps the columns at what the view's width holds.
    fn fit_columns_to_width(&self) {
        let horizontal = self.imp().scroller.hadjustment();
        horizontal.connect_page_size_notify(glib::clone!(
            #[weak(rename_to = view)]
            self,
            move |_| view.fit_columns_when_allocated()
        ));
    }

    /// Fits the columns once GTK has finished allocating the view. The page
    /// size changes while GTK allocates the grid, and GTK ignores a resize
    /// the grid queues then, so a window that opened in the icon view kept
    /// one column.
    fn fit_columns_when_allocated(&self) {
        glib::idle_add_local_once(glib::clone!(
            #[weak(rename_to = view)]
            self,
            move || view.fit_columns()
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
        assert_eq!(view.icon_size(), IconSize::Large, "a new view shows Large icons");
        assert!(grid.has_css_class(IconSize::Large.css_class()));
        view.set_icon_size(IconSize::Small);
        assert_eq!(view.icon_size(), IconSize::Small);
        assert!(grid.has_css_class(IconSize::Small.css_class()));
        assert!(!grid.has_css_class(IconSize::Large.css_class()));
    }

    /// parity: VIEW-005
    #[test]
    fn icon_sizes_round_trip_and_shrink() {
        for size in IconSize::ALL {
            assert_eq!(IconSize::from_key(size.as_str()), Some(size));
        }
        let pixels: Vec<i32> = IconSize::ALL.iter().map(|size| size.pixels()).collect();
        assert!(pixels.windows(2).all(|pair| pair[0] > pair[1]));
        assert_eq!(IconSize::Large.pixels(), 56);
    }

    /// parity: VIEW-005
    #[test]
    fn large_icon_cells_are_the_web_grid_cells() {
        let cell = cell_size(IconSize::Large, TextSize::from_percent(100));
        assert_eq!(
            cell,
            CellSize {
                width: 135,
                height: 130
            }
        );
        let larger_text = cell_size(IconSize::Large, TextSize::from_percent(150));
        assert_eq!(
            larger_text,
            CellSize {
                width: 180,
                height: 153
            }
        );
        assert!(cell_size(IconSize::ExtraLarge, TextSize::from_percent(100)).width > cell.width);
    }

    /// A pane width and the columns `renderRows` gives it.
    struct ColumnCase {
        pane_width: i32,
        columns: u32,
    }

    /// parity: VIEW-005
    #[test]
    fn grid_columns_follow_the_width_as_render_rows_counts_them() {
        let large_icon_cell = cell_size(IconSize::Large, TextSize::from_percent(100));
        assert_eq!(large_icon_cell.width, 135, "the web's gridWidth");
        let cases = [
            ColumnCase {
                pane_width: 0,
                columns: 1,
            },
            ColumnCase {
                pane_width: 134,
                columns: 1,
            },
            ColumnCase {
                pane_width: 270,
                columns: 2,
            },
            ColumnCase {
                pane_width: 944,
                columns: 6,
            },
            ColumnCase {
                pane_width: 962,
                columns: 7,
            },
        ];
        for case in cases {
            assert_eq!(
                large_icon_cell.columns_in(case.pane_width),
                case.columns,
                "{} pixels",
                case.pane_width
            );
        }
    }
}
