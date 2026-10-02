// SPDX-License-Identifier: AGPL-3.0-only
//! Explorer's "Group by" for the folder list (VIEW-022): which section an
//! item is in, the sections' order and their headings.
//!
//! The groups themselves come from [`ox_core::grouping`]. The folder model
//! sorts by these sections first (GTK's section sorter), so both views
//! show the items group by group; the details view also draws a heading
//! for each section.

use std::cell::RefCell;
use std::cmp::Ordering;
use std::rc::Rc;

use gtk::prelude::*;
use ox_core::grouping::{type_group_label, Calendar, DateRanges, GroupBy, NameGroup, SizeGroup};

use crate::folder_view::item::FileItem;

/// What the list is grouped by, with what grouping by date needs.
#[derive(Debug, Clone, Default)]
pub(crate) struct Grouping {
    group_by: GroupBy,
    /// The day and week the date groups count from, when grouping by
    /// date.
    calendar: Option<Calendar>,
    /// The date groups as time ranges, worked out when grouping by date
    /// starts or the dates are refreshed; `None` if the clock could not
    /// be read, when every date is unknown.
    dates: Option<DateRanges>,
}

impl Grouping {
    /// Grouping by `group_by`, counting dates from today.
    pub(crate) fn new(group_by: GroupBy) -> Self {
        let calendar = match group_by {
            GroupBy::Modified => Calendar::now(),
            GroupBy::None | GroupBy::Name | GroupBy::Type | GroupBy::Size => None,
        };
        Self {
            group_by,
            calendar,
            dates: calendar.map(Calendar::date_ranges),
        }
    }

    /// Whether `other` puts every item in the same group as this one.
    pub(crate) fn groups_like(&self, other: &Grouping) -> bool {
        self.group_by == other.group_by && self.calendar == other.calendar
    }

    /// What the list is grouped by.
    #[cfg(test)]
    pub(crate) fn group_by(&self) -> GroupBy {
        self.group_by
    }

    /// Whether the list is in groups at all.
    pub(crate) fn is_grouped(&self) -> bool {
        self.group_by != GroupBy::None
    }

    /// Compares the groups of two items, in the order the groups are
    /// listed: dates newest first, names and types A to Z, sizes smallest
    /// first.
    pub(crate) fn compare(&self, a: &FileItem, b: &FileItem) -> Ordering {
        match self.group_by {
            GroupBy::None => Ordering::Equal,
            GroupBy::Name => name_group(a).cmp(&name_group(b)),
            GroupBy::Modified => self.date_group(a).cmp(&self.date_group(b)),
            GroupBy::Type => {
                let (a_unspecified, b_unspecified) =
                    (a.entry().type_label.is_empty(), b.entry().type_label.is_empty());
                a_unspecified
                    .cmp(&b_unspecified)
                    .then_with(|| a.type_sort_key().natural_cmp(b.type_sort_key()))
            }
            GroupBy::Size => size_group(a).cmp(&size_group(b)),
        }
    }

    /// The heading of `item`'s group.
    pub(crate) fn label(&self, item: &FileItem) -> String {
        match self.group_by {
            GroupBy::None => String::new(),
            GroupBy::Name => name_group(item).label().to_owned(),
            GroupBy::Modified => self.date_group(item).label().to_owned(),
            GroupBy::Type => type_group_label(&item.entry().type_label).to_owned(),
            GroupBy::Size => size_group(item).label().to_owned(),
        }
    }

    fn date_group(&self, item: &FileItem) -> ox_core::grouping::DateGroup {
        let modified = item.entry().modified;
        self.dates
            .as_ref()
            .map_or(ox_core::grouping::DateGroup::Unknown, |dates| {
                dates.group_of_time(modified)
            })
    }
}

fn name_group(item: &FileItem) -> NameGroup {
    NameGroup::of(&item.entry().name)
}

/// Folders are Unspecified, as in Explorer, even once measured.
fn size_group(item: &FileItem) -> SizeGroup {
    SizeGroup::of(item.file_size())
}

/// A section heading: the group's name and, as Explorer shows it, how many
/// items it holds.
pub(crate) fn heading(label: &str, count: u32) -> String {
    format!("{label} ({count})")
}

/// The details view's section headings: the group's name and count,
/// then a line to the end of the row, as Explorer draws them. A heading
/// follows its section as items are added, removed or regrouped.
pub(crate) fn heading_factory(grouping: Rc<RefCell<Grouping>>) -> gtk::SignalListItemFactory {
    let factory = gtk::SignalListItemFactory::new();
    factory.connect_setup(move |_, object| {
        let Some(header) = object.downcast_ref::<gtk::ListHeader>() else {
            return;
        };
        let label = gtk::Label::builder()
            .xalign(0.0)
            .css_classes(["group-heading-label"])
            .build();
        let line = gtk::Separator::builder()
            .orientation(gtk::Orientation::Horizontal)
            .hexpand(true)
            .valign(gtk::Align::Center)
            .css_classes(["group-heading-line"])
            .build();
        let row = gtk::Box::builder()
            .orientation(gtk::Orientation::Horizontal)
            .css_classes(["group-heading"])
            .build();
        row.append(&label);
        row.append(&line);
        row.set_accessible_role(gtk::AccessibleRole::Heading);
        header.set_child(Some(&row));
        let grouping = Rc::clone(&grouping);
        let update = move |header: &gtk::ListHeader| {
            let text = header
                .item()
                .and_downcast::<FileItem>()
                .map(|item| heading(&grouping.borrow().label(&item), header.n_items()))
                .unwrap_or_default();
            label.set_text(&text);
            if let Some(row) = header.child() {
                row.update_property(&[gtk::accessible::Property::Label(&text)]);
            }
        };
        let update = Rc::new(update);
        let on_item = Rc::clone(&update);
        header.connect_item_notify(move |header| on_item(header));
        header.connect_n_items_notify(move |header| update(header));
    });
    factory
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{file_entry, folder_entry};

    fn file(name: &str, size: Option<u64>, type_label: &str) -> FileItem {
        let mut entry = file_entry(name);
        entry.size = size;
        type_label.clone_into(&mut entry.type_label);
        FileItem::new(entry)
    }

    /// parity: VIEW-022
    #[gtk::test]
    fn items_compare_by_their_groups() {
        let small = file("b.txt", Some(10), "Text document");
        let large = file("a.iso", Some(600 << 20), "Disc image");
        let folder = FileItem::new(folder_entry("Zeta"));

        let by_size = Grouping::new(GroupBy::Size);
        assert_eq!(by_size.compare(&small, &large), Ordering::Less);
        assert_eq!(
            by_size.compare(&folder, &large),
            Ordering::Greater,
            "folders are Unspecified, last"
        );
        assert_eq!(by_size.label(&large), "Large (128 MB – 1 GB)");

        let by_name = Grouping::new(GroupBy::Name);
        assert_eq!(by_name.compare(&large, &small), Ordering::Equal, "both A – H");
        assert_eq!(by_name.compare(&small, &folder), Ordering::Less);
        assert_eq!(by_name.label(&folder), "Q – Z");

        let by_type = Grouping::new(GroupBy::Type);
        let unknown = file("c", None, "");
        assert_eq!(by_type.compare(&large, &small), Ordering::Less);
        assert_eq!(
            by_type.compare(&unknown, &small),
            Ordering::Greater,
            "no type is last"
        );
        assert_eq!(by_type.label(&unknown), "Unspecified");

        let none = Grouping::new(GroupBy::None);
        assert!(!none.is_grouped());
        assert_eq!(none.compare(&small, &large), Ordering::Equal);
        assert_eq!(heading("Today", 3), "Today (3)");
    }
}
