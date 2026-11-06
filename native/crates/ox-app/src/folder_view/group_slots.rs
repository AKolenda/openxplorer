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

/// One group of the listing: its title and the positions of its items.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct GroupRun {
    pub title: String,
    pub start: u32,
    pub end: u32,
}

impl GroupRun {
    /// How many items the group holds.
    pub(crate) fn len(&self) -> u32 {
        self.end - self.start
    }
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
            }),
        }
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
            let rows = group.len().div_ceil(columns);
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
        } else if offset - self.columns < group.len() {
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

    /// The slot showing the item at `position` of the listing.
    pub(crate) fn slot_of(&self, position: u32) -> Option<u32> {
        let index = self.groups.partition_point(|group| group.end <= position);
        let group = self.groups.get(index).filter(|group| group.start <= position)?;
        Some(self.starts[index] + self.columns + position - group.start)
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
}
