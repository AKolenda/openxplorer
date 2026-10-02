// SPDX-License-Identifier: AGPL-3.0-only
//! Groups of the listing by its sort key: "Show in groups" (VIEW-022).
//!
//! Ports the group roles of Dolphin's `KFileItemModel::groups()`
//! (`nameRoleGroups`, `sizeRoleGroups`, `timeRoleGroups`, `permissionRoleGroups`
//! and the generic role groups): a name groups by its first letter, a size by
//! Windows Explorer's size buckets, a date by period ("Today", "Yesterday",
//! "Earlier this week", …, then the year), and every other key by its text.
//! A [`Group`] orders the groups the way the key sorts its items, so the
//! folder model can sort by group first ([`super::model`]).

use std::cmp::Ordering;

use gtk::glib;

use crate::folder_view::item::FileItem;
use crate::folder_view::sort_roles::{extension, SortBy, SortRole};
use crate::folder_view::sorting::{SortColumn, SortKey};

/// Seconds in a day.
const DAY: i64 = 24 * 60 * 60;

/// Explorer's size groups: the title and the size each one ends below.
const SIZE_GROUPS: [(&str, u64); 6] = [
    ("Tiny (0 – 16 KB)", 16 * 1024),
    ("Small (16 KB – 1 MB)", 1024 * 1024),
    ("Medium (1 – 128 MB)", 128 * 1024 * 1024),
    ("Large (128 MB – 1 GB)", 1024 * 1024 * 1024),
    ("Huge (1 – 4 GB)", 4 * 1024 * 1024 * 1024),
    ("Gigantic (> 4 GB)", u64::MAX),
];

/// The starts of the periods dates are grouped in, worked out once when the
/// listing is grouped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct GroupClock {
    /// The end of today: later dates are in the future.
    tomorrow: i64,
    today: i64,
    yesterday: i64,
    this_week: i64,
    last_week: i64,
    this_month: i64,
    last_month: i64,
    this_year: i64,
}

impl GroupClock {
    /// The periods as of `now`, in its time zone.
    pub(crate) fn at(now: &glib::DateTime) -> Option<Self> {
        let midnight = glib::DateTime::new(
            &now.timezone(),
            now.year(),
            now.month(),
            now.day_of_month(),
            0,
            0,
            0.0,
        )
        .ok()?;
        let today = midnight.to_unix();
        let weekday = i64::from(now.day_of_week() - 1);
        let month_start = |months_back: i32| {
            let date = midnight.add_months(-months_back).ok()?;
            Some(date.to_unix() - i64::from(date.day_of_month() - 1) * DAY)
        };
        let year_start = today - i64::from(now.day_of_year() - 1) * DAY;
        Some(Self {
            tomorrow: today + DAY,
            today,
            yesterday: today - DAY,
            this_week: today - weekday * DAY,
            last_week: today - (weekday + 7) * DAY,
            this_month: month_start(0)?,
            last_month: month_start(1)?,
            this_year: year_start,
        })
    }

    /// The clock of the present moment.
    pub(crate) fn now() -> Option<Self> {
        Self::at(&glib::DateTime::now_local().ok()?)
    }

    /// The period of `seconds`: its title and the rank that orders the
    /// periods oldest first.
    fn period(&self, seconds: Option<u64>) -> Group {
        let Some(time) = seconds.and_then(|seconds| i64::try_from(seconds).ok()) else {
            return Group::numbered("Unknown date", i64::MIN);
        };
        let periods = [
            (self.tomorrow, "In the future"),
            (self.today, "Today"),
            (self.yesterday, "Yesterday"),
            (self.this_week, "Earlier this week"),
            (self.last_week, "Last week"),
            (self.this_month, "Earlier this month"),
            (self.last_month, "Last month"),
            (self.this_year, "Earlier this year"),
        ];
        if let Some((start, title)) = periods.into_iter().find(|(start, _)| time >= *start) {
            return Group::numbered(title, start);
        }
        let year = glib::DateTime::from_unix_local(time).map_or(0, |date| date.year());
        Group::numbered(&year.to_string(), i64::from(year) - i64::from(i32::MAX))
    }
}

/// One group of the listing: its title, and its rank among the groups.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Group {
    /// What the group's header says.
    pub title: String,
    rank: i64,
    text: Option<SortKey>,
}

impl Group {
    fn numbered(title: &str, rank: i64) -> Self {
        Self {
            title: title.to_owned(),
            rank,
            text: None,
        }
    }

    /// A group of `text`, ordered by it in natural order after the groups
    /// ranked below `rank`.
    fn texted(title: &str, rank: i64) -> Self {
        Self {
            title: title.to_owned(),
            rank,
            text: Some(SortKey::new(title)),
        }
    }

    /// Orders two groups as their items sort ascending.
    pub(crate) fn compare(&self, other: &Group) -> Ordering {
        let by_text = match (&self.text, &other.text) {
            (Some(a), Some(b)) => a.natural_cmp(b),
            _ => Ordering::Equal,
        };
        self.rank.cmp(&other.rank).then(by_text)
    }
}

/// The group `item` falls in when the listing is sorted by `by`.
pub(crate) fn group_of(by: SortBy, item: &FileItem, clock: &GroupClock) -> Group {
    let entry = item.entry();
    let text_or = |text: Option<&str>, missing: &str| match text {
        Some(text) => Group::texted(text, 1),
        None => Group::numbered(missing, 0),
    };
    match by {
        SortBy::Column(SortColumn::Name) => name_group(item),
        SortBy::Column(SortColumn::Size) => size_group(item),
        SortBy::Column(SortColumn::Modified) => clock.period(entry.modified),
        SortBy::Column(SortColumn::Type) => Group::texted(&entry.type_label, 1),
        SortBy::Column(SortColumn::FolderPath) => Group::texted(&item.folder_path().text, 1),
        SortBy::Column(SortColumn::OriginalLocation) => Group::texted(&item.original_location().text, 1),
        SortBy::Column(SortColumn::Deleted) => clock.period(entry.trash_deletion_date),
        SortBy::Column(SortColumn::Created) => group_of(SortBy::Role(SortRole::Created), item, clock),
        SortBy::Column(SortColumn::Extension) => group_of(SortBy::Role(SortRole::Extension), item, clock),
        SortBy::Column(SortColumn::Owner) => group_of(SortBy::Role(SortRole::Owner), item, clock),
        SortBy::Column(SortColumn::Permissions) => group_of(SortBy::Role(SortRole::Permissions), item, clock),
        SortBy::Role(SortRole::Created) => clock.period(entry.meta.created),
        SortBy::Role(SortRole::Accessed) => clock.period(entry.meta.accessed),
        SortBy::Role(SortRole::Extension) => {
            let extension = extension(&entry.name, entry.is_dir).map(str::to_uppercase);
            text_or(extension.as_deref(), "No extension")
        }
        SortBy::Role(SortRole::Permissions) => {
            let permissions = entry.meta.permissions_text();
            text_or(
                Some(&permissions)
                    .filter(|text| !text.is_empty())
                    .map(String::as_str),
                "Unknown",
            )
        }
        SortBy::Role(SortRole::Owner) => text_or(entry.meta.owner.as_deref(), "Unknown"),
        SortBy::Role(SortRole::Group) => text_or(entry.meta.group.as_deref(), "Unknown"),
        SortBy::Role(SortRole::LinkTarget) => text_or(entry.meta.link_target.as_deref(), "Not a link"),
    }
}

/// The first letter of the name, "0 – 9" for a digit, "#" for anything
/// else, in that order (natural order puts punctuation first).
fn name_group(item: &FileItem) -> Group {
    let first = item.lowercase_name().chars().next().unwrap_or(' ');
    if first.is_numeric() {
        Group::numbered("0 – 9", 1)
    } else if first.is_alphabetic() {
        let letter: String = first.to_uppercase().collect();
        Group::texted(&letter, 2)
    } else {
        Group::numbered("#", 0)
    }
}

/// Folders that were not measured first, then Explorer's size buckets.
fn size_group(item: &FileItem) -> Group {
    if item.entry().is_dir && item.folder_size().is_none() {
        return Group::numbered("Folders", -1);
    }
    let size = item.sort_size();
    if size == 0 {
        return Group::numbered("Empty (0 KB)", 0);
    }
    let bucket = SIZE_GROUPS
        .iter()
        .position(|(_, below)| size < *below)
        .unwrap_or(SIZE_GROUPS.len() - 1);
    Group::numbered(SIZE_GROUPS[bucket].0, i64::try_from(bucket).unwrap_or(0) + 1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::file_entry;

    fn modified_at(seconds: i64) -> FileItem {
        let mut entry = file_entry("a.txt");
        entry.modified = u64::try_from(seconds).ok();
        FileItem::new(entry)
    }

    /// Names group by first letter, sizes by Explorer's buckets and dates
    /// by period, each in the order their items sort.
    ///
    /// parity: VIEW-022
    #[gtk::test]
    fn items_group_by_letter_size_and_period() {
        let utc = glib::TimeZone::utc();
        let now = glib::DateTime::new(&utc, 2026, 9, 30, 15, 0, 0.0).expect("a date");
        let clock = GroupClock::at(&now).expect("a clock");
        let by_name = SortBy::Column(SortColumn::Name);
        let group = |name: &str| group_of(by_name, &FileItem::new(file_entry(name)), &clock).title;
        assert_eq!(
            [group("apple"), group("Égal"), group("7 days"), group("_x")],
            ["A", "É", "0 – 9", "#"]
        );

        let mut big = file_entry("big.iso");
        big.size = Some(2 * 1024 * 1024 * 1024);
        let big = group_of(SortBy::Column(SortColumn::Size), &FileItem::new(big), &clock);
        assert_eq!(big.title, "Huge (1 – 4 GB)");

        let by_date = SortBy::Column(SortColumn::Modified);
        let today = now.to_unix();
        let titles: Vec<String> = [today - 3600, today - DAY, today - 100 * DAY, today - 400 * DAY]
            .map(|time| group_of(by_date, &modified_at(time), &clock).title)
            .into();
        assert_eq!(titles, ["Today", "Yesterday", "Earlier this year", "2025"]);
        let older = group_of(by_date, &modified_at(today - 400 * DAY), &clock);
        let newer = group_of(by_date, &modified_at(today), &clock);
        assert_eq!(older.compare(&newer), Ordering::Less);
    }
}
