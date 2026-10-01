// SPDX-License-Identifier: AGPL-3.0-only
//! Which row a click lands on, and which rows show a cut item, a dragged
//! item or the folder a drag would drop into.
//!
//! GTK 4.14 has no public hit-test for list views, so the views register
//! each cell's content widget with its list item, and a click position is
//! picked and walked up to a registered widget. The web app reads the row
//! from the clicked element instead (`closest('.file-row,.file-tile')` in
//! `v2.0.0:desktop/ui/app.js`).
//!
//! The same registry styles cells by their item: the views style each cell
//! when they bind it, since GTK reuses cells for other items, and the
//! registry restyles the cells on screen when the state changes, as
//! `renderRows` toggles the `cut` and `drag-source` classes of every row.
//! Cut items (CLIP-002) and dragged items (DND-007) fade. The folder under
//! a drag gets the drop highlight (`file-drop-active`, DND-011) on its
//! whole row or tile, which GTK owns: that class is moved as the drag
//! moves and never set while a view binds a cell, because changing a row
//! widget's classes while GTK binds its cells upsets the list's
//! bookkeeping (a Gtk-CRITICAL in `gtk_widget_get_next_sibling`).

use std::cell::{Cell, RefCell};
use std::collections::HashSet;
use std::rc::Rc;

use gtk::glib;
use gtk::prelude::*;

use super::row_tooltip::RowTooltip;
use super::FileCell;
use crate::folder_view::item::FileItem;

/// The CSS class of a cell whose item a cut put on the clipboard. The
/// stylesheet draws such cells at half opacity (`.file-row.cut{opacity:.5}`
/// in `v2.0.0:desktop/ui/style.css`).
const CUT_CSS_CLASS: &str = "cut";

/// The CSS class of a cell whose item is hidden, shown only while "Show
/// hidden files" is on; the stylesheet draws it faded, as Windows Explorer
/// and Dolphin do (VIEW-026).
const HIDDEN_CSS_CLASS: &str = "hidden-item";

/// The CSS class of a cell whose item is being dragged out
/// (`.file-dragging .drag-source{opacity:.55}`).
const DRAGGED_CSS_CLASS: &str = "drag-source";

/// The CSS class of the row or tile of the folder a drag would drop into
/// (`.file-drop-active`).
const DROP_TARGET_CSS_CLASS: &str = "file-drop-active";

/// The CSS names of the widgets GTK wraps a row's cells and a tile's
/// content in: a column view's `row` and a grid view's `child`.
const ITEM_WIDGET_NAMES: [&str; 2] = ["row", "child"];

/// A cell's content widget and the list item that shows it.
#[derive(Debug)]
struct CellOwner {
    cell: glib::WeakRef<gtk::Widget>,
    list_item: glib::WeakRef<gtk::ListItem>,
}

impl CellOwner {
    /// True while both the cell and its list item exist.
    fn is_alive(&self) -> bool {
        self.cell.upgrade().is_some() && self.list_item.upgrade().is_some()
    }

    /// True when `widget` is this owner's cell.
    fn has_cell(&self, widget: &gtk::Widget) -> bool {
        self.cell.upgrade().is_some_and(|cell| cell == *widget)
    }

    /// The cell and the item its list item shows, while it shows one.
    fn bound_cell(&self) -> Option<(gtk::Widget, FileItem)> {
        let list_item = self.list_item.upgrade()?;
        let item = list_item.item().and_downcast::<FileItem>()?;
        let cell = self.cell.upgrade()?;
        Some((cell, item))
    }

    /// The cell, while its list item shows `position`.
    fn cell_showing(&self, position: u32) -> Option<gtk::Widget> {
        let list_item = self.list_item.upgrade()?;
        if bound_position(&list_item) != Some(position) {
            return None;
        }
        self.cell.upgrade()
    }
}

/// The position `list_item` shows, or `None` while it shows no item.
fn bound_position(list_item: &gtk::ListItem) -> Option<u32> {
    let position = list_item.position();
    let is_bound = position != gtk::INVALID_LIST_POSITION && list_item.item().is_some();
    is_bound.then_some(position)
}

/// Adds `class` to `widget` when `shown`, and removes it otherwise.
fn toggle_class(widget: &impl IsA<gtk::Widget>, class: &str, shown: bool) {
    if shown {
        widget.add_css_class(class);
    } else {
        widget.remove_css_class(class);
    }
}

/// The row or tile widget GTK wraps `cell` in.
fn item_widget(cell: &gtk::Widget) -> Option<gtk::Widget> {
    std::iter::successors(cell.parent(), WidgetExt::parent)
        .find(|widget| ITEM_WIDGET_NAMES.contains(&widget.css_name().as_str()))
}

/// Maps cell widgets back to their list items, so a click position can be
/// turned into a row, and styles rows by the state of their item.
#[derive(Debug, Default)]
pub(crate) struct CellOwners {
    owners: RefCell<Vec<CellOwner>>,
    /// The URIs of the items a cut put on the clipboard, whose cells are
    /// dimmed until they are pasted or the clipboard changes.
    cut_uris: RefCell<HashSet<String>>,
    /// The URIs of the items this window's drag carries, dimmed while it
    /// lasts.
    dragged_uris: RefCell<HashSet<String>>,
    /// The row or tile of the folder a drag hovers over, highlighted.
    drop_row: RefCell<Option<glib::WeakRef<gtk::Widget>>>,
    /// What the rows' tooltips name.
    row_tooltip: Cell<RowTooltip>,
}

impl CellOwners {
    /// A shared, empty registry.
    pub(crate) fn new() -> Rc<Self> {
        Rc::new(Self::default())
    }

    /// Records that `cell` is the content widget of `list_item`, and
    /// forgets cells that are gone.
    pub(crate) fn register(&self, cell: &impl IsA<gtk::Widget>, list_item: &gtk::ListItem) {
        let mut owners = self.owners.borrow_mut();
        owners.retain(CellOwner::is_alive);
        owners.push(CellOwner {
            cell: cell.as_ref().downgrade(),
            list_item: list_item.downgrade(),
        });
    }

    /// What the tooltips of the rows and tiles name now.
    pub(crate) fn row_tooltip(&self) -> RowTooltip {
        self.row_tooltip.get()
    }

    /// Makes the tooltips of the rows and tiles name `tooltip`: full paths
    /// while the window searches, names otherwise.
    pub(crate) fn set_row_tooltip(&self, tooltip: RowTooltip) {
        self.row_tooltip.set(tooltip);
    }

    /// The item the registered `cell` shows, while it shows one.
    pub(crate) fn item_of(&self, cell: &impl IsA<gtk::Widget>) -> Option<FileItem> {
        let list_item = self.owner_of(cell.as_ref())?;
        list_item.item().and_downcast::<FileItem>()
    }

    /// Styles `cell`, which shows `item`: dimmed while a cut has `item` on
    /// the clipboard or a drag carries it. The views call this whenever
    /// they bind a cell, since GTK reuses cells for other items.
    pub(crate) fn style_cell(&self, cell: &impl IsA<gtk::Widget>, item: &FileItem) {
        let uri = &item.entry().uri;
        toggle_class(cell, CUT_CSS_CLASS, self.cut_uris.borrow().contains(uri));
        toggle_class(cell, DRAGGED_CSS_CLASS, self.dragged_uris.borrow().contains(uri));
        toggle_class(cell, HIDDEN_CSS_CLASS, item.entry().is_hidden);
    }

    /// Styles every cell on screen again after the state changed.
    fn restyle_cells(&self) {
        let bound_cells: Vec<(gtk::Widget, FileItem)> = self
            .owners
            .borrow()
            .iter()
            .filter_map(CellOwner::bound_cell)
            .collect();
        for (cell, item) in bound_cells {
            self.style_cell(&cell, &item);
        }
    }

    /// Looks up the custom icon of the item at `uri` again in every cell
    /// that shows it (PROP-016).
    pub(crate) fn refresh_custom_icon(&self, uri: &str) {
        let showing: Vec<(gtk::Widget, FileItem)> = self
            .owners
            .borrow()
            .iter()
            .filter_map(CellOwner::bound_cell)
            .filter(|(_, item)| item.entry().uri == uri)
            .collect();
        for (cell, item) in showing {
            if let Some(cell) = cell.downcast_ref::<FileCell>() {
                cell.look_up_custom_icon(&item);
            }
        }
    }

    /// Dims the cells of the items at `uris` and no others: the items a cut
    /// put on the clipboard, until they are pasted or the clipboard changes
    /// (CLIP-002, CLIP-009).
    pub(crate) fn show_cut_items(&self, uris: HashSet<String>) {
        if *self.cut_uris.borrow() == uris {
            return;
        }
        self.cut_uris.replace(uris);
        self.restyle_cells();
    }

    /// Dims the cells of the items at `uris` and no others: the items a
    /// drag out of this window carries, until it ends (DND-007).
    pub(crate) fn show_dragged_items(&self, uris: HashSet<String>) {
        if *self.dragged_uris.borrow() == uris {
            return;
        }
        self.dragged_uris.replace(uris);
        self.restyle_cells();
    }

    /// Highlights the row or tile showing `position`, the folder a drag
    /// would drop into, or none (DND-011). Called on every motion of the
    /// drag, so a row GTK has meanwhile reused for another item loses the
    /// highlight at the next one.
    pub(crate) fn show_drop_target(&self, position: Option<u32>) {
        let row = position
            .and_then(|position| self.widget_at(position))
            .and_then(|cell| item_widget(&cell));
        let highlighted = self.drop_row.borrow().as_ref().and_then(glib::WeakRef::upgrade);
        if highlighted == row {
            return;
        }
        if let Some(previous) = highlighted {
            previous.remove_css_class(DROP_TARGET_CSS_CLASS);
        }
        if let Some(row) = &row {
            row.add_css_class(DROP_TARGET_CSS_CLASS);
        }
        self.drop_row.replace(row.map(|row| row.downgrade()));
    }

    /// Whether every cell on screen that shows `position` is dimmed as
    /// cut; `None` when none shows it. For tests of what the views show.
    #[cfg(test)]
    pub(crate) fn is_shown_cut(&self, position: u32) -> Option<bool> {
        self.cells_have_class(position, CUT_CSS_CLASS)
    }

    /// Whether every cell on screen that shows `position` is faded as
    /// hidden; `None` when none shows it. For tests.
    #[cfg(test)]
    pub(crate) fn is_shown_hidden(&self, position: u32) -> Option<bool> {
        self.cells_have_class(position, HIDDEN_CSS_CLASS)
    }

    /// Whether every cell on screen that shows `position` is dimmed as
    /// dragged; `None` when none shows it. For tests.
    #[cfg(test)]
    pub(crate) fn is_shown_dragged(&self, position: u32) -> Option<bool> {
        self.cells_have_class(position, DRAGGED_CSS_CLASS)
    }

    /// Whether every cell on screen that shows `position` has `class`;
    /// `None` when none shows it.
    #[cfg(test)]
    fn cells_have_class(&self, position: u32, class: &str) -> Option<bool> {
        let owners = self.owners.borrow();
        let cells: Vec<gtk::Widget> = owners
            .iter()
            .filter_map(|owner| owner.cell_showing(position))
            .collect();
        if cells.is_empty() {
            return None;
        }
        Some(cells.iter().all(|cell| cell.has_css_class(class)))
    }

    /// Whether the row or tile showing `position` is highlighted as the
    /// folder a drag would drop into; `None` when none shows it. For tests.
    #[cfg(test)]
    pub(crate) fn is_shown_drop_target(&self, position: u32) -> Option<bool> {
        let cell = self.widget_at(position)?;
        let row = item_widget(&cell)?;
        Some(row.has_css_class(DROP_TARGET_CSS_CLASS))
    }

    /// The position of the item under (`x`, `y`) in `view`'s coordinates,
    /// or `None` over empty space. Clicks in a row's padding count too.
    pub(crate) fn position_at(&self, view: &impl IsA<gtk::Widget>, x: f64, y: f64) -> Option<u32> {
        let view = view.as_ref();
        let picked = view.pick(x, y, gtk::PickFlags::DEFAULT)?;
        let list_item = self.owner_near(view, picked)?;
        bound_position(&list_item)
    }

    /// The position of the item whose row or tile holds `widget`, inside
    /// `view`.
    pub(crate) fn position_holding(&self, view: &impl IsA<gtk::Widget>, widget: gtk::Widget) -> Option<u32> {
        let list_item = self.owner_near(view.as_ref(), widget)?;
        bound_position(&list_item)
    }

    /// The position of the item whose row or tile is, or contains,
    /// `widget`, such as the one with keyboard focus.
    pub(crate) fn position_of(&self, widget: &gtk::Widget) -> Option<u32> {
        // A list's own first child is a row, so the walk stops below it.
        let list_item = std::iter::successors(Some(widget.clone()), WidgetExt::parent)
            .take_while(|widget| !widget.is::<gtk::ListBase>())
            .find_map(|widget| self.owner_within(&widget))?;
        bound_position(&list_item)
    }

    /// The content widget showing `position`, if it is on screen.
    pub(crate) fn widget_at(&self, position: u32) -> Option<gtk::Widget> {
        self.owners
            .borrow()
            .iter()
            .find_map(|owner| owner.cell_showing(position))
    }

    /// The icon-and-name cell showing `position` inside `view`, if it is
    /// on screen.
    pub(crate) fn file_cell_at(&self, position: u32, view: &impl IsA<gtk::Widget>) -> Option<FileCell> {
        self.owners
            .borrow()
            .iter()
            .filter_map(|owner| owner.cell_showing(position))
            .filter(|cell| cell.is_ancestor(view))
            .find_map(|cell| cell.downcast::<FileCell>().ok())
    }

    /// The rows or tiles on screen inside `view`, each once, with the
    /// position each shows, for a rubber band to test against.
    pub(crate) fn shown_items(&self, view: &impl IsA<gtk::Widget>) -> Vec<(u32, gtk::Widget)> {
        let view = view.as_ref();
        let mut shown: Vec<(u32, gtk::Widget)> = Vec::new();
        for owner in self.owners.borrow().iter() {
            let Some(list_item) = owner.list_item.upgrade() else {
                continue;
            };
            let (Some(position), Some(cell)) = (bound_position(&list_item), owner.cell.upgrade()) else {
                continue;
            };
            let row = item_widget(&cell).filter(|row| row.is_ancestor(view));
            if let Some(row) = row.filter(|_| !shown.iter().any(|(seen, _)| *seen == position)) {
                shown.push((position, row));
            }
        }
        shown
    }

    /// The list item whose content widget is `widget`.
    fn owner_of(&self, widget: &gtk::Widget) -> Option<gtk::ListItem> {
        let owners = self.owners.borrow();
        let owner = owners.iter().find(|owner| owner.has_cell(widget))?;
        owner.list_item.upgrade()
    }

    /// The list item of the nearest widget from `picked` up to, but not
    /// including, `view` that holds a registered cell.
    fn owner_near(&self, view: &gtk::Widget, picked: gtk::Widget) -> Option<gtk::ListItem> {
        std::iter::successors(Some(picked), WidgetExt::parent)
            .take_while(|widget| widget != view)
            .find_map(|widget| self.owner_within(&widget))
    }

    /// The list item whose content widget is `widget`, its first child or
    /// that child's first child. GTK wraps a content widget in a row
    /// widget, and in a column view in a cell widget too, which a click in
    /// the padding picks.
    fn owner_within(&self, widget: &gtk::Widget) -> Option<gtk::ListItem> {
        let first_child = widget.first_child();
        let grandchild = first_child.as_ref().and_then(WidgetExt::first_child);
        [Some(widget.clone()), first_child, grandchild]
            .iter()
            .flatten()
            .find_map(|candidate| self.owner_of(candidate))
    }
}
