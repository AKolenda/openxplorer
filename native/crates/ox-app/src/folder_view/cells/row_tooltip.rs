// SPDX-License-Identifier: AGPL-3.0-only
//! The tooltip of a row or tile: the item's name, or its full path while
//! searching.
//!
//! Ports `row.title=state.query?displayUri(e.uri):e.name` of `renderRows`
//! in `desktop/ui/app.js` (VIEW-001, VIEW-042). GTK reuses cells for other
//! items, so the text is worked out when the tooltip is asked for, from
//! the item the cell shows then and whether the window searches then.

use std::rc::Rc;

use gtk::prelude::*;
use ox_core::search::display_path;

use super::CellOwners;
use crate::folder_view::item::FileItem;

/// What the tooltip of every row and tile names.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum RowTooltip {
    /// The item's name, while a folder is listed.
    #[default]
    Name,
    /// The item's full display path, while the window searches, so a
    /// result is told apart from others of the same name.
    FullPath,
}

impl RowTooltip {
    /// The tooltip of the row showing `item`.
    pub(crate) fn text(self, item: &FileItem) -> String {
        let entry = item.entry();
        match self {
            RowTooltip::Name => entry.name.clone(),
            RowTooltip::FullPath => display_path(&entry.uri),
        }
    }
}

/// A cell's own tooltip for `item`, which takes the place of the row's,
/// or `None` to show the row's.
pub(crate) type CellTooltip = fn(&FileItem) -> Option<String>;

/// Gives `cell`, one of the cells registered in `owners`, the tooltip of
/// its row, or the one `own_tooltip` gives for its item.
pub(crate) fn show_row_tooltip(
    cell: &impl IsA<gtk::Widget>,
    owners: &Rc<CellOwners>,
    own_tooltip: CellTooltip,
) {
    let owners = Rc::clone(owners);
    cell.set_has_tooltip(true);
    cell.connect_query_tooltip(move |cell, _, _, _, tooltip| {
        let Some(item) = owners.item_of(cell) else {
            return false;
        };
        let text = own_tooltip(&item).unwrap_or_else(|| owners.row_tooltip().text(&item));
        tooltip.set_text(Some(&text));
        true
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::file_entry;

    /// A row's tooltip is its name, or its full path while searching.
    ///
    /// parity: VIEW-001, VIEW-041, VIEW-042
    #[test]
    fn a_row_names_its_item_or_its_full_path_while_searching() {
        let item = FileItem::new(file_entry("Notes 2.txt"));
        assert_eq!(RowTooltip::default().text(&item), "Notes 2.txt");
        assert_eq!(RowTooltip::FullPath.text(&item), "/tmp/ox-test/Notes 2.txt");
    }
}
