// SPDX-License-Identifier: AGPL-3.0-only
//! Locale-aware `strftime` patterns for numeric dates and clock times.
//!
//! The web UI (`dateText` and `timestamp` in `v2.0.0:desktop/ui/app.js`)
//! formatted dates with the browser's `toLocaleDateString` and
//! `toLocaleString`, which follow the CLDR formats of the browser locale.
//! `glib::DateTime` has no CLDR formats, but its `%x` and `%X` conversions
//! use the C library's `LC_TIME` formats. Those differ from the browser's
//! in one way that matters for the Date modified column: many locales
//! write a two-digit year (`en_GB` is `%d/%m/%y`).
//!
//! So instead of `%x` itself, each pattern is derived from a sample: a
//! reference time is formatted with `%x` (or `%X`), and every run of digits
//! in the result is mapped back to the field it came from. The reference
//! values are chosen so no two fields print the same digits. The locale's
//! order and separators survive; the year becomes four digits and the
//! month and day two, as in the web UI's Date modified column. When the
//! sample holds anything unexpected (month names, era years, native
//! digits) the ISO pattern is used instead.

use std::sync::OnceLock;

use glib::DateTime;

/// Used when the locale's date format cannot be mapped to numbers.
const ISO_DATE: &str = "%Y-%m-%d";

/// Used when the locale's time format cannot be mapped to numbers.
const ISO_TIME: &str = "%H:%M:%S";

/// The `strftime` patterns for dates and clock times in one locale.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct LocalePatterns {
    /// The numeric date with a four-digit year and two-digit month and day:
    /// `%m/%d/%Y` for `C` and `en_US`, `%d.%m.%Y` for `de_DE`,
    /// `%Y年%m月%d日` for `ja_JP`.
    pub(super) date: String,
    /// The clock time with seconds: `%H:%M:%S` for `C` and `de_DE`,
    /// `%-I:%M:%S %p` for `en_US`. A time zone name in the locale's format
    /// (`en_IN` has one) is left out, as browsers leave it out.
    pub(super) time: String,
    /// Whether the locale has an AM/PM text, which a 12-hour clock needs.
    pub(super) has_day_period: bool,
}

impl LocalePatterns {
    /// The patterns of the process's current `LC_TIME` locale.
    pub(super) fn from_current_locale() -> Self {
        Self::from_samples(&LocaleSamples::from_current_locale())
    }

    /// The patterns that `samples` show, or the ISO patterns for samples
    /// that cannot be mapped to numbers.
    pub(super) fn from_samples(samples: &LocaleSamples) -> Self {
        let date = date_pattern_from_sample(&samples.date);
        let time = time_pattern_from_sample(&samples.time, &samples.day_period, &samples.zone);
        Self {
            date: date.unwrap_or_else(|| ISO_DATE.to_string()),
            time: time.unwrap_or_else(|| ISO_TIME.to_string()),
            has_day_period: !samples.day_period.trim().is_empty(),
        }
    }

    /// The clock-time pattern on `clock`: the locale's own, or it moved to
    /// a 24-hour or 12-hour clock. A locale without an AM/PM text keeps its
    /// own clock, as a 12-hour time without one would be ambiguous.
    pub(super) fn time_on(&self, clock: ClockFormat) -> String {
        let is_twelve_hour = self.time.contains(Field::Hour12.conversion());
        match clock {
            ClockFormat::TwentyFourHour if is_twelve_hour => {
                let time = self
                    .time
                    .replace(Field::Hour12.conversion(), Field::Hour24.conversion());
                let time = time.replace(Field::DayPeriod.conversion(), "");
                time.split_whitespace().collect::<Vec<_>>().join(" ")
            }
            ClockFormat::TwelveHour if !is_twelve_hour && self.has_day_period => {
                let time = self
                    .time
                    .replace(Field::Hour24.conversion(), Field::Hour12.conversion());
                format!("{time} {}", Field::DayPeriod.conversion())
            }
            _ => self.time.clone(),
        }
    }
}

/// The clock that times are shown on: the locale's, or the one the user
/// chose for the desktop (GNOME's `clock-format`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ClockFormat {
    /// The locale's own clock.
    #[default]
    Locale,
    /// A 24-hour clock: `19:35:35`.
    TwentyFourHour,
    /// A 12-hour clock with the locale's AM/PM text: `7:35:35 PM`.
    TwelveHour,
}

impl ClockFormat {
    /// The clock GNOME's `org.gnome.desktop.interface clock-format` asks
    /// for: `24h` or `12h`, or the locale's while the user has not set the
    /// key (`None`), whose schema default is `24h` in every locale.
    pub fn from_gnome(value: Option<&str>) -> Self {
        match value {
            Some("24h") => Self::TwentyFourHour,
            Some("12h") => Self::TwelveHour,
            _ => Self::Locale,
        }
    }
}

/// The reference time as one locale prints it.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(super) struct LocaleSamples {
    /// `%x`, the numeric date: `11/22/2033` in `en_US`.
    pub(super) date: String,
    /// `%X`, the clock time: `01:44:55 PM` in `en_US`.
    pub(super) time: String,
    /// `%p`, the afternoon marker: `PM` in `en_US`, empty in `de_DE`.
    pub(super) day_period: String,
    /// `%Z`, the time zone name, which the time pattern leaves out.
    pub(super) zone: String,
}

impl LocaleSamples {
    /// The samples in the process's current `LC_TIME` locale. A sample
    /// that cannot be produced stays empty, which selects the ISO pattern.
    fn from_current_locale() -> Self {
        let Some(reference) = reference_time() else {
            return Self::default();
        };
        let sample = |conversion: &str| {
            let formatted = reference.format(conversion);
            formatted.map(String::from).unwrap_or_default()
        };
        Self {
            date: sample("%x"),
            time: sample("%X"),
            day_period: sample("%p"),
            zone: sample("%Z"),
        }
    }
}

/// The patterns of the process's `LC_TIME` locale, derived on the first
/// call and then kept, because deriving them costs more than formatting a
/// date and the locale does not change while the app runs.
///
/// GTK sets the process locale from the environment in `gtk::init`,
/// before any widget can show a date. A call before that would keep the
/// C locale's patterns for the rest of the process.
pub(super) fn current() -> &'static LocalePatterns {
    static PATTERNS: OnceLock<LocalePatterns> = OnceLock::new();
    PATTERNS.get_or_init(LocalePatterns::from_current_locale)
}

/// The reference time: 22 November 2033, 13:44:55 UTC. Every field prints
/// different digits, and the hour reads `13` in 24-hour locales and `01`
/// or `1` in 12-hour ones. [`date_field`] and [`time_field`] map those
/// digits back.
fn reference_time() -> Option<DateTime> {
    DateTime::from_utc(2033, 11, 22, 13, 44, 55.0).ok()
}

/// Maps the digits of a formatted reference date back to `%Y`, `%m` and
/// `%d`. `None` unless each field appears exactly once and nothing else is
/// numeric.
fn date_pattern_from_sample(sample: &str) -> Option<String> {
    let found = pattern_from_sample(sample, &[], date_field)?;
    let is_complete = [Field::Year, Field::Month, Field::Day]
        .into_iter()
        .all(|field| found.has(field));
    is_complete.then_some(found.pattern)
}

/// The field a run of digits in the reference date stands for.
fn date_field(digits: &str) -> Option<Field> {
    match digits {
        "2033" | "33" => Some(Field::Year),
        "11" => Some(Field::Month),
        "22" => Some(Field::Day),
        _ => None,
    }
}

/// Maps the digits of a formatted reference time back to hours, minutes
/// and seconds and the locale's PM text to `%p`, and drops the `zone` name.
/// A 12-hour clock drops the hour's leading zero, as browsers do
/// (`7:35:35 PM`). `None` without an hour and minutes.
fn time_pattern_from_sample(sample: &str, day_period: &str, zone: &str) -> Option<String> {
    let words = [
        KnownWord {
            text: day_period,
            field: Field::DayPeriod,
        },
        KnownWord {
            text: zone,
            field: Field::Zone,
        },
    ];
    let found = pattern_from_sample(sample, &words, time_field)?;
    let has_hour = found.has(Field::Hour24) || found.has(Field::Hour12);
    let is_complete = has_hour && found.has(Field::Minute);
    is_complete.then_some(found.pattern)
}

/// The field a run of digits in the reference time stands for.
fn time_field(digits: &str) -> Option<Field> {
    match digits {
        "13" => Some(Field::Hour24),
        "01" | "1" => Some(Field::Hour12),
        "44" => Some(Field::Minute),
        "55" => Some(Field::Second),
        _ => None,
    }
}

/// A field recognised in a sample.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Field {
    /// The year, with four or two digits.
    Year,
    /// The month number.
    Month,
    /// The day of the month.
    Day,
    /// The hour on a 24-hour clock.
    Hour24,
    /// The hour on a 12-hour clock.
    Hour12,
    /// The minutes.
    Minute,
    /// The seconds.
    Second,
    /// The locale's AM or PM text.
    DayPeriod,
    /// A time zone name, which the pattern leaves out.
    Zone,
}

impl Field {
    /// The `strftime` conversion that prints the field.
    fn conversion(self) -> &'static str {
        match self {
            Field::Year => "%Y",
            Field::Month => "%m",
            Field::Day => "%d",
            Field::Hour24 => "%H",
            Field::Hour12 => "%-I",
            Field::Minute => "%M",
            Field::Second => "%S",
            Field::DayPeriod => "%p",
            Field::Zone => "",
        }
    }
}

/// A known text in a sample, such as the locale's PM text, and the field
/// it stands for.
#[derive(Debug, Clone, Copy)]
struct KnownWord<'a> {
    /// The text as the sample shows it; an empty text never matches.
    text: &'a str,
    /// The field the text stands for.
    field: Field,
}

/// A pattern rebuilt from a sample, and the fields found in it.
#[derive(Debug, Default)]
struct SamplePattern {
    /// The `strftime` pattern.
    pattern: String,
    /// Each field the pattern prints, once.
    fields: Vec<Field>,
}

impl SamplePattern {
    /// True when the pattern prints `field`.
    fn has(&self, field: Field) -> bool {
        self.fields.contains(&field)
    }

    /// Appends the conversion of `field`. False, and nothing appended, when
    /// the pattern already prints `field`: the sample is then ambiguous.
    fn push_field(&mut self, field: Field) -> bool {
        if self.has(field) {
            return false;
        }
        self.fields.push(field);
        self.pattern.push_str(field.conversion());
        true
    }

    /// Appends a character the sample shows as written; `%` is escaped.
    fn push_literal(&mut self, character: char) {
        if character == '%' {
            self.pattern.push_str("%%");
        } else {
            self.pattern.push(character);
        }
    }
}

/// Rebuilds `sample` as a pattern: `field_of_digits` names each run of
/// digits, `words` name known texts such as the day period, and anything
/// else stays literal. Surrounding spaces are trimmed. `None` when a run of
/// digits is not recognised or a field appears twice.
fn pattern_from_sample(
    sample: &str,
    words: &[KnownWord<'_>],
    field_of_digits: fn(&str) -> Option<Field>,
) -> Option<SamplePattern> {
    let mut found = SamplePattern::default();
    let mut rest = sample;
    while let Some((token, after_token)) = next_token(rest, words) {
        match token {
            Token::Digits(digits) => {
                let field = field_of_digits(digits)?;
                if !found.push_field(field) {
                    return None;
                }
            }
            Token::Word(field) => {
                if !found.push_field(field) {
                    return None;
                }
            }
            Token::Literal(character) => found.push_literal(character),
        }
        rest = after_token;
    }
    found.pattern = found.pattern.trim().to_string();
    Some(found)
}

/// One piece of a sample.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Token<'a> {
    /// A run of ASCII digits.
    Digits(&'a str),
    /// One of the known words.
    Word(Field),
    /// Any other character.
    Literal(char),
}

/// The first token of `text` and the text after it; `None` at the end.
fn next_token<'a>(text: &'a str, words: &[KnownWord<'_>]) -> Option<(Token<'a>, &'a str)> {
    let mut chars = text.chars();
    let first = chars.next()?;
    if first.is_ascii_digit() {
        let digits_end = text.find(|c: char| !c.is_ascii_digit()).unwrap_or(text.len());
        let (digits, rest) = text.split_at(digits_end);
        return Some((Token::Digits(digits), rest));
    }
    if let Some((field, rest)) = strip_word(text, words) {
        return Some((Token::Word(field), rest));
    }
    Some((Token::Literal(first), chars.as_str()))
}

/// The field of the first of `words` that `text` starts with, and the text
/// after that word. Empty words never match.
fn strip_word<'a>(text: &'a str, words: &[KnownWord<'_>]) -> Option<(Field, &'a str)> {
    for word in words.iter().filter(|word| !word.text.is_empty()) {
        if let Some(after_word) = text.strip_prefix(word.text) {
            return Some((word.field, after_word));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A locale's `%x` sample of 2033-11-22 and the date pattern it gives.
    struct DateSampleCase {
        /// Where the sample comes from: a glibc locale, or what it tests.
        source: &'static str,
        sample: &'static str,
        pattern: &'static str,
    }

    /// A locale's `%X` sample of 13:44:55 UTC, its `%p` text and the time
    /// pattern they give.
    struct TimeSampleCase {
        /// Where the sample comes from: a glibc locale, or what it tests.
        source: &'static str,
        sample: &'static str,
        day_period: &'static str,
        pattern: &'static str,
    }

    /// `%x` of 2033-11-22 as glibc locales print it, and one constructed
    /// case with a literal `%`.
    const DATE_SAMPLES: [DateSampleCase; 8] = [
        DateSampleCase {
            source: "C",
            sample: "11/22/33",
            pattern: "%m/%d/%Y",
        },
        DateSampleCase {
            source: "en_US",
            sample: "11/22/2033",
            pattern: "%m/%d/%Y",
        },
        DateSampleCase {
            source: "en_GB",
            sample: "22/11/33",
            pattern: "%d/%m/%Y",
        },
        DateSampleCase {
            source: "de_DE",
            sample: "22.11.2033",
            pattern: "%d.%m.%Y",
        },
        DateSampleCase {
            source: "en_CA, sv_SE",
            sample: "2033-11-22",
            pattern: "%Y-%m-%d",
        },
        DateSampleCase {
            source: "ja_JP",
            sample: "2033年11月22日",
            pattern: "%Y年%m月%d日",
        },
        DateSampleCase {
            source: "ko_KR",
            sample: "2033. 11. 22.",
            pattern: "%Y. %m. %d.",
        },
        DateSampleCase {
            source: "a literal percent sign",
            sample: "22%11%33",
            pattern: "%d%%%m%%%Y",
        },
    ];

    /// `%X` of 13:44:55 UTC as glibc locales print it, and constructed
    /// cases for a space-padded hour and a day period before the time.
    const TIME_SAMPLES: [TimeSampleCase; 6] = [
        TimeSampleCase {
            source: "C, de_DE",
            sample: "13:44:55",
            day_period: "PM",
            pattern: "%H:%M:%S",
        },
        TimeSampleCase {
            source: "en_US",
            sample: "01:44:55 PM",
            day_period: "PM",
            pattern: "%-I:%M:%S %p",
        },
        TimeSampleCase {
            source: "en_IN",
            sample: "01:44:55 PM UTC",
            day_period: "PM",
            pattern: "%-I:%M:%S %p",
        },
        TimeSampleCase {
            source: "a space-padded hour",
            sample: " 1:44:55 pm",
            day_period: "pm",
            pattern: "%-I:%M:%S %p",
        },
        TimeSampleCase {
            source: "a day period first",
            sample: "午後01時44分55秒",
            day_period: "午後",
            pattern: "%p%-I時%M分%S秒",
        },
        TimeSampleCase {
            source: "fi_FI",
            sample: "13.44.55",
            day_period: "",
            pattern: "%H.%M.%S",
        },
    ];

    /// Samples are `%x` of 2033-11-22 as glibc locales print it.
    ///
    /// parity: LOOK-026
    #[test]
    fn date_patterns_keep_the_locale_order_with_a_full_year() {
        for case in &DATE_SAMPLES {
            let pattern = date_pattern_from_sample(case.sample);
            assert_eq!(pattern.as_deref(), Some(case.pattern), "{}", case.source);
        }
    }

    #[test]
    fn unrecognised_date_samples_fall_back() {
        for sample in [
            "22 Nov 2033",
            "22/11/2576",
            "٢٢/١١/٢٠٣٣",
            "11/11/33",
            "",
            "Tuesday",
        ] {
            assert_eq!(date_pattern_from_sample(sample), None, "{sample}");
        }
    }

    /// Samples are `%X` of 13:44:55 UTC with the locale's `%p`.
    ///
    /// parity: LOOK-026
    #[test]
    fn time_patterns_follow_the_locale_clock() {
        for case in &TIME_SAMPLES {
            let pattern = time_pattern_from_sample(case.sample, case.day_period, "UTC");
            assert_eq!(pattern.as_deref(), Some(case.pattern), "{}", case.source);
        }
    }

    #[test]
    fn incomplete_or_unrecognised_time_samples_fall_back() {
        assert_eq!(time_pattern_from_sample("PM", "PM", "UTC"), None);
        assert_eq!(time_pattern_from_sample("13:44:55:13", "", "UTC"), None);
        assert_eq!(time_pattern_from_sample("13 Uhr", "", "UTC"), None);
    }

    #[test]
    fn samples_that_cannot_be_mapped_select_the_iso_patterns() {
        let samples = LocaleSamples {
            date: "22 Nov 2033".to_string(),
            time: "13 Uhr".to_string(),
            ..LocaleSamples::default()
        };
        let patterns = LocalePatterns::from_samples(&samples);
        assert_eq!(patterns.date, ISO_DATE);
        assert_eq!(patterns.time, ISO_TIME);
    }

    /// The test process never calls `setlocale`, so it runs in the C locale.
    ///
    /// parity: LOOK-026
    #[test]
    fn the_c_locale_gives_the_us_order() {
        let patterns = LocalePatterns::from_current_locale();
        assert_eq!(patterns.date, "%m/%d/%Y");
        assert_eq!(patterns.time, "%H:%M:%S");
        assert_eq!(current(), &patterns);
    }
}
