// SPDX-License-Identifier: AGPL-3.0-only
//! Where a grouped listing's headings and items fall in the icon view's
//! grid, for Tiles and Content (VIEW-022, VIEW-071).
//!
//! Windows 11 keeps a folder's groups in Tiles and Content: each group
//! starts on a row of its own under a heading ("Today (3)" and a line to
//! the right edge), as in Details. GTK's grid has no headings of its own,
//! so the grid lists slots: a heading row of `columns` slots (the heading,
//! then the line it draws across the rest of the row), the group's items,
//! and blanks that fill its last row, so the next group starts a row.
//! [`SlotLayout`] maps the grid's slots to the items of the listing and
//! back, and finds the item Up and Down go to across the groups.
//!
//! Details lists the same slots in one column. A collapsed group keeps its
//! heading and lists none of its items, as Windows 11 collapses a group
//! under its chevron: its items stay in the listing, and selected, but no
//! slot shows them, so the keys and a Shift range pass them by.

/// What one slot of the grid shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Slot {
    /// The heading of the group with this index.
    Heading(usize),
    /// The line of a heading, past its first slot.
    Rule,
    /// The item at this position of the listing.
    Item(u32),
    /// Nothing: the rest of a group's last row.
    Blank,
}

/// One group of the listing: its title, the positions of its items and
/// whether it is collapsed under its heading.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct GroupRun {
    pub title: String,
    pub start: u32,
    pub end: u32,
    pub collapsed: bool,
}

impl GroupRun {
    /// How many items the group holds.
    pub(crate) fn len(&self) -> u32 {
        self.end - self.start
    }

    /// How many of its items the group shows: none while collapsed.
    pub(crate) fn shown(&self) -> u32 {
        if self.collapsed {
            0
        } else {
            self.len()
        }
    }
}

/// Where the keyboard is in a grouped listing: on an item, or on the
/// heading of the group with this index.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum GroupTarget {
    Item(u32),
    Heading(usize),
}

/// Splits the titles of a listing's items, in order, into runs of equal
/// titles: the listing's groups, as it is sorted by group.
pub(crate) fn group_runs<'a>(titles: impl IntoIterator<Item = &'a str>) -> Vec<GroupRun> {
    let mut runs: Vec<GroupRun> = Vec::new();
    for (position, title) in (0u32..).zip(titles) {
        match runs.last_mut() {
            Some(run) if run.title == title => run.end = position + 1,
            _ => runs.push(GroupRun {
                title: title.to_owned(),
                start: position,
                end: position + 1,
                collapsed: false,
            }),
        }
    }
    runs
}

/// [`group_runs`] with the groups whose titles `collapsed` holds
/// collapsed.
pub(crate) fn collapsed_runs<'a>(
    titles: impl IntoIterator<Item = &'a str>,
    collapsed: &std::collections::HashSet<String>,
) -> Vec<GroupRun> {
    let mut runs = group_runs(titles);
    for run in &mut runs {
        run.collapsed = collapsed.contains(&run.title);
    }
    runs
}

/// A listing change: at `position`, `removed` items went and `added` came.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ItemsChange {
    pub position: u32,
    pub removed: u32,
    pub added: u32,
}

impl ItemsChange {
    /// Where the item that was at `old` is now, if it stayed.
    pub(crate) fn moved(self, old: u32) -> Option<u32> {
        if old < self.position {
            Some(old)
        } else if old >= self.position + self.removed {
            Some(old - self.removed + self.added)
        } else {
            None
        }
    }
}

/// The grid's slots for a grouped listing in `columns` columns.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SlotLayout {
    columns: u32,
    groups: Vec<GroupRun>,
    /// The first slot of each group: its heading.
    starts: Vec<u32>,
    len: u32,
}

impl SlotLayout {
    /// The slots of `groups` in `columns` columns (at least one).
    pub(crate) fn new(groups: Vec<GroupRun>, columns: u32) -> Self {
        let columns = columns.max(1);
        let mut starts = Vec::with_capacity(groups.len());
        let mut len = 0;
        for group in &groups {
            starts.push(len);
            let rows = group.shown().div_ceil(columns);
            len += columns * (1 + rows);
        }
        Self {
            columns,
            groups,
            starts,
            len,
        }
    }

    /// How many slots the grid lists.
    pub(crate) fn len(&self) -> u32 {
        self.len
    }

    /// The columns the slots are laid out for.
    pub(crate) fn columns(&self) -> u32 {
        self.columns
    }

    /// The groups, in order.
    pub(crate) fn groups(&self) -> &[GroupRun] {
        &self.groups
    }

    /// The group whose slots hold `slot`.
    fn group_at(&self, slot: u32) -> Option<usize> {
        if slot >= self.len {
            return None;
        }
        Some(self.starts.partition_point(|start| *start <= slot) - 1)
    }

    /// The group whose slots hold `slot` and how far into them it is.
    pub(crate) fn place(&self, slot: u32) -> Option<(usize, u32)> {
        let index = self.group_at(slot)?;
        Some((index, slot - self.starts[index]))
    }

    /// What `slot` shows; `None` past the end.
    pub(crate) fn slot(&self, slot: u32) -> Option<Slot> {
        let index = self.group_at(slot)?;
        let group = &self.groups[index];
        let offset = slot - self.starts[index];
        Some(if offset == 0 {
            Slot::Heading(index)
        } else if offset < self.columns {
            Slot::Rule
        } else if offset - self.columns < group.shown() {
            Slot::Item(group.start + offset - self.columns)
        } else {
            Slot::Blank
        })
    }

    /// The listing position the slot shows, if it shows an item.
    pub(crate) fn item_at(&self, slot: u32) -> Option<u32> {
        match self.slot(slot)? {
            Slot::Item(position) => Some(position),
            _ => None,
        }
    }

    /// The slot showing the item at `position` of the listing; `None`
    /// while its group is collapsed.
    pub(crate) fn slot_of(&self, position: u32) -> Option<u32> {
        let index = self.group_of(position)?;
        let group = &self.groups[index];
        if group.collapsed {
            return None;
        }
        Some(self.starts[index] + self.columns + position - group.start)
    }

    /// The index of the group holding the item at `position`.
    pub(crate) fn group_of(&self, position: u32) -> Option<usize> {
        let index = self.groups.partition_point(|group| group.end <= position);
        self.groups
            .get(index)
            .filter(|group| group.start <= position)
            .map(|_| index)
    }

    /// The slot of the heading of the group at `index`.
    pub(crate) fn heading_slot(&self, index: usize) -> Option<u32> {
        self.starts.get(index).copied()
    }

    /// Whether a slot shows the item at `position`: its group is not
    /// collapsed.
    pub(crate) fn shows(&self, position: u32) -> bool {
        self.slot_of(position).is_some()
    }

    /// The listing positions of the items the slots show, in order.
    pub(crate) fn shown_items(&self) -> impl Iterator<Item = u32> + '_ {
        self.groups
            .iter()
            .filter(|group| !group.collapsed)
            .flat_map(|group| group.start..group.end)
    }

    /// The shown item before (`forward` false) or after `position`,
    /// passing over collapsed groups.
    pub(crate) fn item_step(&self, position: u32, forward: bool) -> Option<u32> {
        if forward {
            self.shown_items().find(|shown| *shown > position)
        } else {
            self.shown_items().filter(|shown| *shown < position).last()
        }
    }

    /// The first (`last` false) or last item the slots show.
    pub(crate) fn end_item(&self, last: bool) -> Option<u32> {
        if last {
            self.shown_items().last()
        } else {
            self.shown_items().next()
        }
    }

    /// The listing positions the slots `slot..slot + count` show.
    pub(crate) fn items_in_slots(&self, slot: u32, count: u32) -> Vec<u32> {
        let end = slot.saturating_add(count).min(self.len);
        (slot..end).filter_map(|slot| self.item_at(slot)).collect()
    }

    /// The listing positions the slots show from the item at `from` to
    /// the item at `to`, whichever comes first.
    pub(crate) fn shown_between(&self, from: u32, to: u32) -> Vec<u32> {
        let (low, high) = (from.min(to), from.max(to));
        self.shown_items()
            .filter(|position| (low..=high).contains(position))
            .collect()
    }

    /// Where Up (`down` false) or Down goes from `from`, a step of one
    /// row: the item in `column` of the next row of items, as
    /// [`Self::row_step`], but stopping at a heading between them, as
    /// Windows 11 gives a group's heading the keyboard on the way. From a
    /// heading, Down goes to its group's first row, or to the next heading
    /// while the group is collapsed, and Up to the previous group's last
    /// row, or its heading while that one is collapsed.
    pub(crate) fn row_target(&self, from: GroupTarget, column: u32, down: bool) -> Option<GroupTarget> {
        let mut row = match from {
            GroupTarget::Item(position) => self.slot_of(position)? / self.columns,
            GroupTarget::Heading(index) => self.heading_slot(index)? / self.columns,
        };
        let rows = self.len / self.columns;
        loop {
            row = if down {
                Some(row + 1).filter(|row| *row < rows)?
            } else {
                row.checked_sub(1)?
            };
            if let Some(Slot::Heading(index)) = self.slot(row * self.columns) {
                return Some(GroupTarget::Heading(index));
            }
            if let Some(to) = self.item_in_row(row, column) {
                return Some(GroupTarget::Item(to));
            }
        }
    }

    /// The item at `column` of the slot row `row`, or its last item when
    /// the row is shorter; `None` for a heading row or past the end.
    fn item_in_row(&self, row: u32, column: u32) -> Option<u32> {
        let first = row.checked_mul(self.columns)?;
        let last = first + column.min(self.columns - 1);
        (first..=last).rev().find_map(|slot| self.item_at(slot))
    }

    /// The column the item at `position` is in.
    pub(crate) fn column_of(&self, position: u32) -> Option<u32> {
        Some(self.slot_of(position)? % self.columns)
    }

    /// Where Up (`down` false) or Down from the item at `position` goes,
    /// keeping to `column`: the item in that column of the next row of
    /// items, over a heading into the next group, or that row's last item
    /// when it is shorter. `None` at the first or last row.
    pub(crate) fn row_step(&self, position: u32, column: u32, down: bool) -> Option<u32> {
        let mut row = self.slot_of(position)? / self.columns;
        let rows = self.len / self.columns;
        loop {
            row = if down {
                Some(row + 1).filter(|row| *row < rows)?
            } else {
                row.checked_sub(1)?
            };
            if let Some(to) = self.item_in_row(row, column) {
                return Some(to);
            }
        }
    }

    /// The slots that differ between `self`, laid out before `change`, and
    /// `new`, laid out after it, as a grid model reports a change: the
    /// first slot that differs, how many went and how many came. Items
    /// match where `change` left them, headings by title. `None` when
    /// nothing differs.
    pub(crate) fn changes_to(
        &self,
        new: &SlotLayout,
        change: Option<ItemsChange>,
    ) -> Option<(u32, u32, u32)> {
        let same = |old_slot: u32, new_slot: u32| match (self.slot(old_slot), new.slot(new_slot)) {
            (Some(Slot::Heading(old)), Some(Slot::Heading(now))) => {
                self.groups[old].title == new.groups[now].title
            }
            (Some(Slot::Item(old)), Some(Slot::Item(now))) => {
                change.map_or(Some(old), |change| change.moved(old)) == Some(now)
            }
            (Some(Slot::Rule), Some(Slot::Rule)) | (Some(Slot::Blank), Some(Slot::Blank)) => true,
            _ => false,
        };
        let shorter = self.len.min(new.len);
        let prefix = (0..shorter).find(|slot| !same(*slot, *slot)).unwrap_or(shorter);
        let room = shorter - prefix;
        let suffix = (0..room)
            .find(|back| !same(self.len - 1 - back, new.len - 1 - back))
            .unwrap_or(room);
        let removed = self.len - prefix - suffix;
        let added = new.len - prefix - suffix;
        (removed > 0 || added > 0).then_some((prefix, removed, added))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Today: three items; Yesterday: one; Earlier this week: four.
    fn listing() -> Vec<GroupRun> {
        let titles = [
            "Today",
            "Today",
            "Today",
            "Yesterday",
            "Earlier this week",
            "Earlier this week",
            "Earlier this week",
            "Earlier this week",
        ];
        group_runs(titles)
    }

    #[test]
    fn equal_titles_in_a_row_make_a_group() {
        let runs = listing();
        let spans: Vec<(&str, u32, u32)> = runs
            .iter()
            .map(|run| (run.title.as_str(), run.start, run.end))
            .collect();
        assert_eq!(
            spans,
            [("Today", 0, 3), ("Yesterday", 3, 4), ("Earlier this week", 4, 8)]
        );
        assert!(group_runs([]).is_empty());
    }

    /// Each group starts a row under its heading, and blanks fill its last
    /// row, as Windows 11 lays out grouped Tiles.
    ///
    /// parity: VIEW-071, VIEW-022
    #[test]
    fn each_group_starts_a_row_under_its_heading() {
        use Slot::{Blank, Heading, Item, Rule};
        let layout = SlotLayout::new(listing(), 3);
        let slots: Vec<Slot> = (0..layout.len()).filter_map(|slot| layout.slot(slot)).collect();
        assert_eq!(
            slots,
            [
                Heading(0),
                Rule,
                Rule,
                Item(0),
                Item(1),
                Item(2),
                Heading(1),
                Rule,
                Rule,
                Item(3),
                Blank,
                Blank,
                Heading(2),
                Rule,
                Rule,
                Item(4),
                Item(5),
                Item(6),
                Item(7),
                Blank,
                Blank,
            ]
        );
        assert_eq!(layout.slot(layout.len()), None);
        for position in 0..8 {
            let slot = layout.slot_of(position).expect("every item has a slot");
            assert_eq!(layout.item_at(slot), Some(position));
        }
        assert_eq!(layout.slot_of(8), None);
        assert_eq!(layout.item_at(0), None, "a heading is no item");
    }

    /// Content is one column: a heading, then one row per item.
    #[test]
    fn content_has_a_heading_row_then_one_row_per_item() {
        let layout = SlotLayout::new(listing(), 1);
        assert_eq!(layout.len(), 3 + 8);
        assert_eq!(layout.slot(0), Some(Slot::Heading(0)));
        assert_eq!(layout.slot(4), Some(Slot::Heading(1)));
        assert_eq!(layout.slot_of(3), Some(5));
        assert_eq!(layout.row_step(2, 0, true), Some(3), "Down passes the heading");
        assert_eq!(layout.row_step(3, 0, false), Some(2), "Up passes it too");
        assert_eq!(layout.row_step(0, 0, false), None);
        assert_eq!(layout.row_step(7, 0, true), None);
    }

    /// Up and Down cross headings into the next group, keeping to the
    /// column, or to a shorter row's last item.
    ///
    /// parity: VIEW-071, SEL-010
    #[test]
    fn up_and_down_cross_the_headings_and_keep_the_column() {
        let layout = SlotLayout::new(listing(), 3);
        assert_eq!(layout.row_step(2, 2, true), Some(3), "into Yesterday's one item");
        assert_eq!(layout.row_step(3, 2, true), Some(6), "the column again below it");
        assert_eq!(layout.row_step(6, 2, true), Some(7), "a shorter last row");
        assert_eq!(layout.row_step(7, 2, true), None, "the last row of items");
        assert_eq!(layout.row_step(7, 0, false), Some(4));
        assert_eq!(layout.row_step(4, 0, false), Some(3));
        assert_eq!(layout.row_step(3, 1, false), Some(1));
        assert_eq!(layout.row_step(1, 1, false), None, "the first row of items");
        assert_eq!(layout.column_of(7), Some(0));
        assert_eq!(layout.column_of(6), Some(2));
    }

    /// A file added to a group changes only the slots from it to the end
    /// of the group's last row; a heading stays while its title does.
    #[test]
    fn a_change_covers_the_slots_that_differ() {
        let old = SlotLayout::new(listing(), 3);
        let mut titles: Vec<&str> = vec!["Today"; 4];
        titles.extend(["Yesterday"]);
        titles.extend(["Earlier this week"; 4]);
        let new = SlotLayout::new(group_runs(titles), 3);
        let added = ItemsChange {
            position: 1,
            removed: 0,
            added: 1,
        };
        // From the new item at slot 4 to the end of Today's new row.
        assert_eq!(old.changes_to(&new, Some(added)), Some((4, 2, 5)));
        assert_eq!(old.changes_to(&old, None), None);
        let wider = SlotLayout::new(listing(), 4);
        assert_eq!(old.changes_to(&wider, None), Some((3, 18, 21)));
    }

    /// A collapsed group keeps its heading row and shows none of its
    /// items: no slot holds them, so the rows after it move up, and Down
    /// rests on each heading on the way.
    ///
    /// parity: VIEW-022, VIEW-071
    #[test]
    fn a_collapsed_group_keeps_only_its_heading() {
        use Slot::{Blank, Heading, Item, Rule};
        let titles = [
            "Today",
            "Today",
            "Today",
            "Yesterday",
            "Earlier this week",
            "Earlier this week",
        ];
        let collapsed = std::collections::HashSet::from(["Today".to_owned()]);
        let layout = SlotLayout::new(collapsed_runs(titles, &collapsed), 3);
        let slots: Vec<Slot> = (0..layout.len()).filter_map(|slot| layout.slot(slot)).collect();
        assert_eq!(
            slots,
            [
                Heading(0),
                Rule,
                Rule,
                Heading(1),
                Rule,
                Rule,
                Item(3),
                Blank,
                Blank,
                Heading(2),
                Rule,
                Rule,
                Item(4),
                Item(5),
                Blank,
            ]
        );
        assert_eq!(layout.slot_of(1), None, "Today's items are hidden");
        assert!(!layout.shows(0) && layout.shows(3));
        assert_eq!(layout.group_of(1), Some(0));
        assert_eq!(layout.shown_items().collect::<Vec<_>>(), [3, 4, 5]);
        assert_eq!(layout.item_step(3, false), None, "nothing shown before");
        assert_eq!(layout.end_item(false), Some(3));
        assert_eq!(layout.shown_between(5, 0), [3, 4, 5], "a Shift range skips Today");
        assert_eq!(layout.items_in_slots(0, layout.len()), [3, 4, 5]);
        let down = |from| layout.row_target(from, 0, true);
        assert_eq!(down(GroupTarget::Heading(0)), Some(GroupTarget::Heading(1)));
        assert_eq!(down(GroupTarget::Heading(1)), Some(GroupTarget::Item(3)));
        assert_eq!(down(GroupTarget::Item(3)), Some(GroupTarget::Heading(2)));
        assert_eq!(
            layout.row_target(GroupTarget::Item(3), 0, false),
            Some(GroupTarget::Heading(1))
        );
    }

    /// Details lists the groups in one column: a collapsed group is its
    /// heading row alone.
    ///
    /// parity: VIEW-022
    #[test]
    fn details_lists_a_collapsed_group_as_its_heading_alone() {
        let collapsed = std::collections::HashSet::from(["Yesterday".to_owned()]);
        let titles = ["Today", "Today", "Today", "Yesterday", "Earlier this week"];
        let layout = SlotLayout::new(collapsed_runs(titles, &collapsed), 1);
        assert_eq!(layout.len(), 3 + 3 + 1);
        assert_eq!(layout.heading_slot(1), Some(4));
        assert_eq!(layout.slot(5), Some(Slot::Heading(2)));
        assert_eq!(layout.item_step(2, true), Some(4), "past Yesterday's item");
    }
}
