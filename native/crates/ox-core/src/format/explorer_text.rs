// SPDX-License-Identifier: AGPL-3.0-only
//! Sizes, counts and dates as Windows 11's Properties window writes them:
//! `314 KB (322,003 bytes)`, `64 Files, 40 Folders` and
//! `Tuesday, October 20, 2026, 14:24:52`, with `Today` in place of the
//! weekday for today's date.

use glib::DateTime;

use super::locale_pattern::{ClockFormat, LocalePatterns};
use super::SIZE_UNITS;

/// The long date after the weekday, as the user's language orders it.
const LONG_DATE: &str = crate::i18n::message_id("%B %-d, %Y");

/// `314 KB (322,003 bytes)`: the short size with three significant
/// digits, truncated as Explorer truncates them, then every byte.
pub fn exact_size_text(bytes: u64) -> String {
    let pattern = crate::i18n::ngettext("{short} ({bytes} byte)", "{short} ({bytes} bytes)", bytes);
    crate::i18n::format_message(
        &pattern,
        &[("short", &short_size(bytes)), ("bytes", &grouped(bytes))],
    )
}

/// A size as Explorer writes it in Properties before the parenthesis and
/// in its status bar: `1 byte`, `912 bytes`, `1.50 KB`, `15.4 MB`, `314
/// KB`. A size from 1000 of a unit up is written in the next unit (`0.97
/// MB`), as Explorer writes it, so it never has four digits. The status
/// bar, the details pane and an item's tooltip all write a size with it,
/// so one file has one size wherever it is shown outside the Size column.
pub fn short_size(bytes: u64) -> String {
    short_size_with(bytes, &Separators::current())
}

/// [`short_size`] with `separators`.
fn short_size_with(bytes: u64, separators: &Separators) -> String {
    let Some(unit) = SIZE_UNITS
        .iter()
        .rev()
        .find(|unit| u128::from(bytes) * 1024 >= u128::from(unit.bytes) * 1000)
        .filter(|_| bytes >= 1024)
    else {
        let pattern = crate::i18n::ngettext("{bytes} byte", "{bytes} bytes", bytes);
        return crate::i18n::format_message(
            &pattern,
            &[("bytes", &grouped_with(bytes, &separators.thousands))],
        );
    };
    let symbol = crate::i18n::gettext_static(unit.symbol);
    let hundredths = u128::from(bytes) * 100 / u128::from(unit.bytes);
    let point = &separators.decimal;
    let value = if hundredths < 1000 {
        format!("{}{point}{:02}", hundredths / 100, hundredths % 100)
    } else if hundredths < 10_000 {
        format!("{}{point}{}", hundredths / 100, hundredths % 100 / 10)
    } else {
        (hundredths / 100).to_string()
    };
    crate::i18n::format_message("{value} {unit}", &[("value", &value), ("unit", symbol)])
}

/// The decimal point and the thousands separator numbers are written
/// with. They come from the user's language, as its translators write
/// them; everything that writes a number takes them from here, so one
/// place changes when the C library's locale data is used instead.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Separators {
    decimal: String,
    thousands: String,
}

impl Separators {
    /// The separators of the user's language: `.` and `,` in English.
    fn current() -> Self {
        Self {
            decimal: crate::i18n::pgettext("decimal separator", "."),
            thousands: crate::i18n::pgettext("thousands separator", ","),
        }
    }
}

/// `1234567` as `1,234,567`, with the user's thousands separator.
pub fn grouped(value: u64) -> String {
    grouped_with(value, &Separators::current().thousands)
}

/// `value` in groups of three digits separated by `separator`.
fn grouped_with(value: u64, separator: &str) -> String {
    let digits = value.to_string();
    let mut text = String::with_capacity(digits.len() + digits.len() / 3 * separator.len());
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            text.push_str(separator);
        }
        text.push(digit);
    }
    text
}

/// `64 Files, 40 Folders`, as Explorer's Contains row says it, plural
/// even for one.
pub fn contents_text(files: u64, folders: u64) -> String {
    crate::i18n::format_message(
        "{files} Files, {folders} Folders",
        &[("files", &grouped(files)), ("folders", &grouped(folders))],
    )
}

/// `Tuesday, October 20, 2026, 14:24:52` for `time` seen at `now`, with
/// `Today` or `Yesterday` for the weekday of those days, and the clock time
/// with seconds on `clock`.
pub(super) fn long_date_time_with(
    time: &DateTime,
    now: &DateTime,
    patterns: &LocalePatterns,
    clock: ClockFormat,
) -> Option<String> {
    let yesterday = now.add_days(-1).ok()?;
    let day = if time.ymd() == now.ymd() {
        crate::i18n::gettext("Today")
    } else if time.ymd() == yesterday.ymd() {
        crate::i18n::gettext("Yesterday")
    } else {
        time.format("%A").ok()?.to_string()
    };
    let date = time.format(&crate::i18n::gettext(LONG_DATE)).ok()?;
    let clock = time.format(&patterns.time_on(clock)).ok()?;
    Some(crate::i18n::format_message(
        "{day}, {date}, {clock}",
        &[("day", &day), ("date", &date), ("clock", &clock)],
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The size reads as Explorer's: three significant digits, truncated,
    /// and every byte in brackets.
    ///
    /// parity: PROP-003
    #[test]
    fn sizes_read_as_explorer_writes_them() {
        assert_eq!(exact_size_text(322_003), "314 KB (322,003 bytes)");
        assert_eq!(exact_size_text(323_584), "316 KB (323,584 bytes)");
        assert_eq!(exact_size_text(16_154_811), "15.4 MB (16,154,811 bytes)");
        assert_eq!(exact_size_text(16_281_600), "15.5 MB (16,281,600 bytes)");
        assert_eq!(exact_size_text(1_536), "1.50 KB (1,536 bytes)");
        assert_eq!(exact_size_text(20), "20 bytes (20 bytes)");
        assert_eq!(exact_size_text(0), "0 bytes (0 bytes)");
        assert_eq!(exact_size_text(1_023_999), "999 KB (1,023,999 bytes)");
        assert_eq!(exact_size_text(1_024_000), "0.97 MB (1,024,000 bytes)");
    }

    /// One byte is written in the singular, as every other count is.
    ///
    /// parity: PROP-003
    #[test]
    fn one_byte_is_one_byte() {
        assert_eq!(short_size(1), "1 byte");
        assert_eq!(short_size(0), "0 bytes");
        assert_eq!(short_size(2), "2 bytes");
        assert_eq!(exact_size_text(1), "1 byte (1 byte)");
    }

    /// Numbers take the language's separators: a German translation's
    /// `,` and `.` give `15,4 MB (16.154.811 bytes)`.
    ///
    /// parity: PROP-003
    #[test]
    fn sizes_take_the_languages_separators() {
        let german = Separators {
            decimal: ",".to_owned(),
            thousands: ".".to_owned(),
        };
        assert_eq!(short_size_with(16_154_811, &german), "15,4 MB");
        assert_eq!(short_size_with(1_536, &german), "1,50 KB");
        assert_eq!(grouped_with(16_154_811, &german.thousands), "16.154.811");
        assert_eq!(grouped_with(1_234_567, "\u{202f}"), "1\u{202f}234\u{202f}567");
        assert_eq!(
            Separators::current(),
            Separators {
                decimal: ".".to_owned(),
                thousands: ",".to_owned()
            },
            "English without a translation"
        );
    }

    /// parity: PROP-004
    #[test]
    fn contents_count_files_and_folders_as_explorer_does() {
        assert_eq!(contents_text(64, 40), "64 Files, 40 Folders");
        assert_eq!(contents_text(1, 0), "1 Files, 0 Folders");
        assert_eq!(contents_text(12_345, 1), "12,345 Files, 1 Folders");
    }

    /// The long date names the weekday, or Today and Yesterday, and keeps
    /// the seconds.
    ///
    /// parity: PROP-003
    #[test]
    fn long_dates_read_as_explorer_writes_them() {
        let twenty_four = LocalePatterns {
            date: "%m/%d/%Y".to_owned(),
            time: "%H:%M:%S".to_owned(),
            has_day_period: false,
        };
        let at = |month, day, hour| DateTime::from_utc(2026, month, day, hour, 24, 52.0).expect("a date");
        let now = at(10, 31, 16);
        let text = |time: &DateTime| long_date_time_with(time, &now, &twenty_four, ClockFormat::Locale);
        assert_eq!(
            text(&at(10, 20, 14)).as_deref(),
            Some("Tuesday, October 20, 2026, 14:24:52")
        );
        assert_eq!(
            text(&at(10, 31, 9)).as_deref(),
            Some("Today, October 31, 2026, 09:24:52")
        );
        assert_eq!(
            text(&at(10, 30, 9)).as_deref(),
            Some("Yesterday, October 30, 2026, 09:24:52")
        );
    }
}
