// SPDX-License-Identifier: AGPL-3.0-only
//! Explorer's "Group by": what a folder is grouped by, and the groups of
//! names and dates as Windows Explorer's details view shows them.
//!
//! New in the native app (VIEW-022). The grouping is chosen apart from
//! the sort, as in Explorer: grouped by date modified, a folder can be
//! sorted by name within each group. [`GroupBy::SortKey`] groups by the
//! sort key instead, as Dolphin's "Show in groups" does, with the groups
//! of `ox-app`'s `folder_view::groups`.
//!
//! - **Date modified** and **Date created** use calendar periods in local
//!   time, not counts of days: Today, Yesterday, Earlier this week, Last
//!   week, Earlier this month, Last month, Earlier this year and A long
//!   time ago, and the mirror image for dates after today (Tomorrow, Later
//!   this week, Next week, Later this month, Next month, Later this year,
//!   In the future), which files copied from another time zone or a fast
//!   clock have. A date belongs to the first group that holds it, in that
//!   order, so the day before the first day of the week is Yesterday, not
//!   Last week. Weeks start on the region's first day
//!   ([`week_start_for_locale`]).
//! - **Name** uses Explorer's letter ranges: 0 - 9, A - H, I - P, Q - Z
//!   and Other.
//!
//! Size and type groups are the same either way and live with the
//! app's other groups. Nothing here depends on GTK.

use crate::i18n::message_id;

/// What a folder is grouped by.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum GroupBy {
    /// No groups.
    #[default]
    None,
    /// Letter ranges of the name.
    Name,
    /// Calendar periods of the modification date.
    Modified,
    /// The type name.
    Type,
    /// Size buckets.
    Size,
    /// Calendar periods of the creation date.
    Created,
    /// The key the folder is sorted by, in that key's own groups
    /// (Dolphin's "Show in groups").
    SortKey,
}

impl GroupBy {
    /// Every choice, in the Group by menu's order.
    pub const ALL: [GroupBy; 7] = [
        GroupBy::Name,
        GroupBy::Modified,
        GroupBy::Type,
        GroupBy::Size,
        GroupBy::Created,
        GroupBy::SortKey,
        GroupBy::None,
    ];

    /// The key stored in the settings and used as the action target.
    pub fn as_str(self) -> &'static str {
        match self {
            GroupBy::None => "none",
            GroupBy::Name => "name",
            GroupBy::Modified => "modified",
            GroupBy::Type => "type",
            GroupBy::Size => "size",
            GroupBy::Created => "created",
            GroupBy::SortKey => "sort",
        }
    }

    /// The choice for a key, if it is one.
    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|group_by| group_by.as_str() == key)
    }

    /// The Group by menu's label, a message id to translate.
    pub fn label(self) -> &'static str {
        match self {
            GroupBy::None => message_id("(None)"),
            GroupBy::Name => message_id("Name"),
            GroupBy::Modified => message_id("Date modified"),
            GroupBy::Type => message_id("Type"),
            GroupBy::Size => message_id("Size"),
            GroupBy::Created => message_id("Date created"),
            GroupBy::SortKey => message_id("Same as sort"),
        }
    }

    /// True when the folder is shown in groups at all.
    pub fn is_grouped(self) -> bool {
        self != GroupBy::None
    }
}

impl serde::Serialize for GroupBy {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

/// A day of the proleptic Gregorian calendar.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CivilDate {
    /// The year, such as 2026.
    pub year: i32,
    /// The month, 1 to 12.
    pub month: u32,
    /// The day of the month, 1 to 31.
    pub day: u32,
}

impl CivilDate {
    /// The date `year-month-day`, if it exists.
    pub fn new(year: i32, month: u32, day: u32) -> Option<Self> {
        let date = Self { year, month, day };
        ((1..=12).contains(&month) && day >= 1 && day <= days_in_month(year, month)).then_some(date)
    }

    /// The local date of a Unix time, in the process's time zone.
    pub fn from_unix_local(seconds: i64) -> Option<Self> {
        let date_time = glib::DateTime::from_unix_local(seconds).ok()?;
        Self::new(
            date_time.year(),
            u32::try_from(date_time.month()).ok()?,
            u32::try_from(date_time.day_of_month()).ok()?,
        )
    }

    /// Today's local date.
    pub fn today() -> Option<Self> {
        let now = glib::DateTime::now_local().ok()?;
        Self::new(
            now.year(),
            u32::try_from(now.month()).ok()?,
            u32::try_from(now.day_of_month()).ok()?,
        )
    }

    /// Days since 1970-01-01 (negative before), by Howard Hinnant's
    /// `days_from_civil`.
    pub fn day_number(self) -> i64 {
        let year = i64::from(self.year) - i64::from(self.month <= 2);
        let era = year.div_euclid(400);
        let year_of_era = year - era * 400;
        let month = i64::from(self.month);
        let day_of_year =
            (153 * (if month > 2 { month - 3 } else { month + 9 }) + 2) / 5 + i64::from(self.day) - 1;
        let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
        era * 146_097 + day_of_era - 719_468
    }

    /// The date `days` days after 1970-01-01, by Howard Hinnant's
    /// `civil_from_days`; the inverse of [`CivilDate::day_number`].
    pub fn from_day_number(days: i64) -> Self {
        let days = days + 719_468;
        let era = days.div_euclid(146_097);
        let day_of_era = days - era * 146_097;
        let year_of_era = (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
        let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
        let shifted_month = (5 * day_of_year + 2) / 153;
        let day = day_of_year - (153 * shifted_month + 2) / 5 + 1;
        let month = if shifted_month < 10 {
            shifted_month + 3
        } else {
            shifted_month - 9
        };
        let year = year_of_era + era * 400 + i64::from(month <= 2);
        Self {
            year: i32::try_from(year).unwrap_or(if year < 0 { i32::MIN } else { i32::MAX }),
            // Both are in range by construction (1 to 12 and 1 to 31).
            month: u32::try_from(month).unwrap_or(1),
            day: u32::try_from(day).unwrap_or(1),
        }
    }

    /// The Unix time of this date's first moment in the process's time
    /// zone, or `None` outside the years `GLib` represents.
    pub fn local_midnight(self) -> Option<i64> {
        let month = i32::try_from(self.month).ok()?;
        let day = i32::try_from(self.day).ok()?;
        let midnight = glib::DateTime::from_local(self.year, month, day, 0, 0, 0.0).ok()?;
        Some(midnight.to_unix())
    }

    /// The day of the week, 0 for Monday to 6 for Sunday.
    pub fn weekday(self) -> u32 {
        // 1970-01-01 was a Thursday (3).
        // A remainder of 7 is 0 to 6, so the conversion cannot fail.
        u32::try_from((self.day_number() + 3).rem_euclid(7)).unwrap_or(0)
    }

    /// The month before this date's month, as `(year, month)`.
    fn previous_month(self) -> (i32, u32) {
        if self.month == 1 {
            (self.year - 1, 12)
        } else {
            (self.year, self.month - 1)
        }
    }

    /// The month after this date's month, as `(year, month)`.
    fn next_month(self) -> (i32, u32) {
        if self.month == 12 {
            (self.year + 1, 1)
        } else {
            (self.year, self.month + 1)
        }
    }
}

/// The number of days in `month` of `year`.
fn days_in_month(year: i32, month: u32) -> u32 {
    match month {
        4 | 6 | 9 | 11 => 30,
        2 if (year % 4 == 0 && year % 100 != 0) || year % 400 == 0 => 29,
        2 => 28,
        _ => 31,
    }
}

/// The first day of the week, 0 for Monday to 6 for Sunday.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct WeekStart(pub u32);

impl WeekStart {
    /// Monday, the default of most regions and of ISO 8601.
    pub const MONDAY: WeekStart = WeekStart(0);
    /// Saturday.
    pub const SATURDAY: WeekStart = WeekStart(5);
    /// Friday, as in the Maldives.
    pub const FRIDAY: WeekStart = WeekStart(4);
    /// Sunday.
    pub const SUNDAY: WeekStart = WeekStart(6);
}

/// The regions whose week starts on Sunday, from CLDR's
/// `supplementalData.xml` (`weekData`, `firstDay day="sun"`).
const SUNDAY_REGIONS: [&str; 56] = [
    "AG", "AS", "BD", "BR", "BS", "BT", "BW", "BZ", "CA", "CO", "DM", "DO", "ET", "GT", "GU", "HK", "HN",
    "ID", "IL", "IN", "IS", "JM", "JP", "KE", "KH", "KR", "LA", "MH", "MM", "MO", "MT", "MX", "MZ", "NI",
    "NP", "PA", "PE", "PH", "PK", "PR", "PT", "PY", "SA", "SG", "SV", "TH", "TT", "TW", "UM", "US", "VE",
    "VI", "WS", "YE", "ZA", "ZW",
];

/// The regions whose week starts on Saturday (CLDR `firstDay day="sat"`).
const SATURDAY_REGIONS: [&str; 14] = [
    "AF", "BH", "DJ", "DZ", "EG", "IQ", "IR", "JO", "KW", "LY", "OM", "QA", "SD", "SY",
];

/// The regions whose week starts on Friday (CLDR `firstDay day="fri"`).
const FRIDAY_REGIONS: [&str; 1] = ["MV"];

/// The first day of the week for a locale name such as `en_US.UTF-8`:
/// Sunday, Saturday or Friday for the regions CLDR lists, Monday otherwise
/// (including `C` and `POSIX`).
///
/// `GLib` has no first-weekday call, and `nl_langinfo(_NL_TIME_FIRST_WEEKDAY)`
/// would need `unsafe`, so the region decides, as CLDR does.
pub fn week_start_for_locale(locale: &str) -> WeekStart {
    let name = locale.split(['.', '@']).next().unwrap_or_default();
    let Some((_, region)) = name.split_once('_') else {
        return WeekStart::MONDAY;
    };
    if SUNDAY_REGIONS.contains(&region) {
        WeekStart::SUNDAY
    } else if SATURDAY_REGIONS.contains(&region) {
        WeekStart::SATURDAY
    } else if FRIDAY_REGIONS.contains(&region) {
        WeekStart::FRIDAY
    } else {
        WeekStart::MONDAY
    }
}

/// The week start of the process's time locale: `LC_ALL`, else
/// `LC_TIME`, else `LANG`, as the C library chooses it.
pub fn week_start_from_environment() -> WeekStart {
    let locale = ["LC_ALL", "LC_TIME", "LANG"]
        .into_iter()
        .filter_map(|variable| std::env::var(variable).ok())
        .find(|value| !value.is_empty())
        .unwrap_or_default();
    week_start_for_locale(&locale)
}

/// A date group, in Explorer's order from the furthest future to the
/// oldest past.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum DateGroup {
    /// After this year.
    InTheFuture,
    /// After next month, this year.
    LaterThisYear,
    /// The whole next month.
    NextMonth,
    /// After next week, this month.
    LaterThisMonth,
    /// The whole next week.
    NextWeek,
    /// After tomorrow, this week.
    LaterThisWeek,
    /// The next day.
    Tomorrow,
    /// Today.
    Today,
    /// The day before.
    Yesterday,
    /// From the start of this week until the day before yesterday.
    EarlierThisWeek,
    /// The whole previous week.
    LastWeek,
    /// This month, before last week.
    EarlierThisMonth,
    /// The whole previous month.
    LastMonth,
    /// This year, before last month.
    EarlierThisYear,
    /// Before this year.
    LongTimeAgo,
    /// No modification date is known.
    Unknown,
}

impl DateGroup {
    /// The group's heading, a message id to translate.
    pub fn label(self) -> &'static str {
        match self {
            DateGroup::InTheFuture => message_id("In the future"),
            DateGroup::LaterThisYear => message_id("Later this year"),
            DateGroup::NextMonth => message_id("Next month"),
            DateGroup::LaterThisMonth => message_id("Later this month"),
            DateGroup::NextWeek => message_id("Next week"),
            DateGroup::LaterThisWeek => message_id("Later this week"),
            DateGroup::Tomorrow => message_id("Tomorrow"),
            DateGroup::Today => message_id("Today"),
            DateGroup::Yesterday => message_id("Yesterday"),
            DateGroup::EarlierThisWeek => message_id("Earlier this week"),
            DateGroup::LastWeek => message_id("Last week"),
            DateGroup::EarlierThisMonth => message_id("Earlier this month"),
            DateGroup::LastMonth => message_id("Last month"),
            DateGroup::EarlierThisYear => message_id("Earlier this year"),
            DateGroup::LongTimeAgo => message_id("A long time ago"),
            DateGroup::Unknown => message_id("Unknown date"),
        }
    }
}

/// Today and the week start, which every date group depends on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Calendar {
    /// The local date the groups count from.
    pub today: CivilDate,
    /// The first day of the week.
    pub week_start: WeekStart,
}

impl Calendar {
    /// Today in the process's time zone and locale. `None` only if the
    /// clock cannot be read.
    pub fn now() -> Option<Self> {
        Some(Self {
            today: CivilDate::today()?,
            week_start: week_start_from_environment(),
        })
    }

    /// The day number of the first day of today's week.
    fn week_start_day(self) -> i64 {
        let offset = (self.today.weekday() + 7 - self.week_start.0) % 7;
        self.today.day_number() - i64::from(offset)
    }

    /// The days on which the date group can change, as day numbers,
    /// ascending: between two of them every date is in the same group.
    fn boundary_days(self) -> Vec<i64> {
        let today = self.today.day_number();
        let week = self.week_start_day();
        let month_start =
            |(year, month): (i32, u32)| CivilDate::new(year, month, 1).map_or(today, CivilDate::day_number);
        let this_month = (self.today.year, self.today.month);
        let next_month = self.today.next_month();
        let month_after = CivilDate {
            year: next_month.0,
            month: next_month.1,
            day: 1,
        }
        .next_month();
        let mut days = vec![
            today - 1,
            today,
            today + 1,
            today + 2,
            week - 7,
            week,
            week + 7,
            week + 14,
            month_start(self.today.previous_month()),
            month_start(this_month),
            month_start(next_month),
            month_start(month_after),
            month_start((self.today.year, 1)),
            month_start((self.today.year.saturating_add(1), 1)),
        ];
        days.sort_unstable();
        days.dedup();
        days
    }

    /// The date groups as ranges of Unix time, for grouping many items
    /// quickly: one search per item instead of a date conversion.
    pub fn date_ranges(self) -> DateRanges {
        let days = self.boundary_days();
        let first = days.first().copied().unwrap_or_default();
        let before = self.group_of(CivilDate::from_day_number(first - 1));
        let mut ranges = DateRanges {
            before,
            starts: [(0, DateGroup::Unknown); MAX_RANGES],
            count: 0,
        };
        for day in days {
            let date = CivilDate::from_day_number(day);
            let group = self.group_of(date);
            let Some(start) = date.local_midnight() else {
                continue;
            };
            let previous = ranges.starts[..ranges.count]
                .last()
                .map_or(before, |&(_, group)| group);
            if group != previous && ranges.count < MAX_RANGES {
                ranges.starts[ranges.count] = (start, group);
                ranges.count += 1;
            }
        }
        ranges
    }

    /// The group of a modification time, in Unix seconds.
    pub fn group_of_time(self, modified: Option<u64>) -> DateGroup {
        modified
            .and_then(|seconds| i64::try_from(seconds).ok())
            .and_then(CivilDate::from_unix_local)
            .map_or(DateGroup::Unknown, |date| self.group_of(date))
    }

    /// The group of `date`.
    pub fn group_of(self, date: CivilDate) -> DateGroup {
        let today = self.today;
        let day = date.day_number();
        let offset = day - today.day_number();
        let week = self.week_start_day();
        let month = (date.year, date.month);
        match offset {
            0 => DateGroup::Today,
            -1 => DateGroup::Yesterday,
            1 => DateGroup::Tomorrow,
            _ if offset < 0 => {
                if day >= week {
                    DateGroup::EarlierThisWeek
                } else if day >= week - 7 {
                    DateGroup::LastWeek
                } else if month == (today.year, today.month) {
                    DateGroup::EarlierThisMonth
                } else if month == today.previous_month() {
                    DateGroup::LastMonth
                } else if date.year == today.year {
                    DateGroup::EarlierThisYear
                } else {
                    DateGroup::LongTimeAgo
                }
            }
            _ => {
                let next_week = week + 7;
                if day < next_week {
                    DateGroup::LaterThisWeek
                } else if day < next_week + 7 {
                    DateGroup::NextWeek
                } else if month == (today.year, today.month) {
                    DateGroup::LaterThisMonth
                } else if month == today.next_month() {
                    DateGroup::NextMonth
                } else if date.year == today.year {
                    DateGroup::LaterThisYear
                } else {
                    DateGroup::InTheFuture
                }
            }
        }
    }
}

/// The most date groups that can start after the first: one per
/// boundary day of [`Calendar::boundary_days`].
const MAX_RANGES: usize = 14;

/// The date groups of one [`Calendar`] as ranges of Unix time. It is
/// `Copy`, so a sorter can hold it without sharing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DateRanges {
    /// The group of every time before the first range.
    before: DateGroup,
    /// Where each later group starts, ascending; the first `count` hold.
    starts: [(i64, DateGroup); MAX_RANGES],
    /// How many of `starts` hold.
    count: usize,
}

impl DateRanges {
    /// The group of a time, in Unix seconds; the same as
    /// [`Calendar::group_of_time`].
    pub fn group_of_time(&self, time: Option<u64>) -> DateGroup {
        let Some(seconds) = time.and_then(|seconds| i64::try_from(seconds).ok()) else {
            return DateGroup::Unknown;
        };
        let starts = &self.starts[..self.count];
        let later = starts.partition_point(|&(start, _)| start <= seconds);
        later.checked_sub(1).map_or(self.before, |index| starts[index].1)
    }
}

/// A letter range of names.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum NameGroup {
    /// Names that start with a digit.
    Digits,
    /// A to H.
    AToH,
    /// I to P.
    IToP,
    /// Q to Z.
    QToZ,
    /// Anything else: symbols, other scripts.
    Other,
}

impl NameGroup {
    /// The group of `name`, by its first letter or digit, ignoring case
    /// and the accents of Latin letters.
    pub fn of(name: &str) -> Self {
        let Some(first) = name.chars().find(|c| c.is_alphanumeric()) else {
            return NameGroup::Other;
        };
        let letter = base_latin_letter(first);
        match letter {
            '0'..='9' => NameGroup::Digits,
            'a'..='h' => NameGroup::AToH,
            'i'..='p' => NameGroup::IToP,
            'q'..='z' => NameGroup::QToZ,
            _ => NameGroup::Other,
        }
    }

    /// The group's heading, a message id to translate.
    pub fn label(self) -> &'static str {
        match self {
            NameGroup::Digits => message_id("0 – 9"),
            NameGroup::AToH => message_id("A – H"),
            NameGroup::IToP => message_id("I – P"),
            NameGroup::QToZ => message_id("Q – Z"),
            NameGroup::Other => message_id("Other"),
        }
    }
}

/// `c` in lower case without the accent of a common Latin letter, so
/// "Élan" groups with E; other characters are only lower-cased.
fn base_latin_letter(c: char) -> char {
    let lower = c.to_lowercase().next().unwrap_or(c);
    match lower {
        'à'..='å' | 'ā' | 'ă' | 'ą' => 'a',
        'ç' | 'ć' | 'č' => 'c',
        'ď' | 'đ' => 'd',
        'è'..='ë' | 'ē' | 'ė' | 'ę' | 'ě' => 'e',
        'ğ' => 'g',
        'ì'..='ï' | 'ī' | 'į' | 'ı' => 'i',
        'ł' | 'ľ' => 'l',
        'ñ' | 'ń' | 'ň' => 'n',
        'ò'..='ö' | 'ø' | 'ō' | 'ő' => 'o',
        'ř' => 'r',
        'ś' | 'š' | 'ş' => 's',
        'ť' | 'ţ' => 't',
        'ù'..='ü' | 'ū' | 'ů' | 'ű' => 'u',
        'ý' | 'ÿ' => 'y',
        'ź' | 'ż' | 'ž' => 'z',
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn date(year: i32, month: u32, day: u32) -> CivilDate {
        CivilDate::new(year, month, day).expect("a real date")
    }

    /// Thursday 1 October 2026, weeks starting on Sunday (US).
    fn us_calendar() -> Calendar {
        Calendar {
            today: date(2026, 10, 1),
            week_start: WeekStart::SUNDAY,
        }
    }

    /// parity: VIEW-022
    #[test]
    fn day_numbers_convert_back_to_dates() {
        for day in -800_000..800_000 {
            assert_eq!(CivilDate::from_day_number(day).day_number(), day);
        }
        assert_eq!(CivilDate::from_day_number(0), date(1970, 1, 1));
        assert_eq!(CivilDate::from_day_number(-1), date(1969, 12, 31));
        assert_eq!(CivilDate::from_day_number(20_727), date(2026, 10, 1));
    }

    /// Between two boundary days the group never changes, so the time
    /// ranges give every day the group its date has.
    ///
    /// parity: VIEW-022
    #[test]
    fn groups_change_only_on_boundary_days() {
        for today in [
            date(2026, 1, 1),
            date(2026, 1, 3),
            date(2026, 2, 28),
            date(2026, 10, 1),
            date(2026, 12, 27),
            date(2026, 12, 31),
            date(2028, 2, 29),
        ] {
            for week_start in [
                WeekStart::MONDAY,
                WeekStart::FRIDAY,
                WeekStart::SATURDAY,
                WeekStart::SUNDAY,
            ] {
                let calendar = Calendar { today, week_start };
                let boundaries = calendar.boundary_days();
                let first = boundaries[0];
                let before = calendar.group_of(CivilDate::from_day_number(first - 1));
                for day in today.day_number() - 800..today.day_number() + 800 {
                    let start = boundaries.iter().rev().find(|&&boundary| boundary <= day);
                    let expected = start.map_or(before, |&start| {
                        calendar.group_of(CivilDate::from_day_number(start))
                    });
                    assert_eq!(
                        calendar.group_of(CivilDate::from_day_number(day)),
                        expected,
                        "{today:?} {week_start:?} day {day}"
                    );
                }
            }
        }
    }

    /// The Unix-time ranges agree with converting each time to a date.
    ///
    /// parity: VIEW-022
    #[test]
    fn time_ranges_agree_with_dates() {
        let calendar = Calendar::now().expect("the clock is readable");
        let ranges = calendar.date_ranges();
        let today = calendar.today.local_midnight().expect("today has a midnight");
        let mut seconds = today - 500 * 86_400;
        while seconds < today + 500 * 86_400 {
            let time = u64::try_from(seconds).ok();
            assert_eq!(
                ranges.group_of_time(time),
                calendar.group_of_time(time),
                "{seconds}"
            );
            seconds += 3_517;
        }
        assert_eq!(ranges.group_of_time(None), DateGroup::Unknown);
        assert_eq!(ranges.group_of_time(Some(0)), DateGroup::LongTimeAgo);
    }

    /// parity: VIEW-022
    #[test]
    fn day_numbers_and_weekdays_follow_the_calendar() {
        assert_eq!(date(1970, 1, 1).day_number(), 0);
        assert_eq!(date(2000, 3, 1).day_number(), 11_017);
        assert_eq!(date(1969, 12, 31).day_number(), -1);
        assert_eq!(date(2026, 10, 1).weekday(), 3, "a Thursday");
        assert_eq!(date(2024, 2, 29).weekday(), 3);
        assert!(CivilDate::new(2026, 2, 29).is_none());
        assert!(CivilDate::new(2024, 2, 29).is_some());
    }

    /// The past, in the order Explorer checks it.
    ///
    /// parity: VIEW-022
    #[test]
    fn past_dates_fall_in_calendar_periods() {
        let calendar = us_calendar();
        // The week of Thursday 1 October 2026 starts on Sunday 27 September.
        assert_eq!(calendar.group_of(date(2026, 10, 1)), DateGroup::Today);
        assert_eq!(calendar.group_of(date(2026, 9, 30)), DateGroup::Yesterday);
        assert_eq!(calendar.group_of(date(2026, 9, 29)), DateGroup::EarlierThisWeek);
        assert_eq!(calendar.group_of(date(2026, 9, 27)), DateGroup::EarlierThisWeek);
        assert_eq!(calendar.group_of(date(2026, 9, 26)), DateGroup::LastWeek);
        assert_eq!(calendar.group_of(date(2026, 9, 20)), DateGroup::LastWeek);
        // Last week reached into September, so the rest of it is Last month.
        assert_eq!(calendar.group_of(date(2026, 9, 19)), DateGroup::LastMonth);
        assert_eq!(calendar.group_of(date(2026, 9, 1)), DateGroup::LastMonth);
        assert_eq!(calendar.group_of(date(2026, 8, 31)), DateGroup::EarlierThisYear);
        assert_eq!(calendar.group_of(date(2026, 1, 1)), DateGroup::EarlierThisYear);
        assert_eq!(calendar.group_of(date(2025, 12, 31)), DateGroup::LongTimeAgo);
    }

    /// The future mirrors the past.
    ///
    /// parity: VIEW-022
    #[test]
    fn future_dates_fall_in_calendar_periods() {
        let calendar = us_calendar();
        assert_eq!(calendar.group_of(date(2026, 10, 2)), DateGroup::Tomorrow);
        assert_eq!(calendar.group_of(date(2026, 10, 3)), DateGroup::LaterThisWeek);
        assert_eq!(calendar.group_of(date(2026, 10, 4)), DateGroup::NextWeek);
        assert_eq!(calendar.group_of(date(2026, 10, 10)), DateGroup::NextWeek);
        assert_eq!(calendar.group_of(date(2026, 10, 11)), DateGroup::LaterThisMonth);
        assert_eq!(calendar.group_of(date(2026, 10, 31)), DateGroup::LaterThisMonth);
        assert_eq!(calendar.group_of(date(2026, 11, 1)), DateGroup::NextMonth);
        assert_eq!(calendar.group_of(date(2026, 12, 1)), DateGroup::LaterThisYear);
        assert_eq!(calendar.group_of(date(2027, 1, 1)), DateGroup::InTheFuture);
    }

    /// Yesterday and Tomorrow win over the week boundaries; a new year
    /// makes last December "Last month", not "A long time ago".
    ///
    /// parity: VIEW-022
    #[test]
    fn boundaries_take_the_first_group_that_holds_them() {
        // Sunday 4 October 2026 starts a US week: Saturday is Yesterday.
        let sunday = Calendar {
            today: date(2026, 10, 4),
            week_start: WeekStart::SUNDAY,
        };
        assert_eq!(sunday.group_of(date(2026, 10, 3)), DateGroup::Yesterday);
        assert_eq!(sunday.group_of(date(2026, 10, 2)), DateGroup::LastWeek);
        // Saturday 3 October 2026 ends it: Sunday is Tomorrow.
        let saturday = Calendar {
            today: date(2026, 10, 3),
            week_start: WeekStart::SUNDAY,
        };
        assert_eq!(saturday.group_of(date(2026, 10, 4)), DateGroup::Tomorrow);
        assert_eq!(saturday.group_of(date(2026, 10, 5)), DateGroup::NextWeek);
        // 20 January 2027: December 2026 is last month, November 2026 long ago.
        let january = Calendar {
            today: date(2027, 1, 20),
            week_start: WeekStart::MONDAY,
        };
        assert_eq!(january.group_of(date(2026, 12, 15)), DateGroup::LastMonth);
        assert_eq!(january.group_of(date(2026, 11, 15)), DateGroup::LongTimeAgo);
        assert_eq!(january.group_of(date(2027, 1, 2)), DateGroup::EarlierThisMonth);
        // 20 December 2026: January 2027 is next month, February next year.
        let december = Calendar {
            today: date(2026, 12, 20),
            week_start: WeekStart::MONDAY,
        };
        assert_eq!(december.group_of(date(2027, 1, 15)), DateGroup::NextMonth);
        assert_eq!(december.group_of(date(2027, 2, 15)), DateGroup::InTheFuture);
        assert_eq!(december.group_of(date(2026, 12, 31)), DateGroup::LaterThisMonth);
    }

    /// The same day is "Earlier this week" in the US and "Last week" in
    /// Germany, where weeks start on Monday.
    ///
    /// parity: VIEW-022
    #[test]
    fn the_week_starts_on_the_regions_first_day() {
        assert_eq!(week_start_for_locale("en_US.UTF-8"), WeekStart::SUNDAY);
        assert_eq!(week_start_for_locale("de_DE.UTF-8"), WeekStart::MONDAY);
        assert_eq!(week_start_for_locale("ar_EG.UTF-8"), WeekStart::SATURDAY);
        assert_eq!(week_start_for_locale("en_GB"), WeekStart::MONDAY);
        assert_eq!(week_start_for_locale("C.UTF-8"), WeekStart::MONDAY);
        assert_eq!(week_start_for_locale("sr_RS@latin"), WeekStart::MONDAY);
        // CLDR as of 2026: Australia, China and the UAE start on Monday;
        // Zimbabwe and Iceland on Sunday, Sudan on Saturday, the Maldives
        // on Friday.
        for monday in ["en_AU.UTF-8", "zh_CN.UTF-8", "ar_AE.UTF-8"] {
            assert_eq!(week_start_for_locale(monday), WeekStart::MONDAY, "{monday}");
        }
        assert_eq!(week_start_for_locale("en_ZW.UTF-8"), WeekStart::SUNDAY);
        assert_eq!(week_start_for_locale("is_IS.UTF-8"), WeekStart::SUNDAY);
        assert_eq!(week_start_for_locale("ar_SD.UTF-8"), WeekStart::SATURDAY);
        assert_eq!(week_start_for_locale("dv_MV.UTF-8"), WeekStart::FRIDAY);
        // Monday 28 September 2026, seen from Thursday 1 October.
        let monday = date(2026, 9, 28);
        assert_eq!(us_calendar().group_of(monday), DateGroup::EarlierThisWeek);
        let germany = Calendar {
            today: date(2026, 10, 1),
            week_start: WeekStart::MONDAY,
        };
        assert_eq!(germany.group_of(monday), DateGroup::EarlierThisWeek);
        assert_eq!(germany.group_of(date(2026, 9, 27)), DateGroup::LastWeek);
        assert_eq!(
            us_calendar().group_of(date(2026, 9, 27)),
            DateGroup::EarlierThisWeek
        );
    }

    /// Groups sort newest first; an unknown date comes last.
    ///
    /// parity: VIEW-022
    #[test]
    fn date_groups_sort_from_the_future_to_the_past() {
        assert!(DateGroup::InTheFuture < DateGroup::Tomorrow);
        assert!(DateGroup::Tomorrow < DateGroup::Today);
        assert!(DateGroup::Today < DateGroup::LongTimeAgo);
        assert!(DateGroup::LongTimeAgo < DateGroup::Unknown);
        assert_eq!(us_calendar().group_of_time(None), DateGroup::Unknown);
    }

    /// parity: VIEW-022
    #[test]
    fn names_use_explorers_letter_ranges_and_the_keys_round_trip() {
        assert_eq!(NameGroup::of("Budget.xlsx"), NameGroup::AToH);
        assert_eq!(NameGroup::of("invoice"), NameGroup::IToP);
        assert_eq!(NameGroup::of("Zoo"), NameGroup::QToZ);
        assert_eq!(NameGroup::of("2026 photos"), NameGroup::Digits);
        assert_eq!(NameGroup::of("Élan"), NameGroup::AToH);
        assert_eq!(
            NameGroup::of("_draft"),
            NameGroup::AToH,
            "leading symbols are skipped"
        );
        assert_eq!(NameGroup::of("日本"), NameGroup::Other);
        assert_eq!(NameGroup::of("!!!"), NameGroup::Other);
        for group_by in GroupBy::ALL {
            assert_eq!(GroupBy::from_key(group_by.as_str()), Some(group_by));
        }
        assert_eq!(GroupBy::from_key("date"), None);
        assert_eq!(
            serde_json::to_string(&GroupBy::Modified).expect("a key"),
            "\"modified\""
        );
    }
}
