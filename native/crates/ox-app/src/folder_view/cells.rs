// SPDX-License-Identifier: AGPL-3.0-only
//! Cell widgets shared by the details and icon views.
//!
//! Both views show an item as its icon art beside or above its name
//! ([`FileCell`]). Item icons are colour art whose document paper depends
//! on the theme, so the views keep a registry of bound icons
//! ([`IconCells`]) and redraw them when the appearance or the screen scale
//! changes. Names that are cut off show the full name in a tooltip, as
//! `row.title` does in `desktop/ui/app.js`.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gtk::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{glib, pango};

use crate::folder_view::item::FileItem;
use crate::icons;
use crate::theme::Appearance;

/// The list item behind a factory object (column cells are list items too).
pub(crate) fn list_item(object: &glib::Object) -> &gtk::ListItem {
    object
        .downcast_ref::<gtk::ListItem>()
        .expect("list factories receive list items")
}

/// The file item a list item shows, if bound.
pub(crate) fn bound_item(list_item: &gtk::ListItem) -> Option<FileItem> {
    list_item.item().and_downcast::<FileItem>()
}

/// Shows the full label text in a tooltip only while it is ellipsized.
pub(crate) fn tooltip_when_truncated(label: &gtk::Label) {
    label.set_has_tooltip(true);
    label.connect_query_tooltip(|label, _, _, _, tooltip| {
        if !label.layout().is_ellipsized() {
            return false;
        }
        tooltip.set_text(Some(&label.text()));
        true
    });
}

/// An ellipsized, left-aligned label in the muted colour of the date,
/// type and size columns.
pub(crate) fn dim_cell_label() -> gtk::Label {
    let label = gtk::Label::new(None);
    label.set_xalign(0.0);
    label.set_ellipsize(pango::EllipsizeMode::End);
    label.set_single_line_mode(true);
    label.add_css_class("cell-dim");
    label
}

/// Where a [`FileCell`] is shown.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CellLayout {
    /// A details row: the icon left of a one-line name.
    DetailsRow,
    /// An icon-view tile: the icon above up to two centred lines of name.
    IconTile,
}

mod imp {
    use std::cell::Cell;

    use gtk::glib;
    use gtk::prelude::*;
    use gtk::subclass::prelude::*;

    use super::tooltip_when_truncated;

    /// Private state of [`super::FileCell`].
    #[derive(Debug, Default)]
    pub struct FileCell {
        /// The item's icon art.
        pub image: gtk::Image,
        /// The item's name.
        pub label: gtk::Label,
        /// Icon edge in logical pixels.
        pub icon_size: Cell<i32>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for FileCell {
        const NAME: &'static str = "OxFileCell";
        type Type = super::FileCell;
        type ParentType = gtk::Box;
    }

    impl ObjectImpl for FileCell {
        fn constructed(&self) {
            self.parent_constructed();
            let cell = self.obj();
            cell.append(&self.image);
            cell.append(&self.label);
            tooltip_when_truncated(&self.label);
        }
    }

    impl WidgetImpl for FileCell {}
    impl BoxImpl for FileCell {}
}

glib::wrapper! {
    /// An item's icon and name, as one row of the details view or one tile
    /// of the icon view.
    pub struct FileCell(ObjectSubclass<imp::FileCell>)
        @extends gtk::Box, gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget, gtk::Orientable;
}

impl FileCell {
    /// An empty cell with icons of `icon_size` pixels.
    pub(crate) fn new(layout: CellLayout, icon_size: i32) -> Self {
        let cell: Self = glib::Object::new();
        let imp = cell.imp();
        imp.icon_size.set(icon_size);
        imp.image.set_pixel_size(icon_size);
        match layout {
            CellLayout::DetailsRow => cell.lay_out_as_row(),
            CellLayout::IconTile => cell.lay_out_as_tile(),
        }
        cell
    }

    fn lay_out_as_row(&self) {
        self.set_orientation(gtk::Orientation::Horizontal);
        self.set_spacing(11);
        self.imp().image.add_css_class("row-icon");
        let label = &self.imp().label;
        label.set_xalign(0.0);
        label.set_hexpand(true);
        label.set_ellipsize(pango::EllipsizeMode::End);
        label.set_single_line_mode(true);
    }

    fn lay_out_as_tile(&self) {
        self.set_orientation(gtk::Orientation::Vertical);
        self.set_spacing(8);
        self.set_valign(gtk::Align::Start);
        let label = &self.imp().label;
        label.set_wrap(true);
        label.set_wrap_mode(pango::WrapMode::WordChar);
        label.set_lines(2);
        label.set_ellipsize(pango::EllipsizeMode::End);
        label.set_justify(gtk::Justification::Center);
    }

    /// Shows `item`, drawing its art through `icons`.
    pub(crate) fn bind(&self, item: &FileItem, icons: &IconCells) {
        let imp = self.imp();
        icons.bind(&imp.image, item, imp.icon_size.get());
        imp.label.set_text(&item.entry().name);
    }

    /// Forgets the shown item's art.
    pub(crate) fn unbind(&self, icons: &IconCells) {
        icons.unbind(&self.imp().image);
    }

    /// The name label, for tests of what a view shows.
    #[cfg(test)]
    pub fn name(&self) -> String {
        self.imp().label.text().to_string()
    }

    /// True once art is drawn into the icon.
    #[cfg(test)]
    pub fn has_art(&self) -> bool {
        self.imp().image.paintable().is_some()
    }
}

/// Connects `factory` so every list item shows a [`FileCell`].
pub(crate) fn connect_file_cells(
    factory: &gtk::SignalListItemFactory,
    layout: CellLayout,
    icon_size: i32,
    icons: &Rc<IconCells>,
    owners: &Rc<CellOwners>,
) {
    let registry = Rc::clone(owners);
    factory.connect_setup(move |_, object| {
        let cell = FileCell::new(layout, icon_size);
        let list_item = list_item(object);
        list_item.set_child(Some(&cell));
        registry.register(&cell, list_item);
    });
    let binder = Rc::clone(icons);
    factory.connect_bind(move |_, object| {
        let list_item = list_item(object);
        let cell = list_item.child().and_downcast::<FileCell>();
        if let (Some(item), Some(cell)) = (bound_item(list_item), cell) {
            cell.bind(&item, &binder);
        }
    });
    let binder = Rc::clone(icons);
    factory.connect_unbind(move |_, object| {
        if let Some(cell) = list_item(object).child().and_downcast::<FileCell>() {
            cell.unbind(&binder);
        }
    });
}

/// One bound icon: the image, the item it shows and its logical size.
#[derive(Debug)]
struct BoundIcon {
    image: glib::WeakRef<gtk::Image>,
    item: FileItem,
    size: i32,
}

impl BoundIcon {
    fn shows_in(&self, image: &gtk::Image) -> bool {
        self.image.upgrade().is_some_and(|existing| existing == *image)
    }
}

/// Bound item icons, redrawn when the theme or scale changes.
#[derive(Debug, Default)]
pub(crate) struct IconCells {
    appearance: Cell<Option<Appearance>>,
    bound: RefCell<Vec<BoundIcon>>,
}

impl IconCells {
    /// A registry drawing in `appearance`.
    pub fn new(appearance: Appearance) -> Rc<Self> {
        let cells = Self::default();
        cells.appearance.set(Some(appearance));
        Rc::new(cells)
    }

    fn appearance(&self) -> Appearance {
        self.appearance.get().unwrap_or(Appearance::Light)
    }

    /// Draws `item`'s art into `image` at `size` and remembers the pair.
    pub fn bind(&self, image: &gtk::Image, item: &FileItem, size: i32) {
        let scale = image.scale_factor().max(1);
        icons::set_art(image, item.art(), size, self.appearance(), scale);
        let mut bound = self.bound.borrow_mut();
        bound.retain(|icon| !icon.shows_in(image) && icon.image.upgrade().is_some());
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
        if self.appearance.replace(Some(appearance)) == Some(appearance) {
            return;
        }
        self.redraw();
    }

    /// Redraws every bound icon (for example after a scale change).
    pub fn redraw(&self) {
        let appearance = self.appearance();
        for icon in self.bound.borrow().iter() {
            if let Some(image) = icon.image.upgrade() {
                let scale = image.scale_factor().max(1);
                icons::set_art(&image, icon.item.art(), icon.size, appearance, scale);
            }
        }
    }
}

/// Maps cell widgets back to their list items, so a click position can be
/// turned into a row (GTK 4.14 has no public hit-test for list views).
#[derive(Debug, Default)]
pub(crate) struct CellOwners {
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

    /// The list item whose content widget is `widget`, one of its first
    /// two levels of children, or the nearest such ancestor below `view`.
    fn owner_near(&self, view: &gtk::Widget, widget: gtk::Widget) -> Option<gtk::ListItem> {
        let mut current = Some(widget);
        while let Some(widget) = current {
            if widget == *view {
                return None;
            }
            let first = widget.first_child();
            let second = first.as_ref().and_then(WidgetExt::first_child);
            let candidates = [Some(widget.clone()), first, second];
            let owner = candidates
                .iter()
                .flatten()
                .find_map(|candidate| self.owner_of(candidate));
            if owner.is_some() {
                return owner;
            }
            current = widget.parent();
        }
        None
    }

    /// The position of the item under (`x`, `y`) in `view`'s coordinates,
    /// or `None` over empty space. Clicks in a row's padding count too.
    pub fn position_at(&self, view: &impl IsA<gtk::Widget>, x: f64, y: f64) -> Option<u32> {
        let view = view.as_ref();
        let picked = view.pick(x, y, gtk::PickFlags::DEFAULT)?;
        let list_item = self.owner_near(view, picked)?;
        let position = list_item.position();
        let is_bound = position != gtk::INVALID_LIST_POSITION && list_item.item().is_some();
        is_bound.then_some(position)
    }

    /// The content widget showing `position`, if it is on screen.
    pub fn widget_at(&self, position: u32) -> Option<gtk::Widget> {
        self.owners.borrow().iter().find_map(|(widget, item)| {
            let item = item.upgrade()?;
            let is_bound = item.position() == position && item.item().is_some();
            is_bound.then(|| widget.upgrade()).flatten()
        })
    }
}
