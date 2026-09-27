// SPDX-License-Identifier: AGPL-3.0-only
//! The icon view: tiles with a large icon and up to two lines of name.
//!
//! Matches `.file-tile` in style.css ("Large icons": a 56 pixel icon in a
//! 135 pixel cell). Explorer's other icon sizes (Ctrl+Shift+1..4) use the
//! same tiles with a different icon size.

use std::rc::Rc;

use gtk::pango;
use gtk::prelude::*;

use crate::folder_view::cells::{self, CellOwners, IconCells};
use crate::folder_view::model::FolderModel;

/// Icon sizes of Explorer's icon layouts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IconSize {
    /// Ctrl+Shift+1.
    ExtraLarge,
    /// Ctrl+Shift+2, the web interface's "Large icons".
    Large,
    /// Ctrl+Shift+3.
    Medium,
    /// Ctrl+Shift+4.
    Small,
}

impl IconSize {
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

    /// Every size, largest first.
    pub const ALL: [IconSize; 4] = [
        IconSize::ExtraLarge,
        IconSize::Large,
        IconSize::Medium,
        IconSize::Small,
    ];

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
    let pixels = size.pixels();
    let factory = gtk::SignalListItemFactory::new();
    let registry = Rc::clone(owners);
    factory.connect_setup(move |_, object| {
        let tile = gtk::Box::new(gtk::Orientation::Vertical, 8);
        tile.set_valign(gtk::Align::Start);
        let image = gtk::Image::new();
        image.set_pixel_size(pixels);
        let label = gtk::Label::new(None);
        label.set_wrap(true);
        label.set_wrap_mode(pango::WrapMode::WordChar);
        label.set_lines(2);
        label.set_ellipsize(pango::EllipsizeMode::End);
        label.set_justify(gtk::Justification::Center);
        label.set_max_width_chars(14);
        cells::tooltip_when_truncated(&label);
        tile.append(&image);
        tile.append(&label);
        let list_item = cells::list_item(object);
        list_item.set_child(Some(&tile));
        registry.register(&tile, list_item);
    });
    let binder = Rc::clone(icons);
    factory.connect_bind(move |_, object| {
        let list_item = cells::list_item(object);
        let (Some(item), Some(tile)) = (cells::bound_item(list_item), list_item.child()) else {
            return;
        };
        let image = tile.first_child().and_downcast::<gtk::Image>();
        let label = image
            .as_ref()
            .and_then(|image| image.next_sibling())
            .and_downcast::<gtk::Label>();
        if let (Some(image), Some(label)) = (image, label) {
            binder.bind(&image, &item, pixels);
            label.set_text(&item.entry().name);
        }
    });
    let binder = Rc::clone(icons);
    factory.connect_unbind(move |_, object| {
        let image = cells::list_item(object)
            .child()
            .and_then(|tile| tile.first_child())
            .and_downcast::<gtk::Image>();
        if let Some(image) = image {
            binder.unbind(&image);
        }
    });
    factory
}

/// Builds the grid view over `model`.
pub fn build(
    model: &FolderModel,
    icons: &Rc<IconCells>,
    owners: &Rc<CellOwners>,
    size: IconSize,
) -> gtk::GridView {
    let view = gtk::GridView::new(
        Some(model.selection().clone()),
        Some(factory(icons, owners, size)),
    );
    view.add_css_class("files");
    view.add_css_class(size.css_class());
    view.set_enable_rubberband(true);
    view.set_max_columns(64);
    view.set_tab_behavior(gtk::ListTabBehavior::Item);
    view
}

/// Switches the grid to another icon size.
pub fn set_icon_size(view: &gtk::GridView, icons: &Rc<IconCells>, owners: &Rc<CellOwners>, size: IconSize) {
    for other in IconSize::ALL {
        view.remove_css_class(other.css_class());
    }
    view.add_css_class(size.css_class());
    view.set_factory(Some(&factory(icons, owners, size)));
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
}
