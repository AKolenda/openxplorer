// SPDX-License-Identifier: AGPL-3.0-only
//! Text formatting shared by the list, status bar and dialogs.
//!
//! Ports `prettyBytes` and `dateText` from `v2.0.0:desktop/ui/app.js`, and
//! replaces its Properties dialog `timestamp`.
//!
//! Dates follow the user's `LC_TIME` locale as the web UI followed the
//! browser locale: the order and separators of the locale's numeric date,
//! with a four-digit year and two-digit month and day. The order and
//! separators come from the C library; see `format/locale_pattern.rs`. In
//! most locales that is exactly the web UI's Date modified text
//! (`21.09.2026` in `de_DE`). In `en_US` the month and day have no leading
//! zero, as Windows' short date `M/d/yyyy` writes them (`9/21/2026`, where
//! the web UI wrote `09/21/2026`). Where the C library separates the
//! fields differently from the browser, its separators win
//! (`2026年09月21日` in `ja_JP`, where the browser wrote `2026/09/21`).
//!
//! The locale's patterns are read on the first call and kept for the life
//! of the process. GTK sets the process locale in `gtk::init`, so format
//! dates only after that.
//!
//! # Properties timestamps: a deliberate change
//!
//! The web UI's `timestamp` used the browser's `toLocaleString()`. Its
//! CLDR formats drop the zero padding in some locales
//! (`9/21/2026, 2:13:20 PM` in `en-US`, `21.9.2026, 14:13:20` in `de-DE`),
//! have no comma in others (`21/09/2026 14:13:20` in `fr-FR`) and spell the
//! day period their own way (`p.m.` in `en-CA`). `glib::DateTime` and the C
//! library have no CLDR data, so [`date_time_text`] shows the Date modified
//! text, a comma and the locale's clock time instead:
//! `9/21/2026, 2:13:20 PM`. The Properties dialog and the file list then
//! show a date the same way. Matching the browser exactly would need CLDR
//! data from ICU.

mod explorer_text;
mod locale_pattern;
mod relative_date;

use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::OnceLock;

use glib::DateTime;

pub use explorer_text::{contents_text, exact_size_text, grouped, short_size};
pub use locale_pattern::ClockFormat;
use locale_pattern::LocalePatterns;
pub use relative_date::DateStyle;

/// The [`ClockFormat`] times are shown on, as its position in
/// [`CLOCK_FORMATS`]; set by [`set_clock_format`].
static CLOCK_FORMAT: AtomicU8 = AtomicU8::new(0);

/// Every clock format, in the order [`CLOCK_FORMAT`] numbers them.
const CLOCK_FORMATS: [ClockFormat; 3] = [
    ClockFormat::Locale,
    ClockFormat::TwentyFourHour,
    ClockFormat::TwelveHour,
];

/// Shows every later clock time on `clock`, as the desktop's clock-format
/// setting asks; [`ClockFormat::Locale`] until it is called.
pub fn set_clock_format(clock: ClockFormat) {
    let position = CLOCK_FORMATS
        .iter()
        .position(|listed| *listed == clock)
        .unwrap_or(0);
    CLOCK_FORMAT.store(u8::try_from(position).unwrap_or(0), Ordering::Relaxed);
}

/// The clock format set last.
pub fn clock_format() -> ClockFormat {
    CLOCK_FORMATS[usize::from(CLOCK_FORMAT.load(Ordering::Relaxed)) % CLOCK_FORMATS.len()]
}

/// Shown in the Date modified column when a time is unknown, and in the
/// Size column when a file's size is.
const UNKNOWN_DATE: &str = "—";

/// Shown in the Properties dialog when a time is unknown.
const UNKNOWN_TIMESTAMP: &str = crate::i18n::message_id("Not provided");

/// A size unit after bytes and the number of bytes in one of it.
#[derive(Debug, Clone, Copy)]
struct SizeUnit {
    /// The text after the number: `KB`, `MB`, `GB` or `TB`.
    symbol: &'static str,
    /// The bytes in one unit, a power of 1024.
    bytes: u64,
}

/// The size units from smallest to largest. Sizes past 1024 TB stay in TB,
/// as in the web UI.
const SIZE_UNITS: [SizeUnit; 4] = [
    SizeUnit {
        symbol: crate::i18n::message_id("KB"),
        bytes: 1 << 10,
    },
    SizeUnit {
        symbol: crate::i18n::message_id("MB"),
        bytes: 1 << 20,
    },
    SizeUnit {
        symbol: crate::i18n::message_id("GB"),
        bytes: 1 << 30,
    },
    SizeUnit {
        symbol: crate::i18n::message_id("TB"),
        bytes: 1 << 40,
    },
];

/// [`pretty_bytes`] of a size that may be unknown: `—` when it is
/// (`prettyBytes(undefined)` in app.js).
pub fn size_text(bytes: Option<u64>) -> String {
    bytes.map_or_else(|| UNKNOWN_DATE.to_owned(), pretty_bytes)
}

/// A file's size as Windows Explorer's Size column writes it: whole
/// kilobytes, rounded up, with thousands separated (`1 KB` for 20 bytes,
/// `0 KB` for an empty file, `2,345 KB`); empty when unknown.
pub fn column_size_text(bytes: Option<u64>) -> String {
    let Some(bytes) = bytes else {
        return String::new();
    };
    let kilobytes = bytes.div_ceil(1024);
    crate::i18n::format_message(
        "{value} {unit}",
        &[
            ("value", &group_digits(kilobytes)),
            ("unit", crate::i18n::gettext_static("KB")),
        ],
    )
}

/// How a locale groups the digits of a whole number: the separator, and
/// whether groups after the first three digits have two digits, as Indian
/// locales write lakhs and crores (`12,34,567`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DigitGrouping {
    separator: &'static str,
    lakhs: bool,
}

/// Languages that group digits with a full stop (CLDR `group` `.`).
const FULL_STOP_LANGUAGES: [&str; 19] = [
    "az", "ca", "da", "de", "el", "es", "eu", "gl", "hr", "id", "is", "it", "nl", "pt", "ro", "sl", "sr",
    "tr", "vi",
];

/// Languages that group digits with a no-break space (CLDR `group`
/// U+00A0).
const SPACE_LANGUAGES: [&str; 17] = [
    "be", "bg", "cs", "et", "fi", "hu", "kk", "lt", "lv", "nb", "nn", "no", "pl", "ru", "sk", "sv", "uk",
];

/// Regions where Spanish groups digits with a comma (CLDR), where Spain
/// and most of South America use a full stop.
const COMMA_SPANISH_REGIONS: [&str; 11] = ["419", "DO", "GT", "HN", "MX", "NI", "PA", "PE", "PR", "SV", "US"];

/// Languages of India, which group digits in lakhs (CLDR `#,##,##0`).
const LAKH_LANGUAGES: [&str; 12] = [
    "as", "bn", "gu", "hi", "kn", "ml", "mr", "ne", "or", "pa", "ta", "te",
];

impl DigitGrouping {
    /// Commas between groups of three: English, Chinese, Japanese, `C`.
    pub const COMMAS: Self = Self {
        separator: ",",
        lakhs: false,
    };

    /// How the locale named `locale`, such as `de_AT.UTF-8`, groups digits,
    /// after CLDR for the locales listed here; commas in threes for the
    /// rest (including `C` and `POSIX`).
    ///
    /// `GLib` has no number formatting call and `localeconv` would need
    /// `unsafe`, so the language and region decide, as for the first
    /// weekday.
    pub fn for_locale(locale: &str) -> Self {
        let name = locale.split(['.', '@']).next().unwrap_or_default();
        let (language, region) = name.split_once('_').unwrap_or((name, ""));
        let separator = match (language, region) {
            ("de" | "it" | "rm", "CH" | "LI") => "\u{2019}",
            // French in Canada writes a no-break space, elsewhere a narrow
            // one, as Windows does.
            ("de", "AT") | ("pt", "PT" | "AO" | "MZ") | ("fr", "CA") | ("es", "CR") => "\u{a0}",
            ("es", region) if COMMA_SPANISH_REGIONS.contains(&region) => ",",
            ("fr", _) => "\u{202f}",
            _ if FULL_STOP_LANGUAGES.contains(&language) => ".",
            _ if SPACE_LANGUAGES.contains(&language) => "\u{a0}",
            _ => ",",
        };
        Self {
            separator,
            // India groups in lakhs, and so do its languages named without
            // a region, Bengali in Bangladesh and Nepali in Nepal.
            lakhs: region == "IN"
                || (region.is_empty() && LAKH_LANGUAGES.contains(&language))
                || matches!(language, "bn" | "ne"),
        }
    }

    /// `value` with its digits grouped.
    pub fn group(self, value: u64) -> String {
        let digits = value.to_string();
        let mut grouped = String::with_capacity(digits.len() * 2);
        for (index, digit) in digits.chars().enumerate() {
            let remaining = digits.len() - index;
            let starts_group = if self.lakhs && remaining > 3 {
                (remaining - 3).is_multiple_of(2)
            } else {
                remaining.is_multiple_of(3)
            };
            if index > 0 && starts_group {
                grouped.push_str(self.separator);
            }
            grouped.push(digit);
        }
        grouped
    }

    /// How the process's numeric locale groups digits: `LC_ALL`, else
    /// `LC_NUMERIC`, else `LANG`, as the C library chooses it; read once.
    pub fn current() -> Self {
        static CURRENT: OnceLock<DigitGrouping> = OnceLock::new();
        *CURRENT.get_or_init(|| {
            let locale = ["LC_ALL", "LC_NUMERIC", "LANG"]
                .into_iter()
                .filter_map(|variable| std::env::var(variable).ok())
                .find(|value| !value.is_empty())
                .unwrap_or_default();
            Self::for_locale(&locale)
        })
    }
}

/// `value` with its digits grouped as the process's numeric locale groups
/// them: `35,000`, `35.000` in German, `1,23,456` in India.
pub fn group_digits(value: u64) -> String {
    DigitGrouping::current().group(value)
}

/// `912 bytes`, `71.0 KB`, `130 KB`, `1.1 MB`: one decimal below 100 and
/// none from 100 up, in powers of 1024.
///
/// Halves round up, like JavaScript's `toFixed` (`1280` bytes is `1.3 KB`),
/// and the arithmetic is exact, so every size gets the same text as in the
/// web interface.
pub fn pretty_bytes(bytes: u64) -> String {
    let Some(unit) = largest_unit(bytes) else {
        return crate::i18n::format_message("{bytes} bytes", &[("bytes", &bytes.to_string())]);
    };
    let symbol = crate::i18n::gettext_static(unit.symbol);
    // Integer arithmetic wide enough for ten times `u64::MAX`.
    let bytes = u128::from(bytes);
    let unit_bytes = u128::from(unit.bytes);
    if bytes >= 100 * unit_bytes {
        let whole = round_half_up(bytes, unit_bytes);
        return crate::i18n::format_message(
            "{value} {unit}",
            &[("value", &whole.to_string()), ("unit", symbol)],
        );
    }
    let tenths = round_half_up(bytes * 10, unit_bytes);
    crate::i18n::format_message(
        "{whole}.{fraction} {unit}",
        &[
            ("whole", &(tenths / 10).to_string()),
            ("fraction", &(tenths % 10).to_string()),
            ("unit", symbol),
        ],
    )
}

/// The largest of [`SIZE_UNITS`] that `bytes` fills at least once, or
/// `None` below 1 KB.
fn largest_unit(bytes: u64) -> Option<SizeUnit> {
    SIZE_UNITS.into_iter().rev().find(|unit| bytes >= unit.bytes)
}

/// `numerator / denominator` rounded to the nearest integer, halves up.
fn round_half_up(numerator: u128, denominator: u128) -> u128 {
    (2 * numerator + denominator) / (2 * denominator)
}

/// Local date for the Date modified column, for example `9/26/2026` in
/// the US, `26.09.2026` in Germany or `2026年09月26日` in Japan; `—` when
/// the time is unknown (`None`) or out of range.
pub fn date_text(unix_seconds: Option<u64>) -> String {
    unix_seconds
        .and_then(local_time)
        .and_then(|time| format_date(&time))
        .unwrap_or_else(|| UNKNOWN_DATE.to_owned())
}

/// Local date and time for the Properties dialog's Created, Modified and
/// Accessed rows: the [`date_text`] date and the locale's clock time, for
/// example `9/26/2026, 7:35:35 PM` in the US or `26.09.2026, 19:35:35` in
/// Germany; `Not provided` when the time is unknown (`None`) or out of
/// range. See the module documentation for how this differs from the web
/// UI.
pub fn date_time_text(unix_seconds: Option<u64>) -> String {
    unix_seconds
        .and_then(local_time)
        .and_then(|time| format_date_time(&time))
        .unwrap_or_else(|| crate::i18n::gettext(UNKNOWN_TIMESTAMP))
}

/// Local date and short clock time for the Date modified column, as
/// Windows Explorer and Dolphin show it: the [`date_text`] date and the
/// locale's clock time without seconds, for example `9/26/2026 7:35 PM`
/// in the US or `26.09.2026 19:35` in Germany; `—` when unknown.
pub fn date_short_time_text(unix_seconds: Option<u64>) -> String {
    unix_seconds
        .and_then(local_time)
        .and_then(|time| format_date_short_time_with(&time, locale_pattern::current(), clock_format()))
        .unwrap_or_else(|| UNKNOWN_DATE.to_owned())
}

/// The Date modified column's text in `style`: "Today at 7:35 PM" or
/// "Yesterday at 7:35 PM" for recent dates in [`DateStyle::Relative`],
/// else [`date_short_time_text`] (VIEW-004).
pub fn column_date_text(unix_seconds: Option<u64>, style: DateStyle) -> String {
    let relative = || {
        let time = local_time(unix_seconds?)?;
        let now = DateTime::now_local().ok()?;
        relative_date::relative_text(&time, &now, locale_pattern::current(), clock_format())
    };
    let text = match style {
        DateStyle::Relative => relative(),
        DateStyle::Absolute => None,
    };
    text.unwrap_or_else(|| date_short_time_text(unix_seconds))
}

/// The Properties window's Created, Modified and Accessed text, as
/// Windows 11 writes it: `Tuesday, October 20, 2026, 14:24:52`, with
/// `Today` or `Yesterday` for the weekday of those days; `Not provided`
/// when the time is unknown.
pub fn long_date_time_text(unix_seconds: Option<u64>) -> String {
    unix_seconds
        .and_then(local_time)
        .and_then(|time| {
            let now = DateTime::now_local().ok()?;
            explorer_text::long_date_time_with(&time, &now, locale_pattern::current(), clock_format())
        })
        .unwrap_or_else(|| crate::i18n::gettext(UNKNOWN_TIMESTAMP))
}

/// [`date_text`] for a time GIO already returned as a [`DateTime`], in the
/// time zone it carries. `None` if it cannot be formatted.
pub fn format_date(time: &DateTime) -> Option<String> {
    format_date_with(time, locale_pattern::current())
}

/// [`date_time_text`] for a time GIO already returned as a [`DateTime`], in
/// the time zone it carries. `None` if it cannot be formatted.
pub fn format_date_time(time: &DateTime) -> Option<String> {
    format_date_time_with(time, locale_pattern::current(), clock_format())
}

/// [`format_date`] with the given locale `patterns`, which the tests
/// choose per locale.
fn format_date_with(time: &DateTime, patterns: &LocalePatterns) -> Option<String> {
    let date = time.format(&patterns.date).ok()?;
    Some(date.into())
}

/// [`format_date_time`] with the given locale `patterns`: the date, a
/// comma and the clock time on `clock`.
fn format_date_time_with(time: &DateTime, patterns: &LocalePatterns, clock: ClockFormat) -> Option<String> {
    let date = format_date_with(time, patterns)?;
    let clock = time.format(&patterns.time_on(clock)).ok()?;
    Some(format!("{date}, {clock}"))
}

/// [`date_short_time_text`] with the given locale `patterns`: the date, a
/// space and the clock time on `clock` without seconds.
fn format_date_short_time_with(
    time: &DateTime,
    patterns: &LocalePatterns,
    clock: ClockFormat,
) -> Option<String> {
    let date = format_date_with(time, patterns)?;
    let clock = time.format(&without_seconds(&patterns.time_on(clock))).ok()?;
    Some(format!("{date} {clock}"))
}

/// A clock `pattern` without its seconds and the separator before them:
/// `%-I:%M:%S %p` becomes `%-I:%M %p`, and a unit after the seconds, as in
/// Korean `%H시 %M분 %S초`, goes with them.
fn without_seconds(pattern: &str) -> String {
    let Some(at) = pattern.find("%S") else {
        return pattern.to_owned();
    };
    let is_separator = |c: char| c.is_whitespace() || (c.is_ascii_punctuation() && c != '%');
    let before = pattern[..at].trim_end_matches(is_separator);
    let after = pattern[at + 2..].trim_start_matches(|c: char| c.is_alphabetic() && !c.is_ascii());
    format!("{before}{after}")
}

/// The local time for a Unix timestamp; `None` for times a [`DateTime`]
/// cannot hold.
fn local_time(unix_seconds: u64) -> Option<DateTime> {
    let seconds = i64::try_from(unix_seconds).ok()?;
    DateTime::from_unix_local(seconds).ok()
}

#[cfg(test)]
mod tests {
    use super::locale_pattern::LocaleSamples;
    use super::*;

    /// The Size column writes whole kilobytes, rounded up and grouped, as
    /// Windows Explorer's does, and nothing for an unknown size.
    ///
    /// parity: VIEW-001
    #[test]
    fn the_size_column_writes_whole_kilobytes_rounded_up() {
        assert_eq!(column_size_text(Some(0)), "0 KB");
        assert_eq!(column_size_text(Some(20)), "1 KB");
        assert_eq!(column_size_text(Some(1024)), "1 KB");
        assert_eq!(column_size_text(Some(1025)), "2 KB");
        assert_eq!(column_size_text(None), "");
        let commas = DigitGrouping::COMMAS;
        assert_eq!(commas.group(2_345), "2,345");
        assert_eq!(commas.group(999), "999");
        assert_eq!(commas.group(0), "0");
    }

    /// Digits are grouped as the numeric locale groups them, after CLDR:
    /// the separator by language and region, and lakhs in India.
    ///
    /// parity: VIEW-001
    #[test]
    fn digit_groups_follow_the_locale() {
        let grouped = |locale: &str, value: u64| DigitGrouping::for_locale(locale).group(value);
        assert_eq!(grouped("en_US.UTF-8", 1_048_576), "1,048,576");
        assert_eq!(grouped("zh_CN.UTF-8", 1_048_576), "1,048,576");
        assert_eq!(grouped("C", 1_048_576), "1,048,576");
        assert_eq!(grouped("de_DE.UTF-8", 1_048_576), "1.048.576");
        assert_eq!(grouped("de_AT.UTF-8", 1_234), "1\u{a0}234");
        assert_eq!(grouped("de_CH.UTF-8", 1_234), "1\u{2019}234");
        assert_eq!(grouped("es_ES.UTF-8", 1_234), "1.234");
        assert_eq!(grouped("es_PE.UTF-8", 1_234), "1,234");
        assert_eq!(grouped("es_MX.UTF-8", 1_234), "1,234");
        assert_eq!(grouped("pt_BR.UTF-8", 1_234), "1.234");
        assert_eq!(grouped("pt_PT.UTF-8", 1_234), "1\u{a0}234");
        assert_eq!(grouped("fr_FR.UTF-8", 1_234), "1\u{202f}234");
        assert_eq!(grouped("fr_CA.UTF-8", 1_234), "1\u{a0}234");
        assert_eq!(grouped("ru_RU.UTF-8", 1_234), "1\u{a0}234");
        assert_eq!(grouped("en_IN.UTF-8", 123_456), "1,23,456");
        assert_eq!(grouped("hi_IN.UTF-8", 12_345_678), "1,23,45,678");
        assert_eq!(grouped("en_IN.UTF-8", 999), "999");
        assert_eq!(grouped("en_IN.UTF-8", 1_000), "1,000");
        assert_eq!(grouped("bn_BD.UTF-8", 123_456), "1,23,456");
        assert_eq!(grouped("hi.UTF-8", 123_456), "1,23,456");
        assert_eq!(grouped("ne_NP.UTF-8", 123_456), "1,23,456");
        assert_eq!(grouped("ta_LK.UTF-8", 123_456), "123,456");
        assert_eq!(grouped("es_CR.UTF-8", 1_234), "1\u{a0}234");
    }

    /// One locale's samples of the reference time in [`locale_pattern`], as
    /// glibc 2.39 prints them, and the texts for 2026-09-21 14:13:20 UTC.
    struct LocaleCase {
        locale: &'static str,
        /// `%x`.
        date_sample: &'static str,
        /// `%X`.
        time_sample: &'static str,
        /// `%p`.
        day_period: &'static str,
        /// The Date modified column: the web UI's `dateText` for this
        /// locale, from Node.js 24.21 (ICU 78.3, CLDR 48), but for
        /// `en_US`, where it is Windows' short date.
        column: &'static str,
        /// [`date_time_text`]. The comment above each case has the web
        /// UI's `timestamp`, from the same Node.js.
        properties: &'static str,
    }

    impl LocaleCase {
        /// The patterns this locale's samples give.
        fn patterns(&self) -> LocalePatterns {
            LocalePatterns::from_samples(&LocaleSamples {
                date: self.date_sample.to_string(),
                time: self.time_sample.to_string(),
                day_period: self.day_period.to_string(),
                zone: "UTC".to_string(),
                locale: format!("{}.UTF-8", self.locale),
            })
        }
    }

    const LOCALE_CASES: [LocaleCase; 5] = [
        // Web UI: 9/21/2026, 2:13:20 PM. Its Date modified column wrote
        // 09/21/2026; this is Windows' M/d/yyyy.
        LocaleCase {
            locale: "en_US",
            date_sample: "11/22/2033",
            time_sample: "01:44:55 PM",
            day_period: "PM",
            column: "9/21/2026",
            properties: "9/21/2026, 2:13:20 PM",
        },
        // Web UI: 2026-09-21, 2:13:20 p.m.
        LocaleCase {
            locale: "en_CA",
            date_sample: "2033-11-22",
            time_sample: "01:44:55 PM",
            day_period: "PM",
            column: "2026-09-21",
            properties: "2026-09-21, 2:13:20 PM",
        },
        // Web UI: 21/09/2026, 14:13:20
        LocaleCase {
            locale: "en_GB",
            date_sample: "22/11/33",
            time_sample: "13:44:55",
            day_period: "pm",
            column: "21/09/2026",
            properties: "21/09/2026, 14:13:20",
        },
        // Web UI: 21.9.2026, 14:13:20
        LocaleCase {
            locale: "de_DE",
            date_sample: "22.11.2033",
            time_sample: "13:44:55",
            day_period: "",
            column: "21.09.2026",
            properties: "21.09.2026, 14:13:20",
        },
        // Web UI: 21/09/2026 14:13:20
        LocaleCase {
            locale: "fr_FR",
            date_sample: "22/11/2033",
            time_sample: "13:44:55",
            day_period: "",
            column: "21/09/2026",
            properties: "21/09/2026, 14:13:20",
        },
    ];

    /// [`september_21`] as Unix seconds, for the functions that take them.
    const SEPTEMBER_21_UNIX_SECONDS: u64 = 1_790_000_000;

    fn september_21() -> DateTime {
        DateTime::from_utc(2026, 9, 21, 14, 13, 20.0).expect("valid date")
    }

    /// Ported from the size examples in `v2.0.0:desktop/ui/app.js::prettyBytes`.
    ///
    /// parity: VIEW-003
    #[test]
    fn sizes_match_the_web_interface() {
        assert_eq!(size_text(None), "—");
        assert_eq!(size_text(Some(0)), "0 bytes");
        assert_eq!(pretty_bytes(0), "0 bytes");
        assert_eq!(pretty_bytes(1), "1 bytes");
        assert_eq!(pretty_bytes(912), "912 bytes");
        assert_eq!(pretty_bytes(1023), "1023 bytes");
        assert_eq!(pretty_bytes(1024), "1.0 KB");
        assert_eq!(pretty_bytes(72_704), "71.0 KB");
        assert_eq!(pretty_bytes(102_400), "100 KB");
        assert_eq!(pretty_bytes(133_120), "130 KB");
        assert_eq!(pretty_bytes(1_048_063), "1023 KB");
        assert_eq!(pretty_bytes(1_048_064), "1024 KB");
        assert_eq!(pretty_bytes(1_048_576), "1.0 MB");
        assert_eq!(pretty_bytes(104_805_376), "100.0 MB");
        assert_eq!(pretty_bytes(1 << 30), "1.0 GB");
        assert_eq!(pretty_bytes(1 << 40), "1.0 TB");
    }

    /// JavaScript's `toFixed` rounds exact halves up; Rust's formatter
    /// would round them to even. Values from running `prettyBytes` in Node.
    ///
    /// parity: VIEW-003
    #[test]
    fn halves_round_up_like_to_fixed() {
        assert_eq!(pretty_bytes(1280), "1.3 KB");
        assert_eq!(pretty_bytes(1536), "1.5 KB");
        assert_eq!(pretty_bytes(102_912), "101 KB");
        assert_eq!(pretty_bytes(10_291), "10.0 KB");
        assert_eq!(pretty_bytes(10_292), "10.1 KB");
    }

    /// `prettyBytes` stops dividing at TB.
    ///
    /// parity: VIEW-003
    #[test]
    fn sizes_past_a_petabyte_stay_in_terabytes() {
        assert_eq!(pretty_bytes(1 << 50), "1024 TB");
        assert_eq!(pretty_bytes(1 << 53), "8192 TB");
        assert_eq!(pretty_bytes(u64::MAX), "16777216 TB");
    }

    /// Ported from `v2.0.0:desktop/ui/app.js::dateText` (`n ? … : '—'`) and
    /// `timestamp` (`value ? … : 'Not provided'`). The web interface got 0
    /// for an unknown time; here it is `None`, and the entry module reads a
    /// reported 0 as `None` too.
    ///
    /// parity: VIEW-001
    #[test]
    fn unknown_times_use_the_web_placeholders() {
        assert_eq!(date_text(None), "—");
        assert_eq!(date_time_text(None), "Not provided");
        assert_eq!(date_text(Some(u64::MAX)), "—");
        assert_eq!(date_time_text(Some(u64::MAX)), "Not provided");
    }

    /// Without `setlocale` the process uses the C locale, whose `%x` is
    /// `%m/%d/%y`: the result is the US order with a four-digit year, as
    /// `toLocaleDateString` gives for `en-US`.
    ///
    /// parity: VIEW-001, LOOK-026
    #[test]
    fn dates_follow_the_c_locale_with_a_full_year() {
        let time = DateTime::from_utc(2026, 9, 6, 19, 5, 7.0).expect("valid date");
        assert_eq!(format_date(&time).as_deref(), Some("09/06/2026"));
        assert_eq!(format_date_time(&time).as_deref(), Some("09/06/2026, 19:05:07"));
    }

    /// The Date modified column shows what the web UI's `dateText` showed,
    /// but for `en_US`'s Windows short date.
    ///
    /// parity: VIEW-001, LOOK-026
    #[test]
    fn column_dates_match_the_web_ui_in_each_locale() {
        for case in &LOCALE_CASES {
            let date = format_date_with(&september_21(), &case.patterns());
            assert_eq!(date.as_deref(), Some(case.column), "{}", case.locale);
        }
    }

    /// The Date modified column shows the date and the clock time without
    /// seconds, as Windows Explorer does: `9/6/2026 7:05 PM` in the US.
    ///
    /// parity: VIEW-004
    #[test]
    fn column_dates_show_the_time_without_seconds() {
        let us = LOCALE_CASES[0].patterns();
        let time = DateTime::from_utc(2026, 9, 6, 19, 5, 7.0).expect("valid date");
        let text = format_date_short_time_with(&time, &us, ClockFormat::Locale);
        assert_eq!(text.as_deref(), Some("9/6/2026 7:05 PM"));
        let on_24_hours = format_date_short_time_with(&time, &us, ClockFormat::TwentyFourHour);
        assert_eq!(
            on_24_hours.as_deref(),
            Some("9/6/2026 19:05"),
            "the desktop's clock"
        );
        assert_eq!(without_seconds("%H:%M:%S"), "%H:%M");
        assert_eq!(without_seconds("%H시 %M분 %S초"), "%H시 %M분");
        assert_eq!(date_short_time_text(None), "—");
    }

    /// The deliberate change from the web UI's `timestamp`, described in
    /// the module documentation.
    ///
    /// parity: LOOK-026
    #[test]
    fn properties_timestamps_add_the_locale_clock_to_the_column_date() {
        for case in &LOCALE_CASES {
            let timestamp = format_date_time_with(&september_21(), &case.patterns(), ClockFormat::Locale);
            assert_eq!(timestamp.as_deref(), Some(case.properties), "{}", case.locale);
        }
    }

    /// The desktop's clock format moves Properties timestamps to a 24-hour
    /// or 12-hour clock; a locale without an AM/PM text keeps its own.
    ///
    /// parity: LOOK-026
    #[test]
    fn properties_timestamps_follow_the_desktop_clock_format() {
        let on_clock = |locale: &str, clock| {
            let case = LOCALE_CASES
                .iter()
                .find(|case| case.locale == locale)
                .expect("a case");
            format_date_time_with(&september_21(), &case.patterns(), clock).expect("formats")
        };
        assert_eq!(
            on_clock("en_US", ClockFormat::TwentyFourHour),
            "9/21/2026, 14:13:20"
        );
        assert_eq!(
            on_clock("en_US", ClockFormat::TwelveHour),
            "9/21/2026, 2:13:20 PM"
        );
        assert_eq!(
            on_clock("en_GB", ClockFormat::TwentyFourHour),
            "21/09/2026, 14:13:20"
        );
        assert_eq!(on_clock("de_DE", ClockFormat::TwelveHour), "21.09.2026, 14:13:20");
        assert_eq!(ClockFormat::from_gnome(Some("12h")), ClockFormat::TwelveHour);
        assert_eq!(ClockFormat::from_gnome(Some("24h")), ClockFormat::TwentyFourHour);
        assert_eq!(ClockFormat::from_gnome(None), ClockFormat::Locale);
    }

    /// A Date modified text in the test process's own time zone has ten
    /// characters: a four-digit year and a two-digit month and day, with
    /// their separators.
    ///
    /// parity: VIEW-001
    #[test]
    fn column_dates_have_a_four_digit_year_and_two_digit_fields() {
        let text = date_text(Some(SEPTEMBER_21_UNIX_SECONDS));
        assert_eq!(text.len(), 10, "{text}");
        assert!(text.contains("2026"), "{text}");
    }

    /// The Properties timestamp begins with the Date modified text of the
    /// same time.
    ///
    /// parity: VIEW-001
    #[test]
    fn date_time_text_begins_with_the_column_date_text() {
        let column_date = date_text(Some(SEPTEMBER_21_UNIX_SECONDS));
        let timestamp = date_time_text(Some(SEPTEMBER_21_UNIX_SECONDS));
        assert!(timestamp.starts_with(&column_date), "{timestamp}");
    }
}
