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
mod expander;
mod item_check;
pub(crate) use item_check::own_clicks;
mod row_tooltip;

use std::rc::Rc;

use gtk::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{glib, pango};

pub(crate) use cell_owners::CellOwners;
pub(crate) use custom_icon::CUSTOM_ICON;
pub(crate) use expander::connect_expanders;
pub(crate) use row_tooltip::{show_row_tooltip, CellTooltip, RowTooltip};

use crate::folder_view::item::FileItem;
use crate::icons::Art;

/// Gap between a row's icon and its name (`.name-cell{gap:11px}`).
const ROW_ICON_GAP: i32 = 11;

/// Gap between a tile's icon and its name (`.file-tile{gap:8px}`).
const TILE_ICON_GAP: i32 = 8;

/// Gap between a compact item's icon and its name.
const COMPACT_ICON_GAP: i32 = 6;

/// How wide a compact item's name is, in characters: the columns are all
/// this wide, and a longer name is cut off with an ellipsis (Dolphin's
/// Compact view caps its columns the same way).
const COMPACT_NAME_CHARS: i32 = 28;

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
    /// A compact-view item: a small icon left of a name
    /// [`COMPACT_NAME_CHARS`] wide (VIEW-008).
    CompactItem,
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
        /// The item's custom icon (PROP-016) or preview (VIEW-057), shown
        /// in place of the art.
        pub(super) custom_icon: gtk::Picture,
        /// The running lookup of the custom icon or preview.
        pub(super) picture_lookup: RefCell<Option<glib::JoinHandle<()>>>,
        /// Holds the icon, and on a tile the check box over its corner.
        pub(super) icon_frame: gtk::Overlay,
        /// Checked while the item is selected; a click selects or
        /// deselects it alone (SEL-014).
        pub(super) check: gtk::CheckButton,
        /// Expands a folder of a details row in place (VIEW-035).
        pub(super) expander: gtk::Button,
        /// The tree row the cell follows, and its handler.
        pub(super) tree_row: RefCell<Option<(gtk::TreeListRow, glib::SignalHandlerId)>>,
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
            self.icon_frame.set_child(Some(&self.image));
            self.custom_icon.set_content_fit(gtk::ContentFit::Contain);
            self.custom_icon.set_visible(false);
            self.icon_frame.add_overlay(&self.custom_icon);
            cell.append(&self.icon_frame);
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
            CellLayout::CompactItem => cell.lay_out_as_compact_item(),
        }
        cell
    }

    /// The check box and the icon left of a one-line name that is cut off
    /// with an ellipsis.
    fn lay_out_as_row(&self) {
        self.set_orientation(gtk::Orientation::Horizontal);
        self.set_spacing(ROW_ICON_GAP);
        self.put_check_before_icon();
        self.imp().image.add_css_class("row-icon");
        let label = &self.imp().label;
        label.set_xalign(0.0);
        label.set_hexpand(true);
        label.set_ellipsize(pango::EllipsizeMode::End);
        label.set_single_line_mode(true);
    }

    /// The check box and the icon left of a one-line name
    /// [`COMPACT_NAME_CHARS`] wide.
    fn lay_out_as_compact_item(&self) {
        self.set_orientation(gtk::Orientation::Horizontal);
        self.set_spacing(COMPACT_ICON_GAP);
        self.put_check_before_icon();
        let label = &self.imp().label;
        label.set_xalign(0.0);
        label.set_ellipsize(pango::EllipsizeMode::End);
        label.set_single_line_mode(true);
        label.set_width_chars(COMPACT_NAME_CHARS);
        label.set_max_width_chars(COMPACT_NAME_CHARS);
    }

    /// The icon above a centred name wrapped to [`TILE_NAME_LINES`].
    fn lay_out_as_tile(&self) {
        self.set_orientation(gtk::Orientation::Vertical);
        self.set_spacing(TILE_ICON_GAP);
        self.set_valign(gtk::Align::Start);
        // The check box sits on the icon's corner, not the tile's.
        let imp = self.imp();
        imp.icon_frame.set_halign(gtk::Align::Center);
        imp.check.set_halign(gtk::Align::Start);
        imp.check.set_valign(gtk::Align::Start);
        imp.check.add_css_class("on-icon");
        imp.icon_frame.add_overlay(&imp.check);
        let label = &self.imp().label;
        label.set_wrap(true);
        label.set_wrap_mode(pango::WrapMode::WordChar);
        label.set_lines(TILE_NAME_LINES);
        label.set_ellipsize(pango::EllipsizeMode::End);
        label.set_justify(gtk::Justification::Center);
    }

    /// Puts the check box first in a row, centred on the line.
    fn put_check_before_icon(&self) {
        let check = &self.imp().check;
        check.set_valign(gtk::Align::Center);
        self.prepend(check);
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

    /// Whether `point`, in the cell's coordinates, is on the name's text
    /// rather than beside it.
    pub(crate) fn name_text_contains(&self, point: gtk::graphene::Point) -> bool {
        let label = &self.imp().label;
        let Some(point) = self.compute_point(label, &point).filter(|_| label.is_visible()) else {
            return false;
        };
        let (left, top) = label.layout_offsets();
        let (width, height) = label.layout().pixel_size();
        #[expect(clippy::cast_precision_loss, reason = "label sizes are small")]
        let text = gtk::graphene::Rect::new(left as f32, top as f32, width as f32, height as f32);
        text.contains_point(&point)
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
        gtk::accessible::Property::Label(&ox_core::i18n::gettext("Files")),
        gtk::accessible::Property::Description(&ox_core::i18n::gettext(
            "Folder contents — type a filename prefix to select",
        )),
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
        cell.follow_item_check(list_item, &setup_owners);
    });
    let bind_owners = Rc::clone(owners);
    factory.connect_bind(move |_, object| {
        let list_item = as_list_item(object);
        let cell = list_item.child().and_downcast::<FileCell>();
        if let (Some(item), Some(cell)) = (bound_item(list_item), cell) {
            // A tile is named after its item (`aria-label` in app.js).
            list_item.set_accessible_label(&item.entry().name);
            cell.bind(&item);
            cell.show_item_check(bind_owners.shows_item_checks());
            cell.look_up_picture(&item, bind_owners.previews());
            bind_owners.style_cell(&cell, &item);
        }
    });
    factory.connect_unbind(|_, object| {
        if let Some(cell) = as_list_item(object).child().and_downcast::<FileCell>() {
            cell.cancel_picture_lookup();
        }
    });
}
