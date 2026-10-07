// SPDX-License-Identifier: AGPL-3.0-only
//! Groups of the listing (VIEW-022): by a key of their own, as Windows
//! Explorer's Group by, or by the sort key, as Dolphin's "Show in groups".
//!
//! A [`Grouping`] says which. Grouped by a key of its own, the listing is
//! sorted by its sort key within each group, so a folder grouped by date
//! modified can list each period by name. Names then fall in Explorer's
//! letter ranges and dates in its calendar periods, down to "A long time
//! ago" ([`ox_core::grouping`]); types and sizes in the groups below, in a
//! fixed order (dates newest first, the rest ascending).
//!
//! By the sort key, it ports the group roles of Dolphin's `KFileItemModel::groups()`
//! (`nameRoleGroups`, `sizeRoleGroups`, `timeRoleGroups`, `permissionRoleGroups`
//! and the generic role groups): a name groups by its first letter, a size by
//! Windows Explorer's size buckets, a date by period ("Today", "Yesterday",
//! "Earlier this week", …, then the year), and every other key by its text.
//! A [`Group`] orders the groups the way the key sorts its items, so the
//! folder model can sort by group first ([`super::model`]).

use std::cmp::Ordering;

use gtk::glib;
use ox_core::i18n::{gettext, gettext_static};

use ox_core::grouping::{Calendar, DateRanges, GroupBy, NameGroup};

use crate::folder_view::item::FileItem;
use crate::folder_view::sort_roles::{extension, SortBy, SortRole, SortState};
use crate::folder_view::sorting::{SortColumn, SortKey};
use crate::i18n::message_id;

/// Seconds in a day for the fixed-offset test dates.
#[cfg(test)]
const DAY: i64 = 24 * 60 * 60;

/// Explorer's size groups: the title and the size each one ends below.
const SIZE_GROUPS: [(&str, u64); 6] = [
    (message_id("Tiny (0 – 16 KB)"), 16 * 1024),
    (message_id("Small (16 KB – 1 MB)"), 1024 * 1024),
    (message_id("Medium (1 – 128 MB)"), 128 * 1024 * 1024),
    (message_id("Large (128 MB – 1 GB)"), 1024 * 1024 * 1024),
    (message_id("Huge (1 – 4 GB)"), 4 * 1024 * 1024 * 1024),
    (message_id("Gigantic (> 4 GB)"), u64::MAX),
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
        let weekday = now.day_of_week() - 1;
        let day_start = |days: i32| midnight.add_days(days).ok().map(|date| date.to_unix());
        let month_start = |months_back: i32| {
            let date = midnight.add_months(-months_back).ok()?;
            date.add_days(1 - date.day_of_month())
                .ok()
                .map(|date| date.to_unix())
        };
        Some(Self {
            tomorrow: day_start(1)?,
            today,
            yesterday: day_start(-1)?,
            this_week: day_start(-weekday)?,
            last_week: day_start(-weekday - 7)?,
            this_month: month_start(0)?,
            last_month: month_start(1)?,
            this_year: day_start(1 - now.day_of_year())?,
        })
    }

    /// The period of `seconds`: its title and the rank that orders the
    /// periods oldest first.
    fn period(&self, seconds: Option<u64>) -> Group {
        let Some(time) = seconds.and_then(|seconds| i64::try_from(seconds).ok()) else {
            return Group::numbered(&gettext("Unknown date"), i64::MIN);
        };
        let periods = [
            (self.tomorrow, gettext_static("In the future")),
            (self.today, gettext_static("Today")),
            (self.yesterday, gettext_static("Yesterday")),
            (self.this_week, gettext_static("Earlier this week")),
            (self.last_week, gettext_static("Last week")),
            (self.this_month, gettext_static("Earlier this month")),
            (self.last_month, gettext_static("Last month")),
            (self.this_year, gettext_static("Earlier this year")),
        ];
        if let Some((start, title)) = periods.into_iter().find(|(start, _)| time >= *start) {
            return Group::numbered(title, start);
        }
        let year = glib::DateTime::from_unix_local(time).map_or(0, |date| date.year());
        Group::numbered(&year.to_string(), i64::from(year) - i64::from(i32::MAX))
    }
}

/// What the listing is grouped by, and the sort it is grouped within.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Grouping {
    /// The key; never [`GroupBy::None`].
    pub by: GroupBy,
    /// The sort; the groups follow it with [`GroupBy::SortKey`].
    pub sort: SortState,
}

impl Grouping {
    /// The grouping by `by` within `sort`; `None` for no groups.
    pub(crate) fn of(by: GroupBy, sort: SortState) -> Option<Self> {
        by.is_grouped().then_some(Self { by, sort })
    }
}

/// Everything grouping by date depends on, worked out once when the
/// listing is grouped: the sort key's periods and Explorer's.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct GroupClocks {
    /// The periods of [`GroupBy::SortKey`].
    pub clock: GroupClock,
    /// Explorer's periods of [`GroupBy::Modified`] and [`GroupBy::Created`].
    pub dates: DateRanges,
}

impl GroupClocks {
    /// The clocks of the present moment.
    pub(crate) fn now() -> Option<Self> {
        let now = local_now()?;
        let today = ox_core::grouping::CivilDate::new(
            now.year(),
            u32::try_from(now.month()).ok()?,
            u32::try_from(now.day_of_month()).ok()?,
        )?;
        let calendar = Calendar {
            today,
            ..Calendar::now()?
        };
        Some(Self {
            clock: GroupClock::at(&now)?,
            dates: calendar.date_ranges(),
        })
    }
}

#[cfg(test)]
thread_local! {
    /// The time tests pretend it is, `None` for the real time.
    static TEST_NOW: std::cell::RefCell<Option<glib::DateTime>> = const { std::cell::RefCell::new(None) };
}

/// Makes the groups count from `now` instead of the real time, or from the
/// real time again with `None`, for tests.
#[cfg(test)]
pub(crate) fn set_clock_for_tests(now: Option<glib::DateTime>) {
    TEST_NOW.with(|shown| shown.replace(now));
}

/// The local time the groups count from.
fn local_now() -> Option<glib::DateTime> {
    #[cfg(test)]
    if let Some(now) = TEST_NOW.with(|shown| shown.borrow().clone()) {
        return Some(now);
    }
    glib::DateTime::now_local().ok()
}

/// The group `item` falls in under `grouping`.
pub(crate) fn group_in(grouping: Grouping, item: &FileItem, clocks: &GroupClocks) -> Group {
    let entry = item.entry();
    let period = |time: Option<u64>| {
        let group = clocks.dates.group_of_time(time);
        Group::numbered(gettext_static(group.label()), group as i64)
    };
    match grouping.by {
        GroupBy::None | GroupBy::SortKey => group_of(grouping.sort.by, item, &clocks.clock),
        GroupBy::Name => {
            let group = NameGroup::of(&entry.name);
            Group::numbered(gettext_static(group.label()), group as i64)
        }
        GroupBy::Modified => period(entry.modified),
        GroupBy::Created => period(entry.meta.created),
        GroupBy::Type => Group::texted(&entry.type_label, 1),
        GroupBy::Size => size_group(item),
    }
}

/// Orders the groups of two items under `grouping`: the sort key's as its
/// sort runs, Explorer's in their own order.
pub(crate) fn compare_groups(grouping: Grouping, a: &Group, b: &Group) -> Ordering {
    let order = a.compare(b);
    match (grouping.by, grouping.sort.direction) {
        (GroupBy::None | GroupBy::SortKey, crate::folder_view::sorting::SortDirection::Descending) => {
            order.reverse()
        }
        _ => order,
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
            text_or(extension.as_deref(), &gettext("No extension"))
        }
        SortBy::Role(SortRole::Permissions) => {
            let permissions = entry.meta.permissions_text();
            text_or(
                Some(&permissions)
                    .filter(|text| !text.is_empty())
                    .map(String::as_str),
                &gettext("Unknown"),
            )
        }
        SortBy::Role(SortRole::Owner) => text_or(entry.meta.owner.as_deref(), &gettext("Unknown")),
        SortBy::Role(SortRole::Group) => text_or(entry.meta.group.as_deref(), &gettext("Unknown")),
        SortBy::Role(SortRole::LinkTarget) => {
            text_or(entry.meta.link_target.as_deref(), &gettext("Not a link"))
        }
    }
}

/// The first letter of the name, "0 – 9" for a digit, "#" for anything
/// else, in that order (natural order puts punctuation first).
fn name_group(item: &FileItem) -> Group {
    let first = item.lowercase_name().chars().next().unwrap_or(' ');
    if first.is_numeric() {
        Group::numbered(&gettext("0 – 9"), 1)
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
        return Group::numbered(&gettext("Folders"), -1);
    }
    let size = item.sort_size();
    if size == 0 {
        return Group::numbered(&gettext("Empty (0 KB)"), 0);
    }
    let bucket = SIZE_GROUPS
        .iter()
        .position(|(_, below)| size < *below)
        .unwrap_or(SIZE_GROUPS.len() - 1);
    Group::numbered(
        &gettext(SIZE_GROUPS[bucket].0),
        i64::try_from(bucket).unwrap_or(0) + 1,
    )
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

    /// Group boundaries follow calendar days through daylight-saving changes.
    ///
    /// parity: VIEW-022
    #[test]
    fn calendar_groups_follow_short_and_long_days() {
        let zone = glib::TimeZone::from_identifier(Some("America/Edmonton")).expect("a time zone");
        for (month, day) in [(3, 8), (11, 1)] {
            let now = glib::DateTime::new(&zone, 2026, month, day, 12, 0, 0.0).expect("a date");
            let clock = GroupClock::at(&now).expect("calendar boundaries");
            let late = glib::DateTime::new(&zone, 2026, month, day, 23, 30, 0.0).expect("late today");
            let tomorrow = late.add_days(1).expect("tomorrow");
            let group = |date: &glib::DateTime| clock.period(u64::try_from(date.to_unix()).ok()).title;
            assert_eq!(group(&late), "Today");
            assert_eq!(group(&tomorrow), "In the future");
        }
    }
}
