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
use crate::folder_view::group_slots::{GroupTarget, RowHeights, SlotLayout};

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

/// Where `movement` goes from `from` in `layout`, keeping to `column`; a
/// page is as tall as `heights` says. Left and Right go to the shown item
/// before or after, Home and End to the first and last; Up and Down a row,
/// to a heading on the way; Page Up and Page Down a page.
fn grouped_step(
    layout: &SlotLayout,
    movement: GroupedMove,
    from: GroupTarget,
    column: u32,
    heights: RowHeights,
) -> Option<GroupTarget> {
    let item = |position: Option<u32>| position.map(GroupTarget::Item).filter(|to| *to != from);
    if layout.heading_slots() == 1 && layout.columns() > 1 {
        return list_step(layout, movement, from, column);
    }
    // A page moves as far as the view shows, measured in pixels: the
    // heading rows it passes count at their own height, which is less
    // than a row of items. It always moves at least one row.
    let rows = |down: bool| {
        let start = layout.row_of(from)?;
        let mut at = from;
        loop {
            let next = match at {
                GroupTarget::Item(position) => layout.row_step(position, column, down).map(GroupTarget::Item),
                GroupTarget::Heading(_) => layout.row_target(at, column, down),
            };
            let Some(next) = next else { break };
            if at != from && heights.between(layout, start, layout.row_of(next)?) > heights.page {
                break;
            }
            at = next;
        }
        (at != from).then_some(at)
    };
    match (movement, from) {
        (GroupedMove::Back, GroupTarget::Item(position)) => item(layout.item_step(position, false)),
        (GroupedMove::Forward, GroupTarget::Item(position)) => item(layout.item_step(position, true)),
        (GroupedMove::Back | GroupedMove::Forward, GroupTarget::Heading(_)) => None,
        (GroupedMove::Up, _) => layout.row_target(from, column, false),
        (GroupedMove::Down, _) => layout.row_target(from, column, true),
        (GroupedMove::PageUp, _) => rows(false),
        (GroupedMove::PageDown, _) => rows(true),
        (GroupedMove::First, _) => item(layout.end_item(false)),
        (GroupedMove::Last, _) => item(layout.end_item(true)),
    }
}

/// Where `movement` goes from `from` in grouped List, whose lines of slots
/// are columns on screen, keeping to `column`, the row on screen: Up and
/// Down go to the item before or after, across the groups, Left and Right
/// (and Page Up and Page Down) to the same row of the column before or
/// after, Home and End to the first and last item. A heading takes the
/// keyboard only when it is clicked, which selects its group: the keys
/// then go on from there, as from the group's first item on its line.
fn list_step(
    layout: &SlotLayout,
    movement: GroupedMove,
    from: GroupTarget,
    column: u32,
) -> Option<GroupTarget> {
    let position = match from {
        GroupTarget::Item(position) => position,
        GroupTarget::Heading(index) => return list_heading_step(layout, movement, index, column),
    };
    let to = match movement {
        GroupedMove::Up => layout.item_step(position, false),
        GroupedMove::Down => layout.item_step(position, true),
        GroupedMove::Back | GroupedMove::PageUp => layout.row_step(position, column, false),
        GroupedMove::Forward | GroupedMove::PageDown => layout.row_step(position, column, true),
        GroupedMove::First => layout.end_item(false),
        GroupedMove::Last => layout.end_item(true),
    };
    to.filter(|to| *to != position).map(GroupTarget::Item)
}

/// Where `movement` goes in grouped List from the heading of the group at
/// `index`, which a click gave the keyboard: the heading leads its
/// group's items down its column, so Down goes to the group's first item,
/// Up to the item before it (the last of the group before), Left and Right
/// (and Page Up and Page Down) to the same row of the column before or
/// after, as from that first item, and Home and End to the first and last
/// item. Clicking a heading in a long folder used to send every key back
/// to the folder's first item.
fn list_heading_step(
    layout: &SlotLayout,
    movement: GroupedMove,
    index: usize,
    column: u32,
) -> Option<GroupTarget> {
    let start = layout.groups().get(index)?.start;
    let first = layout.shown_items().find(|shown| *shown >= start);
    let to = match movement {
        GroupedMove::Up => layout.item_step(start, false),
        GroupedMove::Down => first,
        GroupedMove::Back | GroupedMove::PageUp => {
            first.and_then(|first| layout.row_step(first, column, false))
        }
        GroupedMove::Forward | GroupedMove::PageDown => {
            first.and_then(|first| layout.row_step(first, column, true))
        }
        GroupedMove::First => layout.end_item(false),
        GroupedMove::Last => layout.end_item(true),
    };
    to.map(GroupTarget::Item)
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
        let Some(to) = grouped_step(&layout, movement, from, column, icons.row_heights()) else {
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
        // List keeps to the row on screen across its columns, the other
        // layouts to the column across their rows.
        let keeps_place = if grouped.is_list() {
            !movement.keeps_column() || matches!(movement, GroupedMove::PageUp | GroupedMove::PageDown)
        } else {
            movement.keeps_column()
        };
        let keeps_place = keeps_place && !matches!(movement, GroupedMove::First | GroupedMove::Last);
        let memory = keeps_place.then_some(GridColumn { column, position: to });
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
        icons.show_heading_above(to);
        Some(glib::Propagation::Stop)
    }

    /// Handles Home and End in grouped Details: they go to the first and
    /// last item shown, as in Windows 11, Shift extending the selection
    /// from where it started and Ctrl moving the keyboard alone. GTK's own
    /// keys went to the list's first row, the first group's heading, which
    /// then selected the whole group. Home scrolls to the top, so the
    /// first heading shows. `None` for every other key or view.
    pub(super) fn grouped_details_end_key(
        &self,
        key: gdk::Key,
        modifiers: gdk::ModifierType,
    ) -> Option<glib::Propagation> {
        let last = match key {
            gdk::Key::Home | gdk::Key::KP_Home => false,
            gdk::Key::End | gdk::Key::KP_End => true,
            _ => return None,
        };
        let others =
            gdk::ModifierType::ALT_MASK | gdk::ModifierType::SUPER_MASK | gdk::ModifierType::META_MASK;
        let pane = self.folder_pane();
        if modifiers.intersects(others) || pane.view() != FolderView::Details {
            return None;
        }
        let grouped = pane.details().grouped()?;
        let to = grouped.layout()?.end_item(last)?;
        self.reset_typeahead();
        self.imp().grid_column.set(None);
        let from = pane.focused_position();
        let extends = modifiers.contains(gdk::ModifierType::SHIFT_MASK);
        let keeps = modifiers.contains(gdk::ModifierType::CONTROL_MASK);
        if extends {
            let anchor = grouped.anchor().or(from).unwrap_or(to);
            grouped.select_shown_between(anchor, to, !keeps);
            pane.reveal(to);
        } else if keeps {
            pane.reveal(to);
        } else {
            pane.select_and_reveal(to);
        }
        if !last {
            let adjustment = pane.details().vadjustment();
            adjustment.set_value(adjustment.lower());
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
        // Small icons, like List, fill columns top to bottom, where GTK's
        // own Up and Down already move along the column.
        let in_rows = matches!(pane.view(), FolderView::Icons(_) | FolderView::Tiles)
            && !pane.icon_view().layout().flows_sideways();
        if !modifiers.is_empty() || !in_rows {
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

    /// A page two rows of items tall, with heading rows about half as tall.
    const PAGE: RowHeights = RowHeights {
        page: 150.0,
        item: 75,
        heading: 39,
    };

    /// Every key moves between items in grouped Tiles; Up and Down rest
    /// on a heading on the way, and Page Down stops at the last row.
    ///
    /// parity: VIEW-071, VIEW-022
    #[test]
    fn grouped_keys_move_between_the_items_across_the_headings() {
        use GroupTarget::{Heading, Item};
        let titles = ["Today", "Today", "Today", "Today", "Yesterday", "Older", "Older"];
        let layout = SlotLayout::new(group_runs(titles), 3);
        let step = |movement, from, column| grouped_step(&layout, movement, from, column, PAGE);
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
            Some(Item(3)),
            "two rows down: the next row of items, then a heading row"
        );
        assert_eq!(
            step(GroupedMove::PageDown, Item(4), 1),
            Some(Item(6)),
            "to the last row"
        );
        assert_eq!(
            step(GroupedMove::PageUp, Item(6), 1),
            Some(Item(4)),
            "two rows up, the heading between counted"
        );
        assert_eq!(step(GroupedMove::First, Item(5), 0), Some(Item(0)));
        assert_eq!(step(GroupedMove::Last, Item(5), 0), Some(Item(6)));
        assert_eq!(step(GroupedMove::Last, Item(6), 0), None);
        assert_eq!(GroupedMove::of(gdk::Key::Left, true), Some(GroupedMove::Forward));
    }

    /// With many small groups a page moves as far as the view shows: a
    /// heading row counts at its own height, not as a row of items, and
    /// not as nothing.
    ///
    /// parity: VIEW-071
    #[test]
    fn a_page_measures_heading_rows_at_their_height() {
        use GroupTarget::Item;
        let titles = ["A", "B", "C", "D", "E", "F", "G"];
        let layout = SlotLayout::new(group_runs(titles), 3);
        let heights = |page| RowHeights {
            page,
            item: 75,
            heading: 39,
        };
        // Each item is a heading (39) and a row (75) below the last.
        let down = |page| grouped_step(&layout, GroupedMove::PageDown, Item(0), 0, heights(page));
        assert_eq!(down(300.0), Some(Item(2)), "two groups fit in 300 pixels");
        assert_eq!(down(400.0), Some(Item(3)), "three fit in 400, not 400 / 75 rows");
        assert_eq!(down(50.0), Some(Item(1)), "at least one row");
        let up = grouped_step(&layout, GroupedMove::PageUp, Item(6), 0, heights(400.0));
        assert_eq!(up, Some(Item(3)));
        assert_eq!(GroupedMove::of(gdk::Key::a, false), None);
    }

    /// Page Down counts the heading rows it passes, so a page of many
    /// small groups moves about as far as the view shows, not that many
    /// rows of items plus every heading between (Codex review of #289).
    ///
    /// parity: VIEW-071, VIEW-022
    #[test]
    fn a_page_counts_the_heading_rows_it_passes() {
        use GroupTarget::Item;
        let titles = ["A", "B", "C", "D", "E", "F"];
        let layout = SlotLayout::new(group_runs(titles), 3);
        // Rows: heading A, A's item, heading B, B's item, heading C, ...
        let step = |movement, from| grouped_step(&layout, movement, from, 0, PAGE);
        assert_eq!(
            step(GroupedMove::PageDown, Item(0)),
            Some(Item(1)),
            "two rows: a heading and an item"
        );
        assert_eq!(step(GroupedMove::PageUp, Item(3)), Some(Item(2)));
        assert_eq!(
            step(GroupedMove::PageDown, Item(5)),
            None,
            "nothing below the last row"
        );
        let one_group = SlotLayout::new(group_runs(["A"; 9]), 3);
        assert_eq!(
            grouped_step(&one_group, GroupedMove::PageDown, Item(0), 0, PAGE),
            Some(Item(6)),
            "without headings a page is that many rows of items"
        );
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
        let step = |movement, from| grouped_step(&layout, movement, from, 0, PAGE);
        assert_eq!(step(GroupedMove::Forward, Item(1)), Some(Item(4)));
        assert_eq!(step(GroupedMove::Back, Item(4)), Some(Item(1)));
        assert_eq!(step(GroupedMove::Down, Item(0)), Some(Heading(1)));
        assert_eq!(step(GroupedMove::Down, Heading(1)), Some(Heading(2)));
        assert_eq!(step(GroupedMove::Up, Heading(2)), Some(Heading(1)));
        assert_eq!(step(GroupedMove::Up, Heading(1)), Some(Item(0)));
        assert_eq!(step(GroupedMove::PageDown, Item(0)), Some(Item(4)));
        assert_eq!(step(GroupedMove::Last, Item(0)), Some(Item(4)));
    }

    /// In grouped List a heading a click gave the keyboard leads its
    /// group's items: Down goes to the group's first item, Up to the item
    /// before it, Right to the same row of the next column, Left to that
    /// of the column before, and Home and End to the first and last item.
    /// Every key used to go to the folder's first item.
    ///
    /// parity: VIEW-022, VIEW-008
    #[test]
    fn keys_go_on_from_a_clicked_list_heading() {
        use GroupTarget::{Heading, Item};
        // Lines of four slots: the heading of A, its items 0 to 2; the
        // heading of B, its items 3 to 5; then B's 6 and 7.
        let titles = ["A", "A", "A", "B", "B", "B", "B", "B"];
        let layout = SlotLayout::inline(group_runs(titles), 4);
        let step = |movement| grouped_step(&layout, movement, Heading(1), 0, PAGE);
        assert_eq!(step(GroupedMove::Down), Some(Item(3)), "the group's first item");
        assert_eq!(step(GroupedMove::Up), Some(Item(2)), "the item before the group");
        assert_eq!(step(GroupedMove::Forward), Some(Item(6)), "the next column");
        assert_eq!(step(GroupedMove::Back), Some(Item(0)), "the column before");
        assert_eq!(step(GroupedMove::First), Some(Item(0)));
        assert_eq!(step(GroupedMove::Last), Some(Item(7)));
        assert_eq!(
            grouped_step(&layout, GroupedMove::Up, Heading(0), 0, PAGE),
            None,
            "nothing before the first group"
        );
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
