// SPDX-License-Identifier: AGPL-3.0-only
//! Which row a click lands on, and which rows show a cut item.
//!
//! GTK 4.14 has no public hit-test for list views, so the views register
//! each cell's content widget with its list item, and a click position is
//! picked and walked up to a registered widget. The web app reads the row
//! from the clicked element instead (`closest('.file-row,.file-tile')` in
//! `desktop/ui/app.js`).
//!
//! The same registry dims the cells of the items a cut put on the
//! clipboard (CLIP-002): the views style each cell when they bind it, and
//! [`CellOwners::show_cut_items`] restyles the cells on screen when the
//! clipboard changes, as `renderRows` toggles the `cut` class of every row.

use std::cell::RefCell;
use std::collections::HashSet;
use std::rc::Rc;

use gtk::glib;
use gtk::prelude::*;

use super::FileCell;
use crate::folder_view::item::FileItem;

/// The CSS class of a cell whose item a cut put on the clipboard. The
/// stylesheet draws such cells at half opacity (`.file-row.cut{opacity:.5}`
/// in `desktop/ui/style.css`).
const CUT_CSS_CLASS: &str = "cut";

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

/// Maps cell widgets back to their list items, so a click position can be
/// turned into a row, and dims the cells of cut items.
#[derive(Debug, Default)]
pub(crate) struct CellOwners {
    owners: RefCell<Vec<CellOwner>>,
    /// The URIs of the items a cut put on the clipboard, whose cells are
    /// dimmed until they are pasted or the clipboard changes.
    cut_uris: RefCell<HashSet<String>>,
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

    /// Dims `cell`, which shows `item`, while a cut has `item` on the
    /// clipboard, and shows it plainly otherwise. The views call this
    /// whenever they bind a cell, since GTK reuses cells for other items.
    pub(crate) fn style_for_cut(&self, cell: &impl IsA<gtk::Widget>, item: &FileItem) {
        let is_cut = self.cut_uris.borrow().contains(&item.entry().uri);
        if is_cut {
            cell.add_css_class(CUT_CSS_CLASS);
        } else {
            cell.remove_css_class(CUT_CSS_CLASS);
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
        for (cell, item) in self.owners.borrow().iter().filter_map(CellOwner::bound_cell) {
            self.style_for_cut(&cell, &item);
        }
    }

    /// Whether every cell on screen that shows `position` is dimmed as
    /// cut; `None` when none shows it. For tests of what the views show.
    #[cfg(test)]
    pub(crate) fn is_shown_cut(&self, position: u32) -> Option<bool> {
        let owners = self.owners.borrow();
        let cells: Vec<gtk::Widget> = owners
            .iter()
            .filter_map(|owner| owner.cell_showing(position))
            .collect();
        if cells.is_empty() {
            return None;
        }
        Some(cells.iter().all(|cell| cell.has_css_class(CUT_CSS_CLASS)))
    }

    /// The position of the item under (`x`, `y`) in `view`'s coordinates,
    /// or `None` over empty space. Clicks in a row's padding count too.
    pub(crate) fn position_at(&self, view: &impl IsA<gtk::Widget>, x: f64, y: f64) -> Option<u32> {
        let view = view.as_ref();
        let picked = view.pick(x, y, gtk::PickFlags::DEFAULT)?;
        let list_item = self.owner_near(view, picked)?;
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
