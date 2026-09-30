// SPDX-License-Identifier: AGPL-3.0-only
//! Up and Down in the icon grid keep to one column across rows of
//! different lengths (SEL-010).
//!
//! GTK's grid moves Down into a shorter last row by clamping to its last
//! item and then forgets the column, so the next Up lands elsewhere.
//! Dolphin remembers the column (`m_keyboardAnchorXPos` in
//! `kitemlistcontroller.cpp`); so does the window, for as long as focus
//! stays on the item it moved to. Left, Right, Shift and Ctrl keep GTK's
//! own keys.

use gtk::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{gdk, glib};

use super::folder_pane::FolderView;
use super::BrowserWindow;

/// The column Up and Down keep to, and the item they last moved to.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct GridColumn {
    column: u32,
    position: u32,
}

/// Where Up (`down` false) or Down from `from` goes in a grid of `len`
/// items in `columns` columns, keeping to `column`: the item in that
/// column of the next row, or the last item when that row is shorter.
/// `None` at the first or last row.
fn row_step(from: u32, column: u32, columns: u32, len: u32, down: bool) -> Option<u32> {
    let columns = columns.max(1);
    let row = from / columns;
    let last_row = len.checked_sub(1)? / columns;
    let row = if down {
        Some(row + 1).filter(|row| *row <= last_row)?
    } else {
        row.checked_sub(1)?
    };
    Some((row * columns + column).min(len - 1))
}

impl BrowserWindow {
    /// Handles a plain Up or Down in the icon grid; `None` for every other
    /// key or view, which GTK handles.
    pub(super) fn grid_row_key(
        &self,
        key: gdk::Key,
        modifiers: gdk::ModifierType,
    ) -> Option<glib::Propagation> {
        let down = match key {
            gdk::Key::Down | gdk::Key::KP_Down => true,
            gdk::Key::Up | gdk::Key::KP_Up => false,
            _ => return None,
        };
        let pane = self.folder_pane();
        if !modifiers.is_empty() || !matches!(pane.view(), FolderView::Icons(_)) {
            return None;
        }
        let focus = GtkWindowExt::focus(self)?;
        let from = pane.owners().position_of(&focus)?;
        let columns = pane.icon_view().grid().max_columns();
        let remembered = self
            .imp()
            .grid_column
            .get()
            .filter(|memory| memory.position == from);
        let column = remembered.map_or(from % columns.max(1), |memory| memory.column);
        let Some(to) = row_step(from, column, columns, pane.model().n_items(), down) else {
            return Some(glib::Propagation::Stop);
        };
        self.reset_typeahead();
        self.imp()
            .grid_column
            .set(Some(GridColumn { column, position: to }));
        pane.select_and_reveal(to);
        Some(glib::Propagation::Stop)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// parity: SEL-010
    #[test]
    fn up_and_down_keep_the_column_across_a_shorter_last_row() {
        // Four columns, ten items: the last row holds 8 and 9.
        let down = row_step(7, 3, 4, 10, true);
        assert_eq!(down, Some(9), "Down into a shorter row goes to its last item");
        assert_eq!(row_step(9, 3, 4, 10, false), Some(7), "Up returns to the column");
        assert_eq!(
            row_step(1, 1, 4, 10, false),
            None,
            "the first row has nothing above"
        );
        assert_eq!(
            row_step(9, 1, 4, 10, true),
            None,
            "the last row has nothing below"
        );
    }
}
