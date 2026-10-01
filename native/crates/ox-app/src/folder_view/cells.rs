// SPDX-License-Identifier: AGPL-3.0-only
//! Cell widgets shared by the details and icon views.
//!
//! Both views show an item as its icon art beside or above its name
//! ([`FileCell`]), as the name cell of `renderRows` in `v2.0.0:desktop/ui/app.js`
//! does. Every cell shows its row's tooltip ([`row_tooltip`]), as
//! `row.title` does. While the item is renamed in place, a text field
//! takes the name's place. [`CellOwners`] follows the cells the views
//! bind and turns a click position back into a row. The art is an [`ArtImage`] of
//! bundled icons, which GTK renders again by itself when the theme or the
//! screen scale changes.

mod cell_owners;
mod custom_icon;
mod row_tooltip;

use std::rc::Rc;

use gtk::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{glib, pango};

pub(crate) use cell_owners::CellOwners;
pub(crate) use custom_icon::CUSTOM_ICON;
pub(crate) use row_tooltip::{show_row_tooltip, CellTooltip, RowTooltip};

use crate::folder_view::item::FileItem;
use crate::icons::Art;

/// Gap between a row's icon and its name (`.name-cell{gap:11px}`).
const ROW_ICON_GAP: i32 = 11;

/// Gap between a tile's icon and its name (`.file-tile{gap:8px}`).
const TILE_ICON_GAP: i32 = 8;

/// Lines of name a tile shows (`.tile-name{max-height:33px}` at a 1.35
/// line height).
const TILE_NAME_LINES: i32 = 2;

/// The list item behind a factory object (column cells are list items too).
///
/// # Panics
///
/// If `object` is not a [`gtk::ListItem`]; GTK hands list factories
/// nothing else.
pub(crate) fn as_list_item(object: &glib::Object) -> &gtk::ListItem {
    object
        .downcast_ref::<gtk::ListItem>()
        .expect("list factories receive list items")
}

/// The file item a list item shows, if bound.
pub(crate) fn bound_item(list_item: &gtk::ListItem) -> Option<FileItem> {
    list_item.item().and_downcast::<FileItem>()
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
    use std::cell::{Cell, RefCell};

    use gtk::glib;
    use gtk::prelude::*;
    use gtk::subclass::prelude::*;

    use crate::icons::ArtImage;

    /// Private state of [`super::FileCell`].
    #[derive(Debug, Default)]
    pub(crate) struct FileCell {
        /// The item's icon art.
        pub(super) image: ArtImage,
        /// The item's name.
        pub(super) label: gtk::Label,
        /// Icon edge in logical pixels.
        pub(super) icon_size: Cell<i32>,
        /// The text field in the name's place while the item is renamed.
        pub(super) name_editor: RefCell<Option<gtk::Entry>>,
        /// The item's custom icon, shown in place of the art (PROP-016).
        pub(super) custom_icon: gtk::Picture,
        /// Counts the lookups, so a late custom icon is dropped.
        pub(super) icon_lookup: Cell<u64>,
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
            self.custom_icon.set_content_fit(gtk::ContentFit::Contain);
            self.custom_icon.set_visible(false);
            cell.append(&self.custom_icon);
            cell.append(&self.label);
        }
    }

    impl WidgetImpl for FileCell {}
    impl BoxImpl for FileCell {}
}

glib::wrapper! {
    /// An item's icon and name, as one row of the details view or one tile
    /// of the icon view.
    pub(crate) struct FileCell(ObjectSubclass<imp::FileCell>)
        @extends gtk::Box, gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget, gtk::Orientable;
}

impl FileCell {
    /// An empty cell laid out for `layout`, with icons of `icon_size`
    /// logical pixels.
    pub(crate) fn new(layout: CellLayout, icon_size: i32) -> Self {
        let cell: Self = glib::Object::new();
        let imp = cell.imp();
        imp.icon_size.set(icon_size);
        imp.custom_icon.set_size_request(icon_size, icon_size);
        // A folder until bound, so the cell has its full size from the start.
        imp.image.set_art(Art::Folder, icon_size);
        match layout {
            CellLayout::DetailsRow => cell.lay_out_as_row(),
            CellLayout::IconTile => cell.lay_out_as_tile(),
        }
        cell
    }

    /// The icon left of a one-line name that is cut off with an ellipsis.
    fn lay_out_as_row(&self) {
        self.set_orientation(gtk::Orientation::Horizontal);
        self.set_spacing(ROW_ICON_GAP);
        self.imp().image.add_css_class("row-icon");
        let label = &self.imp().label;
        label.set_xalign(0.0);
        label.set_hexpand(true);
        label.set_ellipsize(pango::EllipsizeMode::End);
        label.set_single_line_mode(true);
    }

    /// The icon above a centred name wrapped to [`TILE_NAME_LINES`].
    fn lay_out_as_tile(&self) {
        self.set_orientation(gtk::Orientation::Vertical);
        self.set_spacing(TILE_ICON_GAP);
        self.set_valign(gtk::Align::Start);
        let label = &self.imp().label;
        label.set_wrap(true);
        label.set_wrap_mode(pango::WrapMode::WordChar);
        label.set_lines(TILE_NAME_LINES);
        label.set_ellipsize(pango::EllipsizeMode::End);
        label.set_justify(gtk::Justification::Center);
    }

    /// Shows `item`: its art and its name. A rename in place ends: the
    /// cell now shows another item, or the same one listed again.
    pub(crate) fn bind(&self, item: &FileItem) {
        self.hide_name_editor();
        let imp = self.imp();
        let icon_size = imp.icon_size.get();
        imp.image.set_art(item.art(), icon_size);
        imp.image.set_emblems(item.emblems(), icon_size);
        imp.label.set_text(&item.entry().name);
    }

    /// Puts `editor` in the name's place, to rename the item in place.
    pub(crate) fn show_name_editor(&self, editor: &gtk::Entry) {
        self.hide_name_editor();
        let imp = self.imp();
        self.insert_child_after(editor, Some(&imp.label));
        imp.label.set_visible(false);
        imp.name_editor.replace(Some(editor.clone()));
    }

    /// Shows the name again in place of the text field, if one shows.
    pub(crate) fn hide_name_editor(&self) {
        let imp = self.imp();
        let Some(editor) = imp.name_editor.take() else {
            return;
        };
        self.remove(&editor);
        imp.label.set_visible(true);
    }

    /// The name label, for tests of what a view shows.
    #[cfg(test)]
    pub(crate) fn name(&self) -> String {
        self.imp().label.text().to_string()
    }

    /// The art the icon shows, for tests.
    #[cfg(test)]
    pub(crate) fn art(&self) -> Option<Art> {
        self.imp().image.art()
    }

    /// The emblems the icon shows, for tests.
    #[cfg(test)]
    pub(crate) fn emblems(&self) -> crate::icons::Emblems {
        self.imp().image.shown_emblems()
    }
}

/// Names a file view for screen readers as the current app names its
/// file list (`#file-canvas` and `#main` in index.html).
pub(crate) fn label_view(view: &gtk::Widget) {
    view.update_property(&[
        gtk::accessible::Property::Label("Files"),
        gtk::accessible::Property::Description("Folder contents — type a filename prefix to select"),
    ]);
}

/// Connects `factory` so every list item shows a [`FileCell`] in
/// `layout`, with icons of `icon_size` logical pixels, its row's tooltip,
/// and each cell registered in `owners`, which dims the cells of cut
/// items.
pub(crate) fn connect_file_cells(
    factory: &gtk::SignalListItemFactory,
    layout: CellLayout,
    icon_size: i32,
    owners: &Rc<CellOwners>,
) {
    let setup_owners = Rc::clone(owners);
    factory.connect_setup(move |_, object| {
        let cell = FileCell::new(layout, icon_size);
        let list_item = as_list_item(object);
        list_item.set_child(Some(&cell));
        setup_owners.register(&cell, list_item);
        show_row_tooltip(&cell, &setup_owners, |_| None);
    });
    let bind_owners = Rc::clone(owners);
    factory.connect_bind(move |_, object| {
        let list_item = as_list_item(object);
        let cell = list_item.child().and_downcast::<FileCell>();
        if let (Some(item), Some(cell)) = (bound_item(list_item), cell) {
            // A tile is named after its item (`aria-label` in app.js).
            list_item.set_accessible_label(&item.entry().name);
            cell.bind(&item);
            cell.look_up_custom_icon(&item);
            bind_owners.style_cell(&cell, &item);
        }
    });
}
