// SPDX-License-Identifier: AGPL-3.0-only
//! Cell widgets shared by the details and icon views.
//!
//! Item icons are colour art whose document paper depends on the theme, so
//! the views keep a registry of bound icons ([`IconCells`]) and redraw them
//! when the appearance or the screen scale changes. Names that are cut off
//! show the full name in a tooltip, as `row.title` does in app.js.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gtk::prelude::*;
use gtk::{glib, pango};

use crate::folder_view::item::FileItem;
use crate::icons;
use crate::theme::Appearance;

/// The list item behind a factory object (column cells are list items too).
pub fn list_item(object: &glib::Object) -> &gtk::ListItem {
    object
        .downcast_ref::<gtk::ListItem>()
        .expect("list factories receive list items")
}

/// The file item a list item shows, if bound.
pub fn bound_item(list_item: &gtk::ListItem) -> Option<FileItem> {
    list_item.item().and_downcast::<FileItem>()
}

/// Shows the full label text in a tooltip only while it is ellipsized.
pub fn tooltip_when_truncated(label: &gtk::Label) {
    label.set_has_tooltip(true);
    label.connect_query_tooltip(|label, _, _, _, tooltip| {
        if !label.layout().is_ellipsized() {
            return false;
        }
        tooltip.set_text(Some(&label.text()));
        true
    });
}

/// An ellipsized, left-aligned label for a column cell.
pub fn cell_label(dim: bool) -> gtk::Label {
    let label = gtk::Label::new(None);
    label.set_xalign(0.0);
    label.set_ellipsize(pango::EllipsizeMode::End);
    label.set_single_line_mode(true);
    if dim {
        label.add_css_class("cell-dim");
    }
    label
}

/// Bound item icons, redrawn when the theme or scale changes.
#[derive(Default)]
pub struct IconCells {
    appearance: Cell<Option<Appearance>>,
    scale: Cell<i32>,
    bound: RefCell<Vec<(glib::WeakRef<gtk::Image>, FileItem, i32)>>,
}

impl IconCells {
    /// A registry drawing in `appearance`.
    pub fn new(appearance: Appearance) -> Rc<Self> {
        let cells = Self::default();
        cells.appearance.set(Some(appearance));
        cells.scale.set(1);
        Rc::new(cells)
    }

    fn appearance(&self) -> Appearance {
        self.appearance.get().unwrap_or(Appearance::Light)
    }

    /// Draws `item`'s art into `image` at `size` and remembers the pair.
    pub fn bind(&self, image: &gtk::Image, item: &FileItem, size: i32) {
        let scale = image.scale_factor().max(1);
        self.scale.set(scale);
        icons::set_art(image, item.art(), size, self.appearance(), scale);
        let mut bound = self.bound.borrow_mut();
        bound.retain(|(weak, _, _)| weak.upgrade().is_some_and(|existing| existing != *image));
        bound.push((image.downgrade(), item.clone(), size));
    }

    /// Forgets `image` when its row is unbound.
    pub fn unbind(&self, image: &gtk::Image) {
        self.bound
            .borrow_mut()
            .retain(|(weak, _, _)| weak.upgrade().is_some_and(|existing| existing != *image));
    }

    /// Redraws every bound icon in a new appearance.
    pub fn set_appearance(&self, appearance: Appearance) {
        if self.appearance.replace(Some(appearance)) == Some(appearance) {
            return;
        }
        self.redraw();
    }

    /// Redraws every bound icon (for example after a scale change).
    pub fn redraw(&self) {
        let appearance = self.appearance();
        for (weak, item, size) in self.bound.borrow().iter() {
            if let Some(image) = weak.upgrade() {
                let scale = image.scale_factor().max(1);
                icons::set_art(&image, item.art(), *size, appearance, scale);
            }
        }
    }
}

/// Maps cell widgets back to their list items, so a click position can be
/// turned into a row (GTK 4.14 has no public hit-test for list views).
#[derive(Default)]
pub struct CellOwners {
    owners: RefCell<Vec<(glib::WeakRef<gtk::Widget>, glib::WeakRef<gtk::ListItem>)>>,
}

impl CellOwners {
    /// A shared, empty registry.
    pub fn new() -> Rc<Self> {
        Rc::new(Self::default())
    }

    /// Records that `child` is the content widget of `list_item`.
    pub fn register(&self, child: &impl IsA<gtk::Widget>, list_item: &gtk::ListItem) {
        let mut owners = self.owners.borrow_mut();
        owners.retain(|(widget, item)| widget.upgrade().is_some() && item.upgrade().is_some());
        owners.push((child.as_ref().downgrade(), list_item.downgrade()));
    }

    fn owner_of(&self, widget: &gtk::Widget) -> Option<gtk::ListItem> {
        self.owners
            .borrow()
            .iter()
            .find(|(candidate, _)| candidate.upgrade().is_some_and(|candidate| candidate == *widget))
            .and_then(|(_, item)| item.upgrade())
    }

    /// The position of the item under (`x`, `y`) in `view`'s coordinates,
    /// or `None` over empty space. Clicks in a row's padding count too:
    /// each ancestor's first and second-level children are checked.
    pub fn position_at(&self, view: &impl IsA<gtk::Widget>, x: f64, y: f64) -> Option<u32> {
        let view = view.as_ref();
        let mut current = view.pick(x, y, gtk::PickFlags::DEFAULT);
        while let Some(widget) = current {
            if widget == *view {
                break;
            }
            let first = widget.first_child();
            let second = first.as_ref().and_then(|child| child.first_child());
            let candidates = [Some(widget.clone()), first, second];
            let owner = candidates
                .iter()
                .flatten()
                .find_map(|candidate| self.owner_of(candidate));
            if let Some(list_item) = owner {
                let position = list_item.position();
                let valid = position != gtk::INVALID_LIST_POSITION && list_item.item().is_some();
                return valid.then_some(position);
            }
            current = widget.parent();
        }
        None
    }
}
