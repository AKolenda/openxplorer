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
use crate::theme::narrowest_tile_width;

/// Icon sizes of Explorer's icon layouts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IconSize {
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

/// The most columns tiles of `size` can fill in `width` pixels.
///
/// GTK keeps tiles for about thirty rows of `max-columns` alive, so a
/// generous fixed cap (64 columns) made every listing build thousands of
/// tiles. Bounding it by the width keeps the live tiles near what is
/// visible without ever limiting the columns a wide window shows.
pub(crate) fn columns_for_width(size: IconSize, width: f64) -> u32 {
    let tile = f64::from(narrowest_tile_width(size));
    let columns = (width / tile).floor().max(1.0);
    // At most a few hundred columns fit on any screen.
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "bounded above"
    )]
    let columns = columns.min(512.0) as u32;
    columns
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn icon_sizes_round_trip_and_shrink() {
        for size in IconSize::ALL {
            assert_eq!(IconSize::from_key(size.key()), Some(size));
        }
        let pixels: Vec<i32> = IconSize::ALL.iter().map(|size| size.pixels()).collect();
        assert!(pixels.windows(2).all(|pair| pair[0] > pair[1]));
        assert_eq!(IconSize::Large.pixels(), 56);
    }

    #[test]
    fn grid_columns_follow_the_width_and_never_reach_zero() {
        let large = f64::from(narrowest_tile_width(IconSize::Large));
        assert_eq!(columns_for_width(IconSize::Large, 0.0), 1);
        assert_eq!(columns_for_width(IconSize::Large, large * 3.5), 3);
        assert!(columns_for_width(IconSize::Small, 1920.0) > columns_for_width(IconSize::ExtraLarge, 1920.0));
        assert!(columns_for_width(IconSize::Small, 1920.0) < 64);
    }
}
