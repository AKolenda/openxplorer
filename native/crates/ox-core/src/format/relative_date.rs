// SPDX-License-Identifier: AGPL-3.0-only
//! Relative dates in the Date modified column: "Today at 3:00 PM" and
//! "Yesterday at 3:00 PM", as Dolphin shows them by default
//! (`UseShortRelativeDates`); older and future dates keep the absolute date
//! and time. Settings can turn them off for absolute dates everywhere.

use glib::DateTime;

use super::locale_pattern::{ClockFormat, LocalePatterns};
use super::without_seconds;

/// How the Date modified column writes a date.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum DateStyle {
    /// "Today at 3:00 PM", "Yesterday at 3:00 PM", else the date and time.
    #[default]
    Relative,
    /// Always the date and time.
    Absolute,
}

/// "Today at 7:05 PM" or "Yesterday at 7:05 PM" for `time` seen at `now`,
/// with the clock time on `clock` without seconds; `None` for any other
/// day.
pub(super) fn relative_text(
    time: &DateTime,
    now: &DateTime,
    patterns: &LocalePatterns,
    clock: ClockFormat,
) -> Option<String> {
    let yesterday = now.add_days(-1).ok()?;
    let word = if time.ymd() == now.ymd() {
        "Today"
    } else if time.ymd() == yesterday.ymd() {
        "Yesterday"
    } else {
        return None;
    };
    let clock = time.format(&without_seconds(&patterns.time_on(clock))).ok()?;
    Some(format!("{word} at {clock}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Today and yesterday read as words with the clock time; other days
    /// and the future keep the absolute date.
    ///
    /// parity: VIEW-004
    #[test]
    fn today_and_yesterday_read_as_words_with_the_time() {
        let us = LocalePatterns {
            date: "%m/%d/%Y".to_owned(),
            time: "%-I:%M:%S %p".to_owned(),
            has_day_period: true,
        };
        let at = |day, hour| DateTime::from_utc(2026, 9, day, hour, 5, 7.0).expect("valid date");
        let now = at(6, 20);
        let text = |time: &DateTime| relative_text(time, &now, &us, ClockFormat::Locale);
        assert_eq!(text(&at(6, 19)).as_deref(), Some("Today at 7:05 PM"));
        assert_eq!(text(&at(5, 9)).as_deref(), Some("Yesterday at 9:05 AM"));
        assert_eq!(text(&at(4, 9)), None);
        assert_eq!(text(&at(7, 9)), None, "tomorrow");
    }
}
