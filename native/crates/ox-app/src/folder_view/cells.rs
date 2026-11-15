// SPDX-License-Identifier: AGPL-3.0-only
//! Cell widgets shared by the details and icon views.
//!
//! Both views show an item as its icon art beside or above its name
//! ([`FileCell`]), as the name cell of `renderRows` in `v2.0.0:desktop/ui/app.js`
//! does. The item shows Explorer's infotip ([`row_tooltip`]) where
//! `row.title` showed its name. While the item is renamed in place, a text field
//! takes the name's place. [`CellOwners`] follows the cells the views
//! bind and turns a click position back into a row. The art is an [`ArtImage`] of
//! bundled icons, which GTK renders again by itself when the theme or the
//! screen scale changes.

mod cell_owners;
mod custom_icon;
mod expander;
mod folder_picture;
mod heading_span;
mod item_check;
mod marks;
pub(crate) use item_check::own_clicks;
mod row_tooltip;

use std::rc::Rc;

use gtk::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{glib, pango};

pub(crate) use cell_owners::{CellOwners, PositionMap};
pub(crate) use custom_icon::{icon_texture, CUSTOM_ICON};
pub(crate) use expander::connect_expanders;
pub(crate) use folder_picture::{folder_picture_of, FolderPicture, FOLDER_PICTURE};
pub(crate) use row_tooltip::{show_row_tooltip, CellTooltip, RowTooltip, TooltipCell};

use crate::folder_view::grid::{GroupSlot, SlotKind};
use crate::folder_view::item::FileItem;
use crate::icons::{self, Art, Icon};
use crate::window::widget_tree::toggle_class;
use heading_span::HeadingSpan;
use marks::marked_words;

/// Gap between a row's icon and its name (`.name-cell{gap:11px}`).
const ROW_ICON_GAP: i32 = 11;

/// Width of a row's check box: 14 pixels and its border
/// (`.files checkbutton.item-check > check`).
const ROW_CHECK_WIDTH: i32 = 16;

/// How far a row's check box moves right after a folder arrow
/// (`.has-folder-expander checkbutton.item-check`).
const ROW_CHECK_ARROW_SHIFT: i32 = 16;

/// The room a row's check box and its gap take before the icon, with
/// folder arrows before it or not.
pub(crate) fn row_check_room(after_arrows: bool) -> f64 {
    let shift = if after_arrows { ROW_CHECK_ARROW_SHIFT } else { 0 };
    f64::from(ROW_CHECK_WIDTH + ROW_ICON_GAP + shift)
}

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

/// Gap between the icon and the text of Tiles and Content (VIEW-071).
const LINES_ICON_GAP: i32 = 10;

/// How wide Content's date modified is, in characters, so its column of
/// date modified and size starts at the same place on every row, as
/// Windows 11 sets these properties at the right of each row.
const CONTENT_DATE_CHARS: i32 = 20;

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
    dim_cell_label_with("")
}

/// [`dim_cell_label`] saying `text`.
fn dim_cell_label_with(text: &str) -> gtk::Label {
    let label = gtk::Label::new(Some(text).filter(|text| !text.is_empty()));
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
    /// One of Explorer's Tiles: the icon left of three lines, the name,
    /// the type and, for a file, the size (VIEW-071).
    Tile,
    /// One of Explorer's Content rows: the icon left of the name with the
    /// type under it, and the date modified and, for a file, the size at
    /// the right, each after its caption (VIEW-071).
    ContentRow,
}

/// The CSS class of a cell that shows a grid slot with no item: a group's
/// heading, its line or a blank (VIEW-071).
const GROUP_SLOT_CLASS: &str = "group-slot";

/// The CSS class of a slot row that heads a group, which the keyboard
/// can rest on.
const HEADING_CLASS: &str = "group-heading";

/// The CSS class of a cell in a heading's row: the heading or a slot
/// carrying its line.
const HEADING_ROW_CLASS: &str = "heading-row";

/// The CSS class of a heading row's cells while the pointer is over the
/// row.
const ROW_HOVER_CLASS: &str = "row-hover";

/// The CSS class of a heading row's cells while its heading has the
/// keyboard.
const ROW_FOCUS_CLASS: &str = "row-focus";

/// The CSS class of a heading row's cells, and of a Details heading's
/// row, while the heading's group is collapsed with every item selected.
const ROW_FILLED_CLASS: &str = "row-filled";

/// The CSS class of the cell starting a heading's row: the heading.
const ROW_START_CLASS: &str = "row-start";

/// The CSS class of the cell ending a heading's row at the right.
const ROW_END_CLASS: &str = "row-end";

/// The edge of a heading's chevron, as Windows 11 draws it.
const HEADING_CHEVRON: i32 = 16;

/// A heading's chevron: down while its group is expanded, right while
/// it is collapsed, as Windows 11 draws them.
fn chevron_icon(collapsed: bool) -> Icon {
    if collapsed {
        Icon::ChevronRight16
    } else {
        Icon::ChevronDown16
    }
}

/// What a Tile or Content row says about its item beside the name: the
/// type, the size of a file (empty for a folder) and the date modified.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ItemLines {
    pub kind: String,
    pub size: String,
    pub modified: String,
}

impl ItemLines {
    /// The lines for `item`, written as Windows 11's Tiles and Content
    /// write them: the type's name, the size with three significant digits
    /// as the status bar writes it ("1.23 MB", "11.4 KB"), and the date and
    /// time.
    pub(crate) fn of(item: &FileItem) -> Self {
        let entry = item.entry();
        let size = if entry.is_dir {
            String::new()
        } else {
            item.file_size()
                .map(ox_core::format::short_size)
                .unwrap_or_default()
        };
        let modified = if entry.modified.is_some() {
            ox_core::format::column_date_text(entry.modified, ox_core::format::DateStyle::Absolute)
        } else {
            String::new()
        };
        Self {
            kind: entry.type_label.clone(),
            size,
            modified,
        }
    }
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
        /// The folder's picture on its icon (PROP-037), on a tile of
        /// Medium icons or larger.
        pub(super) folder_picture: super::FolderPicture,
        /// Whether the cell is an icon-view tile.
        pub(super) is_tile: Cell<bool>,
        /// Whether the cell is a Details row's Name cell, whose group
        /// heading spans the whole row.
        pub(super) is_details_row: Cell<bool>,
        /// The running lookup of the custom icon or preview.
        pub(super) picture_lookup: RefCell<Option<glib::JoinHandle<()>>>,
        /// Holds the icon, and on a tile spans it, with the check box in
        /// its top left corner.
        pub(super) icon_frame: gtk::Overlay,
        /// Checked while the item is selected; a click selects or
        /// deselects it alone (SEL-014).
        pub(super) check: gtk::CheckButton,
        /// Whether the check box takes clicks though unchecked; else only a
        /// checked one does, while the boxes wait for Ctrl (SEL-014).
        pub(super) check_takes_clicks: Cell<bool>,
        /// Expands a folder of a details row in place (VIEW-035).
        pub(super) expander: gtk::Button,
        /// The tree row the cell follows, and its handler.
        pub(super) tree_row: RefCell<Option<(gtk::TreeListRow, glib::SignalHandlerId)>>,
        /// The lines beside the icon of Tiles and Content: the name, and
        /// under it the type (and on a tile the size). Unused elsewhere.
        pub(super) lines: gtk::Box,
        /// The type, under the name.
        pub(super) kind: gtk::Label,
        /// A tile's size, under the type; Content's at its right.
        pub(super) size: gtk::Label,
        /// Content's date modified, at its right.
        pub(super) modified: gtk::Label,
        /// Content's "Date modified:" and "Size:" lines, each hidden while
        /// it has nothing to say.
        pub(super) properties: RefCell<Vec<gtk::Box>>,
        /// Whether the cell shows [`super::ItemLines`].
        pub(super) shows_item_lines: Cell<bool>,
        /// Whether the cell is a Content row, which shows no type under a
        /// folder's name, as Windows 11.
        pub(super) is_content_row: Cell<bool>,
        /// A group's heading or its line, shown in place of the item's
        /// widgets while the cell shows a [`super::GroupSlot`].
        pub(super) slot_row: RefCell<Option<gtk::Box>>,
        /// The heading's text, bound to its slot's.
        pub(super) slot_binding: RefCell<Vec<glib::Binding>>,
        /// Follows whether the heading's group is collapsed.
        pub(super) slot_watch: RefCell<Option<glib::SignalHandlerId>>,
        /// The expanded state the cell last told assistive technology:
        /// `None` while it shows no heading.
        pub(super) announced_expanded: Cell<Option<bool>>,
        /// The slot the cell shows, while it shows one.
        pub(super) slot: RefCell<Option<super::GroupSlot>>,
        /// Which of the item's widgets were shown when a slot took the
        /// cell, to show them again after.
        pub(super) hidden_for_slot: RefCell<Vec<(gtk::Widget, bool)>>,
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
            self.folder_picture.set_visible(false);
            self.icon_frame.add_overlay(&self.folder_picture);
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
        imp.folder_picture.place_on_icon(icon_size);
        // A folder until bound, so the cell has its full size from the start.
        imp.image.set_art(Art::Folder, icon_size);
        match layout {
            CellLayout::DetailsRow => cell.lay_out_as_row(),
            CellLayout::IconTile => cell.lay_out_as_tile(),
            CellLayout::CompactItem => cell.lay_out_as_compact_item(),
            CellLayout::Tile => cell.lay_out_with_lines(false),
            CellLayout::ContentRow => cell.lay_out_with_lines(true),
        }
        cell
    }

    /// Shows `slot` of a grouped grid in place of an item: a group's
    /// heading (its chevron, "Today" and the start of its line), the
    /// rest of its line, or nothing. `None` brings the item's widgets back.
    pub(crate) fn show_group_slot(&self, slot: Option<&GroupSlot>) {
        let imp = self.imp();
        for binding in imp.slot_binding.take() {
            binding.unbind();
        }
        let watched = imp.slot.replace(slot.cloned());
        if let (Some(watched), Some(handler)) = (watched, imp.slot_watch.take()) {
            watched.disconnect(handler);
            // The cell goes on to show something else: the row it showed
            // loses the pointer it had through it.
            if slot != Some(&watched) && watched.hovered() {
                // Later: the row's other cells restyle, which GTK must not
                // see while it binds this one. The window follows the
                // keyboard itself (`window/group_headings.rs`).
                glib::idle_add_local_once(move || watched.set_row_hovered(false));
            }
        }
        let Some(slot) = slot else {
            self.clear_row_classes();
            if !self.has_css_class(GROUP_SLOT_CLASS) {
                return;
            }
            self.remove_css_class(GROUP_SLOT_CLASS);
            if let Some(span) = imp.slot_row.borrow().as_ref().and_then(WidgetExt::parent) {
                span.set_visible(false);
            }
            for (widget, shown) in imp.hidden_for_slot.take() {
                widget.set_visible(shown);
            }
            return;
        };
        if !self.has_css_class(GROUP_SLOT_CLASS) {
            self.hide_name_editor();
            let row = self.slot_row();
            let hidden: Vec<(gtk::Widget, bool)> =
                std::iter::successors(self.first_child(), WidgetExt::next_sibling)
                    .filter(|widget| Some(widget) != row.parent().as_ref())
                    .map(|widget| {
                        let shown = widget.is_visible();
                        widget.set_visible(false);
                        (widget, shown)
                    })
                    .collect();
            imp.hidden_for_slot.replace(hidden);
            self.add_css_class(GROUP_SLOT_CLASS);
        }
        let row = self.slot_row();
        let hit = row.first_child().expect("a slot row starts with its heading");
        let chevron = hit
            .first_child()
            .and_downcast::<gtk::Image>()
            .expect("a heading starts with its chevron");
        let title = chevron
            .next_sibling()
            .and_downcast::<gtk::Label>()
            .expect("the title follows the chevron");
        let line = hit.next_sibling().expect("a slot row ends with its line");
        let kind = slot.kind();
        let is_heading = kind == SlotKind::Heading;
        hit.set_visible(is_heading);
        line.set_visible(kind != SlotKind::Blank);
        row.set_visible(true);
        if let Some(span) = row.parent() {
            span.set_visible(true);
        }
        toggle_class(&row, HEADING_CLASS, is_heading);
        if is_heading {
            let text = slot.bind_property("text", &title, "label").sync_create().build();
            imp.slot_binding.borrow_mut().push(text);
        }
        let show_state = glib::clone!(
            #[weak(rename_to = cell)]
            self,
            #[weak]
            chevron,
            move |slot: &GroupSlot| {
                let in_row = slot.kind() != SlotKind::Blank;
                if slot.kind() == SlotKind::Heading {
                    let collapsed = slot.collapsed();
                    icons::set_icon(&chevron, chevron_icon(collapsed), HEADING_CHEVRON);
                    cell.announce_expanded(Some(!collapsed));
                } else {
                    cell.announce_expanded(None);
                }
                toggle_class(&cell, HEADING_ROW_CLASS, in_row);
                toggle_class(&cell, ROW_HOVER_CLASS, in_row && slot.hovered());
                toggle_class(&cell, ROW_FOCUS_CLASS, in_row && slot.focused());
                toggle_class(&cell, ROW_START_CLASS, slot.kind() == SlotKind::Heading);
                toggle_class(&cell, ROW_END_CLASS, in_row && slot.ends_row());
                toggle_class(&cell, ROW_FILLED_CLASS, in_row && slot.filled());
                cell.mark_item_widget(in_row, in_row && slot.filled());
            }
        );
        show_state(slot);
        let handler = slot.connect_notify_local(None, move |slot, _| show_state(slot));
        imp.slot_watch.replace(Some(handler));
    }

    /// Tells assistive technology whether the heading the cell shows is
    /// expanded, or, with `None`, that the cell shows no heading: GTK
    /// reuses a heading's cell for files, lines and blanks.
    fn announce_expanded(&self, expanded: Option<bool>) {
        if self.imp().announced_expanded.replace(expanded) == expanded {
            return;
        }
        match expanded {
            Some(expanded) => self.update_state(&[gtk::accessible::State::Expanded(Some(expanded))]),
            None => self.reset_state(gtk::AccessibleState::Expanded),
        }
    }

    /// Whether the cell tells assistive technology it is expanded, for
    /// tests.
    #[cfg(test)]
    pub(crate) fn announced_expanded(&self) -> Option<bool> {
        self.imp().announced_expanded.get()
    }

    /// Takes the heading row's looks off the cell, which shows an item.
    fn clear_row_classes(&self) {
        self.announce_expanded(None);
        for class in [
            HEADING_ROW_CLASS,
            ROW_HOVER_CLASS,
            ROW_FOCUS_CLASS,
            ROW_START_CLASS,
            ROW_END_CLASS,
        ] {
            toggle_class(self, class, false);
        }
        self.mark_item_widget(false, false);
    }

    /// Marks the row or tile GTK wraps the cell in as a heading's, or not,
    /// and as filled while its folded group is all selected: Details draws
    /// a heading's row by these classes. Never on the pointer's account, as restyling GTK's rows under the
    /// pointer can make it send the pointer's crossings again. Done once
    /// GTK has bound the cell, as changing a row's classes while GTK binds
    /// its cells upsets the list (see `cells/cell_owners.rs`).
    fn mark_item_widget(&self, heading: bool, filled: bool) {
        glib::idle_add_local_once(glib::clone!(
            #[weak(rename_to = cell)]
            self,
            move || {
                let item = std::iter::successors(cell.parent(), WidgetExt::parent)
                    .find(|widget| ["row", "child"].contains(&widget.css_name().as_str()));
                // Details fills a folded, selected heading's row by this
                // class, as its line cells cannot see the heading.
                if let Some(item) = item
                    .as_ref()
                    .filter(|item| item.has_css_class(ROW_FILLED_CLASS) != filled)
                {
                    toggle_class(item, ROW_FILLED_CLASS, filled);
                }
                if let Some(item) = item.filter(|item| item.has_css_class(HEADING_ROW_CLASS) != heading) {
                    toggle_class(&item, HEADING_ROW_CLASS, heading);
                    // A Details heading's box is drawn across the columns
                    // by each column's cell, which GTK clips to the column:
                    // its pieces overlap only while the cells let them.
                    let overflow = if heading {
                        gtk::Overflow::Visible
                    } else {
                        gtk::Overflow::Hidden
                    };
                    for cell in std::iter::successors(item.first_child(), WidgetExt::next_sibling) {
                        cell.set_overflow(overflow);
                    }
                }
            }
        ));
    }

    /// The group slot the cell shows, while it shows one.
    pub(crate) fn slot(&self) -> Option<GroupSlot> {
        self.imp().slot.borrow().clone()
    }

    /// Lays a Details heading out across its row again, once a column's
    /// width changed where the Name cell's own did not.
    pub(crate) fn respan_heading(&self) {
        if !self.has_css_class(GROUP_SLOT_CLASS) {
            return;
        }
        let span = self.imp().slot_row.borrow().as_ref().and_then(WidgetExt::parent);
        if let Some(span) = span.and_downcast::<HeadingSpan>() {
            span.respan();
        }
    }

    /// The heading row of a slot, made the first time a slot shows: the
    /// chevron, the title and the rest of the row. The window handles
    /// clicks on it (`window/group_headings.rs`).
    fn slot_row(&self) -> gtk::Box {
        let imp = self.imp();
        if let Some(row) = imp.slot_row.borrow().as_ref() {
            return row.clone();
        }
        let chevron = icons::image(chevron_icon(false), HEADING_CHEVRON);
        chevron.add_css_class("group-chevron");
        let title = gtk::Label::builder()
            .xalign(0.0)
            .ellipsize(pango::EllipsizeMode::End)
            .single_line_mode(true)
            .css_classes(["group-title"])
            .build();
        // The rest of the row after the title: Windows 11 draws no line
        // there, so it is an empty filler that only takes the room, and
        // the slots after it in a grid's row carry it on.
        let rest = gtk::Box::builder()
            .hexpand(true)
            .css_classes(["group-rest"])
            .build();
        let row = gtk::Box::builder()
            .spacing(10)
            .hexpand(true)
            .css_classes(["group-header"])
            .build();
        // The chevron and the title, which the rest of the row follows.
        let hit = gtk::Box::builder()
            .spacing(8)
            .css_classes(["group-heading-hit"])
            .build();
        hit.append(&chevron);
        hit.append(&title);
        row.append(&hit);
        row.append(&rest);
        // The pointer anywhere over a heading's row, its heading or a slot
        // carrying the row on, lights the whole row, as in Windows 11. Only
        // a change of state restyles anything.
        let motion = gtk::EventControllerMotion::new();
        let hover = glib::clone!(
            #[weak(rename_to = cell)]
            self,
            move |hovered: bool| {
                if let Some(slot) = cell.slot() {
                    if slot.hovered() != hovered {
                        slot.set_row_hovered(hovered);
                    }
                }
            }
        );
        let enter = hover.clone();
        motion.connect_enter(move |_, _, _| enter(true));
        let moved = hover.clone();
        motion.connect_motion(move |_, _, _| moved(true));
        motion.connect_leave(move |_| hover(false));
        self.add_controller(motion);
        // The keyboard on a heading rings its whole row; the grid gives it
        // to the item widget around the cell.
        let span = HeadingSpan::new(&row, imp.is_details_row.get());
        if imp.is_tile.get() {
            // An icon tile is too narrow for a month's name (HOME-019).
            span.span_grid();
        }
        self.append(&span);
        imp.slot_row.replace(Some(row.clone()));
        row
    }

    /// The heading text the cell shows while it is a heading, for tests.
    #[cfg(test)]
    pub(crate) fn group_heading(&self) -> Option<String> {
        let row = self.imp().slot_row.borrow().clone()?;
        let title = row
            .first_child()?
            .first_child()?
            .next_sibling()
            .and_downcast::<gtk::Label>()?;
        (row.is_visible() && title.is_visible()).then(|| title.text().to_string())
    }

    /// The check box and the icon left of the name with the type under
    /// it: a tile with the size under them, or a Content row with the date
    /// modified and the size at its right (VIEW-071).
    fn lay_out_with_lines(&self, content: bool) {
        let imp = self.imp();
        imp.shows_item_lines.set(true);
        imp.is_content_row.set(content);
        // Tiles and Content draw an item's hover and selection on its cell,
        // not on the grid's slot, so a heading's slots show none.
        self.add_css_class("item-cell");
        // The cell spans its tile or row, so the lines can use its width
        // and Content's date and size line up at its right on every row.
        self.set_hexpand(true);
        self.set_orientation(gtk::Orientation::Horizontal);
        self.set_spacing(LINES_ICON_GAP);
        self.put_check_before_icon();
        imp.icon_frame.set_valign(gtk::Align::Center);
        // The name moves into the lines beside the icon.
        self.remove(&imp.label);
        let lines = &imp.lines;
        lines.set_orientation(gtk::Orientation::Vertical);
        lines.set_valign(gtk::Align::Center);
        lines.set_hexpand(true);
        lines.add_css_class("item-lines");
        let label = &imp.label;
        label.set_xalign(0.0);
        label.set_ellipsize(pango::EllipsizeMode::End);
        label.set_single_line_mode(true);
        lines.append(label);
        for line in [&imp.kind, &imp.size, &imp.modified] {
            line.set_xalign(0.0);
            line.set_ellipsize(pango::EllipsizeMode::End);
            line.set_single_line_mode(true);
            line.add_css_class("cell-dim");
        }
        lines.append(&imp.kind);
        self.append(lines);
        if !content {
            lines.append(&imp.size);
            return;
        }
        let properties = gtk::Box::new(gtk::Orientation::Vertical, 0);
        properties.set_valign(gtk::Align::Center);
        // Only the name's lines take the room to spare, which keeps the
        // date and size at the row's right.
        properties.set_hexpand(false);
        properties.add_css_class("item-properties");
        let mut captioned = Vec::new();
        for (caption, value) in [
            (ox_core::i18n::gettext("Date modified:"), &imp.modified),
            (ox_core::i18n::gettext("Size:"), &imp.size),
        ] {
            let line = gtk::Box::new(gtk::Orientation::Horizontal, 4);
            let caption = dim_cell_label_with(&caption);
            line.append(&caption);
            value.remove_css_class("cell-dim");
            line.append(value);
            properties.append(&line);
            captioned.push(line);
        }
        // The date is as wide on every row, so the column starts at the
        // same place on each.
        imp.modified.set_width_chars(CONTENT_DATE_CHARS);
        imp.modified.set_max_width_chars(CONTENT_DATE_CHARS);
        self.append(&properties);
        imp.properties.replace(captioned);
    }

    /// The check box and the icon left of a one-line name that is cut off
    /// with an ellipsis.
    fn lay_out_as_row(&self) {
        self.imp().is_details_row.set(true);
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
        // The check box sits in the tile's top left corner, as Explorer's:
        // the frame spans the tile, and the icon stays centred in it at its
        // own size, so nothing moves.
        let imp = self.imp();
        imp.is_tile.set(true);
        imp.icon_frame.set_halign(gtk::Align::Fill);
        imp.image.set_halign(gtk::Align::Center);
        imp.custom_icon.set_halign(gtk::Align::Center);
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

    /// Shows `item`: its art and its name, and in Tiles and Content its
    /// type, size and date modified. A rename in place ends: the cell now
    /// shows another item, or the same one listed again.
    pub(crate) fn bind(&self, item: &FileItem) {
        self.hide_name_editor();
        let imp = self.imp();
        let icon_size = imp.icon_size.get();
        imp.image.set_art(item.art(), icon_size);
        imp.image.set_emblems(item.emblems(), icon_size);
        imp.label.set_text(&item.entry().name);
        if imp.shows_item_lines.get() {
            let mut lines = ItemLines::of(item);
            if imp.is_content_row.get() && item.entry().is_dir {
                // Windows 11's Content rows give a folder its name alone.
                lines.kind.clear();
            }
            self.show_item_lines(&lines);
        }
    }

    /// Shows `lines` beside the name, each line hidden while empty.
    fn show_item_lines(&self, lines: &ItemLines) {
        let imp = self.imp();
        for (label, text) in [
            (&imp.kind, &lines.kind),
            (&imp.size, &lines.size),
            (&imp.modified, &lines.modified),
        ] {
            label.set_text(text);
            label.set_visible(!text.is_empty());
        }
        // Content's lines go with their captions.
        for line in imp.properties.borrow().iter() {
            let shows = std::iter::successors(line.first_child(), WidgetExt::next_sibling)
                .last()
                .is_some_and(|value| value.is_visible());
            line.set_visible(shows);
        }
    }

    /// The type, size and date modified the cell shows beside the name,
    /// for tests; `None` outside Tiles and Content.
    #[cfg(test)]
    pub(crate) fn shown_item_lines(&self) -> Option<ItemLines> {
        let imp = self.imp();
        imp.shows_item_lines.get().then(|| ItemLines {
            kind: imp.kind.text().to_string(),
            size: imp.size.text().to_string(),
            modified: imp.modified.text().to_string(),
        })
    }

    /// Shows `name` as the item's name: its own, or without a known type's
    /// extension (VIEW-069), with `words`, which a search looks for, marked
    /// in it as Windows 11 marks them in its results (SRCH-046).
    pub(crate) fn show_name(&self, name: &str, words: &[String]) {
        let label = &self.imp().label;
        label.set_text(name);
        label.set_attributes(marked_words(name, words).as_ref());
    }

    /// The byte ranges of the name marked as found, for tests.
    #[cfg(test)]
    pub(crate) fn marked_ranges(&self) -> Vec<(u32, u32)> {
        let Some(attributes) = self.imp().label.attributes() else {
            return Vec::new();
        };
        attributes
            .attributes()
            .iter()
            .filter(|attribute| attribute.type_() == gtk::pango::AttrType::Background)
            .map(|attribute| (attribute.start_index(), attribute.end_index()))
            .collect()
    }

    /// The box that holds the name: the cell itself, or the lines beside
    /// the icon of Tiles and Content.
    fn name_box(&self) -> gtk::Box {
        self.imp()
            .label
            .parent()
            .and_downcast::<gtk::Box>()
            .unwrap_or_else(|| self.clone().upcast())
    }

    /// Puts `editor` in the name's place, to rename the item in place: on
    /// the name's own line in Tiles and Content.
    pub(crate) fn show_name_editor(&self, editor: &gtk::Entry) {
        self.hide_name_editor();
        let imp = self.imp();
        self.name_box().insert_child_after(editor, Some(&imp.label));
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

    /// Whether `point`, in the cell's coordinates, is right of the end of
    /// the name's text: on the empty part of a details row, whose icon and
    /// name are the item (SEL-012). Never while the name is being renamed,
    /// as the text field takes the whole cell, nor before the name is laid
    /// out, when where it ends is not known yet.
    pub(crate) fn is_past_name(&self, point: gtk::graphene::Point) -> bool {
        let label = &self.imp().label;
        if !label.is_mapped() || label.width() == 0 {
            return false;
        }
        let Some(in_label) = self.compute_point(label, &point) else {
            return false;
        };
        let (left, _) = label.layout_offsets();
        let (width, _) = label.layout().pixel_size();
        f64::from(in_label.x()) > f64::from(left + width)
    }

    /// Shows the name again in place of the text field, if one shows.
    pub(crate) fn hide_name_editor(&self) {
        let imp = self.imp();
        let Some(editor) = imp.name_editor.take() else {
            return;
        };
        self.name_box().remove(&editor);
        imp.label.set_visible(true);
    }

    /// Whether the name is cut short with an ellipsis where it is shown.
    pub(crate) fn name_is_elided(&self) -> bool {
        self.imp().label.layout().is_ellipsized()
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
/// items. `positions` maps the view's positions to the listing's when
/// it lists group slots among the items ([`GroupSlot`]): a slot's cell
/// shows its heading or line, and can be neither selected nor activated.
pub(crate) fn connect_mapped_file_cells(
    factory: &gtk::SignalListItemFactory,
    layout: CellLayout,
    icon_size: i32,
    owners: &Rc<CellOwners>,
    positions: Option<PositionMap>,
) {
    let setup_owners = Rc::clone(owners);
    factory.connect_setup(move |_, object| {
        let cell = FileCell::new(layout, icon_size);
        let list_item = as_list_item(object);
        list_item.set_child(Some(&cell));
        setup_owners.register_mapped(&cell, list_item, positions.clone());
        show_row_tooltip(&cell, &setup_owners, |_| None, TooltipCell::Item);
        cell.follow_item_check(list_item, &setup_owners);
    });
    let bind_owners = Rc::clone(owners);
    factory.connect_bind(move |_, object| {
        let list_item = as_list_item(object);
        let cell = list_item.child().and_downcast::<FileCell>();
        if let (Some(slot), Some(cell)) = (list_item.item().and_downcast::<GroupSlot>(), cell.as_ref()) {
            show_slot_item(list_item, cell, Some(&slot));
            CellOwners::unstyle_cell(cell);
            return;
        }
        if let Some(cell) = cell.as_ref().filter(|cell| cell.has_css_class(GROUP_SLOT_CLASS)) {
            show_slot_item(list_item, cell, None);
        }
        if let (Some(item), Some(cell)) = (bound_item(list_item), cell) {
            // A tile is named after its item (`aria-label` in app.js), as
            // it shows the name.
            let name = bind_owners.shown_name(&item);
            list_item.set_accessible_label(&name);
            cell.bind(&item);
            cell.show_name(&name, &bind_owners.marked_words());
            cell.show_item_check(bind_owners.shows_item_checks());
            // A cell bound while it showed a group's heading, or while no
            // item, missed the last change of whether check boxes take
            // clicks (Ctrl held or not).
            cell.item_check_takes_clicks(bind_owners.item_checks_take_clicks());
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

/// Whether `widget` is part of a grid slot that shows no item and is no
/// heading's row (a blank), or the grid's child holding one: blank space
/// for clicks and the rubber band (VIEW-071). A group's heading row takes
/// clicks and the keyboard itself (VIEW-022).
pub(crate) fn is_in_group_slot(widget: &gtk::Widget) -> bool {
    std::iter::successors(Some(widget.clone()), WidgetExt::parent)
        .chain(widget.first_child())
        .filter_map(|widget| widget.downcast::<FileCell>().ok())
        .any(|cell| {
            cell.has_css_class(GROUP_SLOT_CLASS)
                && cell.slot().is_none_or(|slot| slot.kind() == SlotKind::Blank)
        })
}

/// Makes `list_item` show `slot`, which no one selects or opens, or an
/// item again with `None`. A heading takes the keyboard, as Windows 11
/// rests it on a group's heading, where Left and Right collapse and expand
/// the group; a heading's line and a blank do not.
fn show_slot_item(list_item: &gtk::ListItem, cell: &FileCell, slot: Option<&GroupSlot>) {
    let is_item = slot.is_none();
    let is_heading = slot.is_some_and(|slot| slot.kind() == SlotKind::Heading);
    list_item.set_selectable(is_item);
    list_item.set_activatable(is_item);
    list_item.set_focusable(is_item || is_heading);
    cell.show_group_slot(slot);
    if let Some(slot) = slot {
        // A screen reader hears the heading's text as the heading shows
        // it, after a change too: the slot keeps its object while its
        // group stays, so GTK does not bind it again.
        let label = slot
            .bind_property("text", list_item, "accessible-label")
            .sync_create()
            .build();
        cell.imp().slot_binding.borrow_mut().push(label);
    }
}
