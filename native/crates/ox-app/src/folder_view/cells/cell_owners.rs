// SPDX-License-Identifier: AGPL-3.0-only
//! Which row a click lands on.
//!
//! GTK 4.14 has no public hit-test for list views, so the views register
//! each cell's content widget with its list item, and a click position is
//! picked and walked up to a registered widget. The web app reads the row
//! from the clicked element instead (`closest('.file-row,.file-tile')` in
//! `desktop/ui/app.js`).

use std::cell::RefCell;
use std::rc::Rc;

use gtk::glib;
use gtk::prelude::*;

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
/// turned into a row.
#[derive(Debug, Default)]
pub(crate) struct CellOwners {
    owners: RefCell<Vec<CellOwner>>,
}

impl CellOwners {
    /// A shared, empty registry.
    pub fn new() -> Rc<Self> {
        Rc::new(Self::default())
    }

    /// Records that `cell` is the content widget of `list_item`, and
    /// forgets cells that are gone.
    pub fn register(&self, cell: &impl IsA<gtk::Widget>, list_item: &gtk::ListItem) {
        let mut owners = self.owners.borrow_mut();
        owners.retain(CellOwner::is_alive);
        owners.push(CellOwner {
            cell: cell.as_ref().downgrade(),
            list_item: list_item.downgrade(),
        });
    }

    /// The position of the item under (`x`, `y`) in `view`'s coordinates,
    /// or `None` over empty space. Clicks in a row's padding count too.
    pub fn position_at(&self, view: &impl IsA<gtk::Widget>, x: f64, y: f64) -> Option<u32> {
        let view = view.as_ref();
        let picked = view.pick(x, y, gtk::PickFlags::DEFAULT)?;
        let list_item = self.owner_near(view, picked)?;
        bound_position(&list_item)
    }

    /// The content widget showing `position`, if it is on screen.
    pub fn widget_at(&self, position: u32) -> Option<gtk::Widget> {
        self.owners
            .borrow()
            .iter()
            .find_map(|owner| owner.cell_showing(position))
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
