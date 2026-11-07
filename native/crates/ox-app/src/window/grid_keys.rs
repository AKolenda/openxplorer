// SPDX-License-Identifier: AGPL-3.0-only
//! Up and Down in the icon grid and in Tiles keep to one column across
//! rows of different lengths (SEL-010). In grouped Tiles and Content the
//! window moves between the items for every arrow, Home, End, Page Up and
//! Page Down, across the groups' headings (VIEW-071). Up and Down rest on
//! a heading on the way, selecting its group's items, as Windows 11 does,
//! where Left and Right collapse and expand its group ([`super::group_headings`]); the items of a
//! collapsed group are passed by.
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
use crate::folder_view::group_slots::{GroupTarget, SlotLayout};

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

/// A key that moves between the items of grouped Tiles and Content.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum GroupedMove {
    Back,
    Forward,
    Up,
    Down,
    PageUp,
    PageDown,
    First,
    Last,
}

impl GroupedMove {
    /// The move `key` makes, Left and Right swapped when `rtl`.
    fn of(key: gdk::Key, rtl: bool) -> Option<Self> {
        let (left, right) = if rtl {
            (GroupedMove::Forward, GroupedMove::Back)
        } else {
            (GroupedMove::Back, GroupedMove::Forward)
        };
        Some(match key {
            gdk::Key::Left | gdk::Key::KP_Left => left,
            gdk::Key::Right | gdk::Key::KP_Right => right,
            gdk::Key::Up | gdk::Key::KP_Up => GroupedMove::Up,
            gdk::Key::Down | gdk::Key::KP_Down => GroupedMove::Down,
            gdk::Key::Page_Up | gdk::Key::KP_Page_Up => GroupedMove::PageUp,
            gdk::Key::Page_Down | gdk::Key::KP_Page_Down => GroupedMove::PageDown,
            gdk::Key::Home | gdk::Key::KP_Home => GroupedMove::First,
            gdk::Key::End | gdk::Key::KP_End => GroupedMove::Last,
            _ => return None,
        })
    }

    /// Whether the move goes up or down rows, keeping to a column.
    fn keeps_column(self) -> bool {
        matches!(
            self,
            GroupedMove::Up | GroupedMove::Down | GroupedMove::PageUp | GroupedMove::PageDown
        )
    }
}

/// Where `movement` goes from `from` in `layout`, keeping to `column` and
/// moving `page` rows a page. Left and Right go to the shown item before
/// or after, Home and End to the first and last; Up and Down a row, to a
/// heading on the way; Page Up and Page Down a page of rows of items.
fn grouped_step(
    layout: &SlotLayout,
    movement: GroupedMove,
    from: GroupTarget,
    column: u32,
    page: u32,
) -> Option<GroupTarget> {
    let item = |position: Option<u32>| position.map(GroupTarget::Item).filter(|to| *to != from);
    let rows = |down: bool, count: u32| {
        let mut at = from;
        for _ in 0..count.max(1) {
            let next = match at {
                GroupTarget::Item(position) => layout.row_step(position, column, down).map(GroupTarget::Item),
                GroupTarget::Heading(_) => layout.row_target(at, column, down),
            };
            match next {
                Some(next) => at = next,
                None => break,
            }
        }
        (at != from).then_some(at)
    };
    match (movement, from) {
        (GroupedMove::Back, GroupTarget::Item(position)) => item(layout.item_step(position, false)),
        (GroupedMove::Forward, GroupTarget::Item(position)) => item(layout.item_step(position, true)),
        (GroupedMove::Back | GroupedMove::Forward, GroupTarget::Heading(_)) => None,
        (GroupedMove::Up, _) => layout.row_target(from, column, false),
        (GroupedMove::Down, _) => layout.row_target(from, column, true),
        (GroupedMove::PageUp, _) => rows(false, page),
        (GroupedMove::PageDown, _) => rows(true, page),
        (GroupedMove::First, _) => item(layout.end_item(false)),
        (GroupedMove::Last, _) => item(layout.end_item(true)),
    }
}

impl BrowserWindow {
    /// Handles a move in grouped Tiles and Content, whose grid lists the
    /// groups' headings among the items: GTK's own keys would stop at a
    /// heading. The keys move between items across the groups, Shift
    /// extends the selection from where it started and Ctrl moves the
    /// focus alone, as in Details. `None` for every other key or view.
    pub(super) fn grouped_grid_key(
        &self,
        key: gdk::Key,
        modifiers: gdk::ModifierType,
    ) -> Option<glib::Propagation> {
        let others =
            gdk::ModifierType::ALT_MASK | gdk::ModifierType::SUPER_MASK | gdk::ModifierType::META_MASK;
        if modifiers.intersects(others) {
            return None;
        }
        let pane = self.folder_pane();
        let icons = pane.icon_view();
        let grouped = icons.grouped()?;
        let movement = GroupedMove::of(key, icons.direction() == gtk::TextDirection::Rtl)?;
        let layout = grouped.layout()?;
        let focus = GtkWindowExt::focus(self)?;
        let from = match pane.owners().position_of(&focus) {
            Some(position) => GroupTarget::Item(position),
            None => GroupTarget::Heading(self.focused_heading()?.group_index()?),
        };
        let remembered = self
            .imp()
            .grid_column
            .get()
            .filter(|memory| GroupTarget::Item(memory.position) == from);
        let column = match (remembered, from) {
            (Some(memory), _) => memory.column,
            (None, GroupTarget::Item(position)) => layout.column_of(position).unwrap_or(0),
            (None, GroupTarget::Heading(_)) => 0,
        };
        let Some(to) = grouped_step(&layout, movement, from, column, icons.rows_per_page()) else {
            return Some(glib::Propagation::Stop);
        };
        self.reset_typeahead();
        let to = match to {
            GroupTarget::Item(to) => to,
            GroupTarget::Heading(index) => {
                // The heading takes the keyboard and, as in Windows 11, its
                // group's items the selection: Shift extends it to them,
                // Ctrl moves the keyboard alone.
                self.imp().grid_column.set(None);
                let extends = modifiers.contains(gdk::ModifierType::SHIFT_MASK);
                if !modifiers.contains(gdk::ModifierType::CONTROL_MASK) {
                    grouped.select_group(index, false, extends);
                }
                if let Some(heading) = grouped.heading(index) {
                    self.keep_heading_focus(&heading);
                }
                return Some(glib::Propagation::Stop);
            }
        };
        let memory = movement
            .keeps_column()
            .then_some(GridColumn { column, position: to });
        self.imp().grid_column.set(memory);
        let extends = modifiers.contains(gdk::ModifierType::SHIFT_MASK);
        let keeps = modifiers.contains(gdk::ModifierType::CONTROL_MASK);
        if extends {
            let anchor = grouped.anchor().unwrap_or(match from {
                GroupTarget::Item(position) => position,
                GroupTarget::Heading(_) => to,
            });
            grouped.select_shown_between(anchor, to, !keeps);
            pane.reveal(to);
        } else if keeps {
            pane.reveal(to);
        } else {
            pane.select_and_reveal(to);
        }
        Some(glib::Propagation::Stop)
    }

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
        if !modifiers.is_empty() || !matches!(pane.view(), FolderView::Icons(_) | FolderView::Tiles) {
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
    use crate::folder_view::group_slots::{collapsed_runs, group_runs, SlotLayout};

    /// Every key moves between items in grouped Tiles; Up and Down rest
    /// on a heading on the way, and Page Down stops at the last row.
    ///
    /// parity: VIEW-071, VIEW-022
    #[test]
    fn grouped_keys_move_between_the_items_across_the_headings() {
        use GroupTarget::{Heading, Item};
        let titles = ["Today", "Today", "Today", "Today", "Yesterday", "Older", "Older"];
        let layout = SlotLayout::new(group_runs(titles), 3);
        let step = |movement, from, column| grouped_step(&layout, movement, from, column, 2);
        assert_eq!(
            step(GroupedMove::Forward, Item(3), 0),
            Some(Item(4)),
            "Right into the next group"
        );
        assert_eq!(step(GroupedMove::Back, Item(4), 0), Some(Item(3)));
        assert_eq!(step(GroupedMove::Back, Item(0), 0), None);
        assert_eq!(step(GroupedMove::Forward, Item(6), 0), None);
        assert_eq!(
            step(GroupedMove::Down, Item(1), 1),
            Some(Item(3)),
            "a shorter row's last item"
        );
        assert_eq!(
            step(GroupedMove::Down, Item(3), 1),
            Some(Heading(1)),
            "the next heading"
        );
        assert_eq!(step(GroupedMove::Down, Heading(1), 1), Some(Item(4)));
        assert_eq!(
            step(GroupedMove::Up, Item(0), 0),
            Some(Heading(0)),
            "its own heading"
        );
        assert_eq!(step(GroupedMove::Up, Heading(0), 0), None);
        assert_eq!(step(GroupedMove::Up, Heading(1), 0), Some(Item(3)));
        assert_eq!(
            step(GroupedMove::PageDown, Item(0), 0),
            Some(Item(4)),
            "two rows down"
        );
        assert_eq!(
            step(GroupedMove::PageDown, Item(4), 1),
            Some(Item(6)),
            "to the last row"
        );
        assert_eq!(step(GroupedMove::PageUp, Item(6), 1), Some(Item(3)));
        assert_eq!(step(GroupedMove::First, Item(5), 0), Some(Item(0)));
        assert_eq!(step(GroupedMove::Last, Item(5), 0), Some(Item(6)));
        assert_eq!(step(GroupedMove::Last, Item(6), 0), None);
        assert_eq!(GroupedMove::of(gdk::Key::Left, true), Some(GroupedMove::Forward));
        assert_eq!(GroupedMove::of(gdk::Key::a, false), None);
    }

    /// A collapsed group's items are passed by: Right, Down and End skip
    /// them, and Down from its heading goes to the next heading.
    ///
    /// parity: VIEW-022
    #[test]
    fn grouped_keys_pass_a_collapsed_group_by() {
        use GroupTarget::{Heading, Item};
        let titles = ["Today", "Today", "Yesterday", "Yesterday", "Older"];
        let collapsed = std::collections::HashSet::from(["Yesterday".to_owned()]);
        let layout = SlotLayout::new(collapsed_runs(titles, &collapsed), 3);
        let step = |movement, from| grouped_step(&layout, movement, from, 0, 2);
        assert_eq!(step(GroupedMove::Forward, Item(1)), Some(Item(4)));
        assert_eq!(step(GroupedMove::Back, Item(4)), Some(Item(1)));
        assert_eq!(step(GroupedMove::Down, Item(0)), Some(Heading(1)));
        assert_eq!(step(GroupedMove::Down, Heading(1)), Some(Heading(2)));
        assert_eq!(step(GroupedMove::Up, Heading(2)), Some(Heading(1)));
        assert_eq!(step(GroupedMove::Up, Heading(1)), Some(Item(0)));
        assert_eq!(step(GroupedMove::PageDown, Item(0)), Some(Item(4)));
        assert_eq!(step(GroupedMove::Last, Item(0)), Some(Item(4)));
    }

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
