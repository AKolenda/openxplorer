// SPDX-License-Identifier: AGPL-3.0-only
//! Locale-aware `strftime` patterns for numeric dates and clock times.
//!
//! The web UI called `toLocaleDateString(undefined, {year: 'numeric',
//! month: '2-digit', day: '2-digit'})` and `toLocaleString()`. GLib has no
//! such API, but its `%x` and `%X` conversions use the C library's
//! `LC_TIME` formats. Those differ from the browser's in one way that
//! matters: many locales write a two-digit year (`en_GB` is `%d/%m/%y`).
//!
//! So instead of `%x` itself, each pattern is derived from a sample: a
//! reference time is formatted with `%x` (or `%X`), and every run of digits
//! in the result is mapped back to the field it came from. The reference
//! values are chosen so no two fields print the same digits. The locale's
//! order and separators survive; the year becomes four digits and the
//! month and day two, as in the web UI. When the sample holds anything
//! unexpected (month names, era years, native digits) the ISO pattern is
//! used instead.

use glib::DateTime;

/// Used when the locale's date format cannot be mapped to numbers.
const ISO_DATE: &str = "%Y-%m-%d";

/// Used when the locale's time format cannot be mapped to numbers.
const ISO_TIME: &str = "%H:%M:%S";

/// The reference time: 22 November 2033, 13:44:55 UTC. Every field prints
/// different digits, and the hour reads `13` in 24-hour locales and `01`
/// or `1` in 12-hour ones.
const REFERENCE: (i32, i32, i32, i32, i32, f64) = (2033, 11, 22, 13, 44, 55.0);

/// The `strftime` pattern for the current locale's numeric date, with a
/// four-digit year and two-digit month and day: `%m/%d/%Y` for `C` and
/// `en_US`, `%d.%m.%Y` for `de_DE`, `%Y年%m月%d日` for `ja_JP`.
pub(super) fn date_pattern() -> String {
    locale_sample("%x")
        .and_then(|sample| date_pattern_from_sample(&sample))
        .unwrap_or_else(|| ISO_DATE.to_string())
}

/// The `strftime` pattern for the current locale's clock time with
/// seconds: `%H:%M:%S` for `C` and `de_DE`, `%-I:%M:%S %p` for `en_US`.
/// A time zone name in the locale's format (`en_IN` has one) is left out,
/// as browsers leave it out.
pub(super) fn time_pattern() -> String {
    let day_period = locale_sample("%p").unwrap_or_default();
    let zone = locale_sample("%Z").unwrap_or_default();
    locale_sample("%X")
        .and_then(|sample| time_pattern_from_sample(&sample, &day_period, &zone))
        .unwrap_or_else(|| ISO_TIME.to_string())
}

/// The reference time formatted with `conversion` in the current locale.
fn locale_sample(conversion: &str) -> Option<String> {
    let (year, month, day, hour, minute, seconds) = REFERENCE;
    let reference = DateTime::from_utc(year, month, day, hour, minute, seconds).ok()?;
    reference.format(conversion).ok().map(String::from)
}

/// Maps the digits of a formatted reference date back to `%Y`, `%m` and
/// `%d`. `None` unless each field appears exactly once and nothing else is
/// numeric.
fn date_pattern_from_sample(sample: &str) -> Option<String> {
    let (pattern, fields) = pattern_from_sample(sample, &[], |digits| match digits {
        "2033" | "33" => Some(Field::Year),
        "11" => Some(Field::Month),
        "22" => Some(Field::Day),
        _ => None,
    })?;
    let complete = [Field::Year, Field::Month, Field::Day]
        .iter()
        .all(|field| fields.contains(field));
    complete.then_some(pattern)
}

/// Maps the digits of a formatted reference time back to hours, minutes
/// and seconds and the locale's PM text to `%p`, and drops the `zone` name.
/// A 12-hour clock drops the hour's leading zero, as browsers do
/// (`7:35:35 PM`). `None` without an hour and minutes.
fn time_pattern_from_sample(sample: &str, day_period: &str, zone: &str) -> Option<String> {
    let words = [(day_period, Field::DayPeriod), (zone, Field::Zone)];
    let (pattern, fields) = pattern_from_sample(sample, &words, |digits| match digits {
        "13" => Some(Field::Hour24),
        "01" | "1" => Some(Field::Hour12),
        "44" => Some(Field::Minute),
        "55" => Some(Field::Second),
        _ => None,
    })?;
    let has_hour = fields.contains(&Field::Hour24) || fields.contains(&Field::Hour12);
    let complete = has_hour && fields.contains(&Field::Minute);
    complete.then_some(pattern)
}

/// A field recognised in a sample.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Field {
    Year,
    Month,
    Day,
    Hour24,
    Hour12,
    Minute,
    Second,
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

/// Splits `sample` into digit runs, known `words` and literal text, and
/// rebuilds it as a pattern with the fields it found. Literal `%` is
/// escaped and surrounding spaces are trimmed. Fails when a digit run is
/// not recognised or a field appears twice.
fn pattern_from_sample(
    sample: &str,
    words: &[(&str, Field)],
    classify: impl Fn(&str) -> Option<Field>,
) -> Option<(String, Vec<Field>)> {
    let mut pattern = String::new();
    let mut seen: Vec<Field> = Vec::new();
    let mut rest = sample;
    while let Some(first) = rest.chars().next() {
        let (field, length) = if first.is_ascii_digit() {
            let length = rest.find(|c: char| !c.is_ascii_digit()).unwrap_or(rest.len());
            (Some(classify(&rest[..length])?), length)
        } else if let Some((word, field)) = find_word(rest, words) {
            (Some(field), word.len())
        } else {
            (None, first.len_utf8())
        };
        match field {
            Some(field) if seen.contains(&field) => return None,
            Some(field) => {
                seen.push(field);
                pattern.push_str(field.conversion());
            }
            None if first == '%' => pattern.push_str("%%"),
            None => pattern.push(first),
        }
        rest = &rest[length..];
    }
    Some((pattern.trim().to_string(), seen))
}

/// The first of `words` that `text` starts with; empty words never match.
fn find_word<'a>(text: &str, words: &[(&'a str, Field)]) -> Option<(&'a str, Field)> {
    words
        .iter()
        .copied()
        .find(|(word, _)| !word.is_empty() && text.starts_with(word))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Samples are `%x` of 2033-11-22 as glibc locales print it.
    #[test]
    fn date_patterns_keep_the_locale_order_with_a_full_year() {
        let cases = [
            ("11/22/33", "%m/%d/%Y"),           // C
            ("11/22/2033", "%m/%d/%Y"),         // en_US
            ("22/11/33", "%d/%m/%Y"),           // en_GB
            ("22.11.2033", "%d.%m.%Y"),         // de_DE
            ("2033-11-22", "%Y-%m-%d"),         // en_CA, sv_SE
            ("2033年11月22日", "%Y年%m月%d日"), // ja_JP
            ("2033. 11. 22.", "%Y. %m. %d."),   // ko_KR
            ("22%11%33", "%d%%%m%%%Y"),         // a literal percent sign
        ];
        for (sample, expected) in cases {
            assert_eq!(
                date_pattern_from_sample(sample).as_deref(),
                Some(expected),
                "{sample}"
            );
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
    #[test]
    fn time_patterns_follow_the_locale_clock() {
        let cases = [
            ("13:44:55", "PM", "%H:%M:%S"),                  // C, de_DE
            ("01:44:55 PM", "PM", "%-I:%M:%S %p"),           // en_US
            ("01:44:55 PM UTC", "PM", "%-I:%M:%S %p"),       // en_IN
            (" 1:44:55 pm", "pm", "%-I:%M:%S %p"),           // space-padded hour
            ("午後01時44分55秒", "午後", "%p%-I時%M分%S秒"), // a day period first
            ("13.44.55", "", "%H.%M.%S"),
        ];
        for (sample, period, expected) in cases {
            assert_eq!(
                time_pattern_from_sample(sample, period, "UTC").as_deref(),
                Some(expected),
                "{sample}"
            );
        }
        assert_eq!(time_pattern_from_sample("PM", "PM", "UTC"), None);
        assert_eq!(time_pattern_from_sample("13:44:55:13", "", "UTC"), None);
        assert_eq!(time_pattern_from_sample("13 Uhr", "", "UTC"), None);
    }

    /// The test process never calls `setlocale`, so it runs in the C locale.
    #[test]
    fn the_c_locale_gives_the_us_order() {
        assert_eq!(date_pattern(), "%m/%d/%Y");
        assert_eq!(time_pattern(), "%H:%M:%S");
    }
}
