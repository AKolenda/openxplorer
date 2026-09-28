// SPDX-License-Identifier: AGPL-3.0-only
//! The icon view: tiles with a large icon and up to two lines of name.
//!
//! Matches `.file-tile` in `desktop/ui/style.css` ("Large icons": a 56
//! pixel icon in a 135 pixel cell). Explorer's other icon layouts use the
//! same tiles with a different icon size; the window binds them to
//! Ctrl+Shift+1..4 as Explorer does.

use std::rc::Rc;

use gtk::prelude::*;

use crate::folder_view::cells::{self, CellLayout, CellOwners, IconCells};
use crate::text_size;

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
    pub const ALL: [IconSize; 4] = [
        IconSize::ExtraLarge,
        IconSize::Large,
        IconSize::Medium,
        IconSize::Small,
    ];

    /// Icon edge in logical pixels.
    pub const fn pixels(self) -> i32 {
        match self {
            IconSize::ExtraLarge => 96,
            IconSize::Large => 56,
            IconSize::Medium => 40,
            IconSize::Small => 28,
        }
    }

    /// Menu label.
    pub const fn label(self) -> &'static str {
        match self {
            IconSize::ExtraLarge => "Extra large icons",
            IconSize::Large => "Large icons",
            IconSize::Medium => "Medium icons",
            IconSize::Small => "Small icons",
        }
    }

    /// Action-state key.
    pub const fn key(self) -> &'static str {
        match self {
            IconSize::ExtraLarge => "extra-large",
            IconSize::Large => "large",
            IconSize::Medium => "medium",
            IconSize::Small => "small",
        }
    }

    /// Explorer's shortcut for the layout.
    pub const fn accelerator(self) -> &'static str {
        match self {
            IconSize::ExtraLarge => "<Primary><Shift>1",
            IconSize::Large => "<Primary><Shift>2",
            IconSize::Medium => "<Primary><Shift>3",
            IconSize::Small => "<Primary><Shift>4",
        }
    }

    /// The size for an action-state key.
    pub fn from_key(key: &str) -> Option<IconSize> {
        Self::ALL.into_iter().find(|size| size.key() == key)
    }

    /// CSS class that widens tiles for large icons.
    pub const fn css_class(self) -> &'static str {
        match self {
            IconSize::ExtraLarge => "icons-extra-large",
            IconSize::Large => "icons-large",
            IconSize::Medium => "icons-medium",
            IconSize::Small => "icons-small",
        }
    }
}

/// The icon view's tiles: `size` icons above their names.
fn factory(icons: &Rc<IconCells>, owners: &Rc<CellOwners>, size: IconSize) -> gtk::SignalListItemFactory {
    let factory = gtk::SignalListItemFactory::new();
    cells::connect_file_cells(&factory, CellLayout::IconTile, size.pixels(), icons, owners);
    factory
}

/// Builds the icon view. It shows no model until the window makes it the
/// visible view.
pub(crate) fn build(icons: &Rc<IconCells>, owners: &Rc<CellOwners>, size: IconSize) -> gtk::GridView {
    let view = gtk::GridView::new(None::<gtk::MultiSelection>, Some(factory(icons, owners, size)));
    view.add_css_class("files");
    view.add_css_class(size.css_class());
    view.set_enable_rubberband(true);
    view.set_tab_behavior(gtk::ListTabBehavior::Item);
    view
}

/// Switches the grid to another icon size.
pub(crate) fn set_icon_size(
    view: &gtk::GridView,
    icons: &Rc<IconCells>,
    owners: &Rc<CellOwners>,
    size: IconSize,
) {
    for other in IconSize::ALL {
        view.remove_css_class(other.css_class());
    }
    view.add_css_class(size.css_class());
    view.set_factory(Some(&factory(icons, owners, size)));
}

/// An icon-view cell in pixels: a tile plus the gap to its neighbours.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct CellSize {
    /// The narrowest a cell may be.
    pub width: i32,
    /// The row pitch.
    pub height: i32,
}

/// The cell of tiles of `size` at `text_size` percent: `gridWidth` ×
/// `gridRow` from `metrics()` in text-size.js for large icons (135 × 130
/// at 100%), widened and heightened with the icon for the other sizes,
/// which the Python app does not have.
pub(crate) fn cell_size(size: IconSize, text_size: u32) -> CellSize {
    let metrics = text_size::metrics(text_size);
    let icon_growth = size.pixels() - IconSize::Large.pixels();
    let width_for_icon = size.pixels() + TILE_WIDTH_BEYOND_ICON;
    CellSize {
        width: metrics.grid_width.max(width_for_icon),
        height: metrics.grid_row + icon_growth,
    }
}

/// How many columns of `cell_width` a `width`-pixel pane shows:
/// `max(1, floor(clientWidth / gridWidth))`, as `renderRows` in app.js
/// counts them. The tiles then share the pane's width less its 20 pixels
/// of inset, so they can be a little narrower than a cell.
///
/// GTK keeps tiles for about thirty rows of `max-columns` alive, so the
/// window sets exactly this many columns rather than a generous cap,
/// which made every listing build thousands of tiles.
pub(crate) fn columns_for_width(cell_width: i32, width: i32) -> u32 {
    let columns = width / cell_width.max(1);
    u32::try_from(columns.max(1)).unwrap_or(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// parity: VIEW-005
    #[test]
    fn icon_sizes_round_trip_and_shrink() {
        for size in IconSize::ALL {
            assert_eq!(IconSize::from_key(size.key()), Some(size));
        }
        let pixels: Vec<i32> = IconSize::ALL.iter().map(|size| size.pixels()).collect();
        assert!(pixels.windows(2).all(|pair| pair[0] > pair[1]));
        assert_eq!(IconSize::Large.pixels(), 56);
    }

    /// parity: VIEW-005
    #[test]
    fn large_icon_cells_are_the_web_grid_cells() {
        let cell = cell_size(IconSize::Large, 100);
        assert_eq!(
            cell,
            CellSize {
                width: 135,
                height: 130
            }
        );
        let larger_text = cell_size(IconSize::Large, 150);
        assert_eq!(
            larger_text,
            CellSize {
                width: 180,
                height: 153
            }
        );
        assert!(cell_size(IconSize::ExtraLarge, 100).width > cell.width);
    }

    /// A pane width and the columns `renderRows` gives it.
    struct ColumnCase {
        width: i32,
        columns: u32,
    }

    /// parity: VIEW-005
    #[test]
    fn grid_columns_follow_the_width_as_render_rows_counts_them() {
        let cases = [
            ColumnCase { width: 0, columns: 1 },
            ColumnCase {
                width: 134,
                columns: 1,
            },
            ColumnCase {
                width: 270,
                columns: 2,
            },
            ColumnCase {
                width: 944,
                columns: 6,
            },
            ColumnCase {
                width: 962,
                columns: 7,
            },
        ];
        for case in cases {
            assert_eq!(
                columns_for_width(135, case.width),
                case.columns,
                "{} pixels",
                case.width
            );
        }
    }
}
