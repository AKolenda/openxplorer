// SPDX-License-Identifier: AGPL-3.0-only
//! The item icons the views have bound, redrawn when the theme or the
//! screen scale changes.
//!
//! Item icons are colour art whose document paper depends on the theme.
//! In the web app they are SVG coloured by the theme's CSS variables
//! (`--doc-paper` in `desktop/ui/style.css`), so they follow a theme
//! change by themselves. Here they are drawn into textures at the screen's
//! scale, so the registry remembers each bound image and draws it again.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gtk::glib;
use gtk::prelude::*;

use crate::folder_view::item::FileItem;
use crate::icons;
use crate::theme::Appearance;

/// One bound icon: the image, the item it shows and its logical size.
#[derive(Debug)]
struct BoundIcon {
    image: glib::WeakRef<gtk::Image>,
    item: FileItem,
    size: i32,
}

impl BoundIcon {
    /// True when this icon is drawn into `image`.
    fn shows_in(&self, image: &gtk::Image) -> bool {
        self.image.upgrade().is_some_and(|existing| existing == *image)
    }

    /// True while its image still exists.
    fn is_alive(&self) -> bool {
        self.image.upgrade().is_some()
    }
}

/// Bound item icons, redrawn when the theme or scale changes.
#[derive(Debug)]
pub(crate) struct IconCells {
    appearance: Cell<Appearance>,
    bound: RefCell<Vec<BoundIcon>>,
}

impl IconCells {
    /// A registry drawing in `appearance`, shared by the views' cell
    /// factories.
    pub fn new(appearance: Appearance) -> Rc<Self> {
        Rc::new(Self {
            appearance: Cell::new(appearance),
            bound: RefCell::default(),
        })
    }

    /// Draws `item`'s art into `image` at `size` and remembers the pair.
    pub fn bind(&self, image: &gtk::Image, item: &FileItem, size: i32) {
        draw_art(image, item, size, self.appearance.get());
        let mut bound = self.bound.borrow_mut();
        // A recycled image shows only its new item; icons whose image is
        // gone are dropped on the way.
        bound.retain(|icon| !icon.shows_in(image) && icon.is_alive());
        bound.push(BoundIcon {
            image: image.downgrade(),
            item: item.clone(),
            size,
        });
    }

    /// Forgets `image` when its row is unbound.
    pub fn unbind(&self, image: &gtk::Image) {
        self.bound.borrow_mut().retain(|icon| !icon.shows_in(image));
    }

    /// Redraws every bound icon in a new appearance.
    pub fn set_appearance(&self, appearance: Appearance) {
        if self.appearance.replace(appearance) == appearance {
            return;
        }
        self.redraw();
    }

    /// Redraws every bound icon (for example after a scale change).
    pub fn redraw(&self) {
        let appearance = self.appearance.get();
        for icon in self.bound.borrow().iter() {
            if let Some(image) = icon.image.upgrade() {
                draw_art(&image, &icon.item, icon.size, appearance);
            }
        }
    }
}

/// Draws `item`'s art into `image` at `size` logical pixels, sharp at the
/// image's screen scale.
fn draw_art(image: &gtk::Image, item: &FileItem, size: i32, appearance: Appearance) {
    let scale = image.scale_factor().max(1);
    icons::set_art(image, item.art(), size, appearance, scale);
}
