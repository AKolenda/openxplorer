// SPDX-License-Identifier: AGPL-3.0-only
//! The date a snapshot's folder name carries (PROP-020).
//!
//! Ports `parse` and the wording of `describe` in
//! `desktop/ui/snapshot-meta.js`. A date is read only from the name: a
//! Windows `@GMT-YYYY.MM.DD-HH.MM.SS` version, which is in UTC, or a
//! `YYYY-MM-DD` inside the name, optionally followed by a time whose
//! timezone the server did not say and which is never converted. The
//! localised medium date ("Sep 5, 2026") is left to the window.

/// Shown instead of a date when the snapshot's name has none.
pub const DATE_UNAVAILABLE: &str = "Date unavailable";

/// Where the date of a snapshot whose name has none would come from.
pub const NO_DATE_IN_NAME: &str = "No date in name";

/// The tooltip of [`DATE_UNAVAILABLE`].
pub const NO_DATE_EXPLANATION: &str = "This snapshot has no recognized date in its name. Folder \
                                       modification times do not establish snapshot creation time.";

/// Where the date of a snapshot whose name has one comes from.
pub const DATE_FROM_NAME: &str = "From snapshot name";

/// The start of a Windows "Previous Versions" folder name.
const SMB_VERSION_PREFIX: &str = "@GMT-";

/// The characters that may separate a date from its time: `T`, `_`, a
/// space or `-`.
const DATE_TIME_SEPARATORS: [char; 4] = ['T', '_', ' ', '-'];

/// The characters that may separate the hours, minutes and seconds.
const TIME_SEPARATORS: [char; 2] = [':', '-'];

/// The earliest and latest years a snapshot name may carry.
const YEARS: std::ops::RangeInclusive<u16> = 1970..=9999;

/// The date, and possibly the time, in a snapshot's folder name.
///
/// The snapshot folder's modification time is never used instead: it can
/// describe the live folder rather than when the snapshot was taken.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SnapshotDate {
    /// The year, 1970 to 9999.
    pub year: u16,
    /// The month, 1 to 12.
    pub month: u8,
    /// The day of the month; always a real day of that month.
    pub day: u8,
    /// The time of day, when the name has one.
    pub time: Option<SnapshotTime>,
    /// True for a Windows `@GMT-…` name, whose time is in UTC. Other
    /// names have no timezone, and none is assumed.
    pub is_utc: bool,
}

/// The time of day in a snapshot's folder name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SnapshotTime {
    /// The hour, 0 to 23.
    pub hour: u8,
    /// The minute, 0 to 59.
    pub minute: u8,
    /// The second, 0 to 59, when the name has one.
    pub second: Option<u8>,
}

impl SnapshotDate {
    /// The date in the snapshot folder name `name`, or `None` when it has
    /// none or the date is impossible, such as 29 February 2026.
    pub fn from_snapshot_name(name: &str) -> Option<Self> {
        let characters: Vec<char> = name.chars().collect();
        // Only the first date in the name counts: an impossible one is not
        // replaced by a later one, as in snapshot-meta.js.
        let fields = smb_version_fields(&characters).or_else(|| embedded_date_fields(&characters))?;
        fields.to_date()
    }

    /// The date as `YYYY-MM-DD`.
    pub fn date_text(&self) -> String {
        format!("{:04}-{:02}-{:02}", self.year, self.month, self.day)
    }

    /// The time as `HH:MM` or `HH:MM:SS`, followed by ` UTC` for a UTC
    /// name; empty when the name has no time.
    pub fn time_text(&self) -> String {
        let clock = self.clock_text();
        if self.is_utc && !clock.is_empty() {
            format!("{clock} UTC")
        } else {
            clock
        }
    }

    /// The date and time as ISO 8601, for a `<time datetime>` value: a
    /// trailing `Z` only for a UTC name, so no timezone is invented.
    pub fn iso_8601(&self) -> String {
        let date = self.date_text();
        let clock = self.clock_text();
        if clock.is_empty() {
            date
        } else if self.is_utc {
            format!("{date}T{clock}Z")
        } else {
            format!("{date}T{clock}")
        }
    }

    /// Why the date can be trusted, for its tooltip.
    pub fn explanation(&self) -> &'static str {
        if self.is_utc {
            "Date encoded in the @GMT snapshot name, shown in UTC."
        } else {
            "Date encoded in the snapshot name. Timezone was not supplied by the server; no conversion \
             has been applied."
        }
    }

    /// `HH:MM` or `HH:MM:SS`, or empty without a time.
    fn clock_text(&self) -> String {
        let Some(time) = self.time else {
            return String::new();
        };
        match time.second {
            Some(second) => format!("{:02}:{:02}:{second:02}", time.hour, time.minute),
            None => format!("{:02}:{:02}", time.hour, time.minute),
        }
    }
}

/// The numbers read from a name, before they are checked.
#[derive(Debug, Clone, Copy)]
struct DateFields {
    year: u32,
    month: u32,
    day: u32,
    time: Option<TimeFields>,
    is_utc: bool,
}

/// The time numbers read from a name, before they are checked.
#[derive(Debug, Clone, Copy)]
struct TimeFields {
    hour: u32,
    minute: u32,
    second: Option<u32>,
}

impl DateFields {
    /// The date these numbers name, or `None` when it does not exist.
    fn to_date(self) -> Option<SnapshotDate> {
        let year = u16::try_from(self.year)
            .ok()
            .filter(|year| YEARS.contains(year))?;
        let month = u8::try_from(self.month)
            .ok()
            .filter(|month| (1..=12).contains(month))?;
        let day = u8::try_from(self.day).ok()?;
        if day == 0 || day > days_in_month(year, month) {
            return None;
        }
        let time = match self.time {
            Some(time) => Some(time.to_time()?),
            None => None,
        };
        Some(SnapshotDate {
            year,
            month,
            day,
            time,
            is_utc: self.is_utc,
        })
    }
}

impl TimeFields {
    /// The time these numbers name, or `None` when it does not exist.
    fn to_time(self) -> Option<SnapshotTime> {
        let hour = u8::try_from(self.hour).ok().filter(|hour| *hour <= 23)?;
        let minute = u8::try_from(self.minute).ok().filter(|minute| *minute <= 59)?;
        let second = match self.second {
            Some(second) => Some(u8::try_from(second).ok().filter(|second| *second <= 59)?),
            None => None,
        };
        Some(SnapshotTime { hour, minute, second })
    }
}

/// The number of days in `month` of `year`, with the Gregorian leap years.
fn days_in_month(year: u16, month: u8) -> u8 {
    let is_leap_year = year.is_multiple_of(4) && (!year.is_multiple_of(100) || year.is_multiple_of(400));
    match month {
        2 if is_leap_year => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

/// `@GMT-YYYY.MM.DD-HH.MM.SS` at the start of the name, not followed by
/// another digit.
fn smb_version_fields(characters: &[char]) -> Option<DateFields> {
    let mut cursor = Cursor::new(characters, 0);
    cursor.text(SMB_VERSION_PREFIX)?;
    let year = cursor.digits(4)?;
    cursor.character('.')?;
    let month = cursor.digits(2)?;
    cursor.character('.')?;
    let day = cursor.digits(2)?;
    cursor.character('-')?;
    let hour = cursor.digits(2)?;
    cursor.character('.')?;
    let minute = cursor.digits(2)?;
    cursor.character('.')?;
    let second = Some(cursor.digits(2)?);
    let time = Some(TimeFields { hour, minute, second });
    cursor.is_at_number_end().then_some(DateFields {
        year,
        month,
        day,
        time,
        is_utc: true,
    })
}

/// The first `YYYY-MM-DD` in the name that no digit precedes or follows,
/// with the longest time that follows it.
fn embedded_date_fields(characters: &[char]) -> Option<DateFields> {
    let starts = 0..characters.len();
    starts
        .filter(|&start| start == 0 || !characters[start - 1].is_ascii_digit())
        .find_map(|start| embedded_date_at(characters, start))
}

/// A `YYYY-MM-DD` starting at `start`, with its time if one follows.
fn embedded_date_at(characters: &[char], start: usize) -> Option<DateFields> {
    let mut cursor = Cursor::new(characters, start);
    let year = cursor.digits(4)?;
    cursor.character('-')?;
    let month = cursor.digits(2)?;
    cursor.character('-')?;
    let day = cursor.digits(2)?;
    let time = time_after(cursor.clone());
    if time.is_none() && !cursor.is_at_number_end() {
        return None;
    }
    Some(DateFields {
        year,
        month,
        day,
        time,
        is_utc: false,
    })
}

/// The time after a date at `cursor`: a separator, `HH`, `MM` and maybe
/// `SS`, each optionally after `:` or `-`, and then no digit. Seconds are
/// dropped when a digit would follow them, as the JavaScript pattern
/// backtracks.
fn time_after(mut cursor: Cursor<'_>) -> Option<TimeFields> {
    cursor.one_of(&DATE_TIME_SEPARATORS)?;
    let hour = cursor.digits(2)?;
    cursor.skip_one_of(&TIME_SEPARATORS);
    let minute = cursor.digits(2)?;
    let mut with_seconds = cursor.clone();
    with_seconds.skip_one_of(&TIME_SEPARATORS);
    let second = with_seconds.digits(2);
    if second.is_some() && with_seconds.is_at_number_end() {
        return Some(TimeFields { hour, minute, second });
    }
    cursor.is_at_number_end().then_some(TimeFields {
        hour,
        minute,
        second: None,
    })
}

/// A position in a name being read.
#[derive(Debug, Clone)]
struct Cursor<'a> {
    characters: &'a [char],
    position: usize,
}

impl<'a> Cursor<'a> {
    /// A cursor at `position` in `characters`.
    fn new(characters: &'a [char], position: usize) -> Self {
        Self { characters, position }
    }

    /// The character at the cursor, without reading it.
    fn peek(&self) -> Option<char> {
        self.characters.get(self.position).copied()
    }

    /// Reads exactly `count` ASCII digits as a number.
    fn digits(&mut self, count: usize) -> Option<u32> {
        let end = self.position + count;
        let digits = self.characters.get(self.position..end)?;
        let mut number = 0;
        for digit in digits {
            // `to_digit` accepts only the ASCII digits, like \d in JavaScript.
            number = number * 10 + digit.to_digit(10)?;
        }
        self.position = end;
        Some(number)
    }

    /// Reads `expected`.
    fn character(&mut self, expected: char) -> Option<()> {
        self.one_of(&[expected]).map(|_| ())
    }

    /// Reads the characters of `expected`.
    fn text(&mut self, expected: &str) -> Option<()> {
        expected
            .chars()
            .try_for_each(|character| self.character(character))
    }

    /// Reads one of `choices`.
    fn one_of(&mut self, choices: &[char]) -> Option<char> {
        let character = self.peek().filter(|character| choices.contains(character))?;
        self.position += 1;
        Some(character)
    }

    /// Reads one of `choices` if it is next.
    fn skip_one_of(&mut self, choices: &[char]) {
        let _ = self.one_of(choices);
    }

    /// True at the end of the name or before a character that is not a
    /// digit, so a number read before it is complete.
    fn is_at_number_end(&self) -> bool {
        self.peek().is_none_or(|character| !character.is_ascii_digit())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A snapshot name and the date and time shown for it.
    struct ParsedCase {
        name: &'static str,
        date: &'static str,
        time: &'static str,
    }

    const PARSED_CASES: [ParsedCase; 8] = [
        ParsedCase {
            name: "auto-2026-09-04_16-30",
            date: "2026-09-04",
            time: "16:30",
        },
        ParsedCase {
            name: "auto-2026-09-04_16-30-21",
            date: "2026-09-04",
            time: "16:30:21",
        },
        ParsedCase {
            name: "2026-09-05_180000",
            date: "2026-09-05",
            time: "18:00:00",
        },
        ParsedCase {
            name: "2026-09-05_18:00:00",
            date: "2026-09-05",
            time: "18:00:00",
        },
        ParsedCase {
            name: "daily-2026-09-05",
            date: "2026-09-05",
            time: "",
        },
        ParsedCase {
            name: "@GMT-2026.09.05-18.00.00",
            date: "2026-09-05",
            time: "18:00:00 UTC",
        },
        ParsedCase {
            name: "2024-02-29_12-05",
            date: "2024-02-29",
            time: "12:05",
        },
        ParsedCase {
            name: "backup_2026-12-31_23-59",
            date: "2026-12-31",
            time: "23:59",
        },
    ];

    /// Ported from `desktop/tests/snapshot_meta.test.cjs` ("Parse …").
    ///
    /// parity: PROP-020
    #[test]
    fn dates_and_times_are_read_from_snapshot_names() {
        for case in PARSED_CASES {
            let date = SnapshotDate::from_snapshot_name(case.name).expect(case.name);

            assert_eq!(
                (date.date_text().as_str(), date.time_text().as_str()),
                (case.date, case.time),
                "{}",
                case.name
            );
        }
    }

    /// Ported from `desktop/tests/snapshot_meta.test.cjs` ("Reject invalid
    /// / unknown …").
    ///
    /// parity: PROP-020
    #[test]
    fn names_without_a_possible_date_have_none() {
        let names = [
            "manual",
            "1",
            "2026-02-29_12-00",
            "2026-09-31_00-00",
            "2026-13-01",
            "2026-00-01",
            "2026-09-00",
            "2026-09-04_24-01",
            "2026-09-04_12-60",
            "2026-09-04_12-01-60",
            "@GMT-2026.13.05-18.00.00",
            "<img onerror=alert(1)>",
        ];
        for name in names {
            assert_eq!(SnapshotDate::from_snapshot_name(name), None, "{name}");
        }
    }

    /// Ported from `desktop/tests/snapshot_meta.test.cjs` ("Does not invent
    /// a timezone" and "GMT has explicit timezone").
    ///
    /// parity: PROP-020
    #[test]
    fn only_a_gmt_name_has_a_timezone() {
        let local = SnapshotDate::from_snapshot_name("auto-2026-09-04_16-30").unwrap();
        let utc = SnapshotDate::from_snapshot_name("@GMT-2026.09.04-16.30.00").unwrap();

        assert_eq!(local.iso_8601(), "2026-09-04T16:30");
        assert_eq!(utc.iso_8601(), "2026-09-04T16:30:00Z");
        assert_eq!(
            local.explanation(),
            "Date encoded in the snapshot name. Timezone was not supplied by the server; no conversion \
             has been applied."
        );
        assert_eq!(
            utc.explanation(),
            "Date encoded in the @GMT snapshot name, shown in UTC."
        );
    }

    /// Ported from `desktop/tests/snapshot_meta.test.cjs` ("Name source
    /// made explicit"), with the texts `describe` shows for a name without
    /// a date.
    ///
    /// parity: PROP-020
    #[test]
    fn the_date_source_is_named_in_the_web_ui_wording() {
        assert_eq!(DATE_FROM_NAME, "From snapshot name");
        assert_eq!(NO_DATE_IN_NAME, "No date in name");
        assert_eq!(DATE_UNAVAILABLE, "Date unavailable");
        assert_eq!(
            NO_DATE_EXPLANATION,
            "This snapshot has no recognized date in its name. Folder modification times do not \
             establish snapshot creation time."
        );
    }

    /// A day-precision name has no time, in text or in ISO 8601.
    ///
    /// parity: PROP-020
    #[test]
    fn a_name_with_only_a_date_has_no_time() {
        let date = SnapshotDate::from_snapshot_name("daily-2026-09-05_18").unwrap();

        assert_eq!(date.time, None);
        assert_eq!(date.iso_8601(), "2026-09-05");
        assert_eq!(date.time_text(), "");
    }

    /// The first date in a name counts, and a date glued to other digits
    /// is not a date.
    ///
    /// parity: PROP-020
    #[test]
    fn the_first_date_not_glued_to_digits_counts() {
        let first = SnapshotDate::from_snapshot_name("from-2026-01-02-to-2026-03-04").unwrap();

        assert_eq!(first.date_text(), "2026-01-02");
        assert_eq!(SnapshotDate::from_snapshot_name("12026-09-05"), None);
        assert_eq!(SnapshotDate::from_snapshot_name("2026-09-051"), None);
    }
}
