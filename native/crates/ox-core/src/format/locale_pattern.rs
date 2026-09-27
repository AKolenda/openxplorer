// SPDX-License-Identifier: AGPL-3.0-only
//! Locale-aware `strftime` patterns for numeric dates and clock times.
//!
//! The web UI called `toLocaleDateString(undefined, {year: 'numeric',
//! month: '2-digit', day: '2-digit'})` and `toLocaleString()`.
//! `g_date_time_format` has no such options, but its `%x` and `%X`
//! conversions use the C library's `LC_TIME` formats. Those differ from the
//! browser's in one way that matters: many locales write a two-digit year
//! (`en_GB` is `%d/%m/%y`).
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

/// The `strftime` pattern for the current locale's numeric date, with a
/// four-digit year and two-digit month and day: `%m/%d/%Y` for `C` and
/// `en_US`, `%d.%m.%Y` for `de_DE`, `%Y年%m月%d日` for `ja_JP`.
pub(super) fn date_pattern() -> String {
    locale_sample("%x")
        .and_then(|sample| date_pattern_from_sample(&sample))
        .unwrap_or_else(|| ISO_DATE.to_owned())
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
        .unwrap_or_else(|| ISO_TIME.to_owned())
}

/// The time every sample shows: 22 November 2033, 13:44:55 UTC. Every
/// field prints different digits, and the hour reads `13` in 24-hour
/// locales and `01` or `1` in 12-hour ones; [`date_field`] and
/// [`time_field`] rely on exactly these digits.
fn reference_time() -> Option<DateTime> {
    DateTime::from_utc(2033, 11, 22, 13, 44, 55.0).ok()
}

/// The reference time formatted with `conversion` in the current locale.
fn locale_sample(conversion: &str) -> Option<String> {
    let sample = reference_time()?.format(conversion).ok()?;
    Some(sample.into())
}

/// Maps the digits of a formatted reference date back to `%Y`, `%m` and
/// `%d`. `None` unless each field appears exactly once and nothing else is
/// numeric.
fn date_pattern_from_sample(sample: &str) -> Option<String> {
    let pattern = SamplePattern::parse(sample, &[], date_field)?;
    let is_complete = [Field::Year, Field::Month, Field::Day]
        .into_iter()
        .all(|field| pattern.has(field));
    is_complete.then_some(pattern.text)
}

/// Maps the digits of a formatted reference time back to hours, minutes
/// and seconds and the locale's PM text to `%p`, and drops the `zone` name.
/// A 12-hour clock drops the hour's leading zero, as browsers do
/// (`7:35:35 PM`). `None` without an hour and minutes.
fn time_pattern_from_sample(sample: &str, day_period: &str, zone: &str) -> Option<String> {
    let words = [(day_period, Field::DayPeriod), (zone, Field::Zone)];
    let pattern = SamplePattern::parse(sample, &words, time_field)?;
    let has_hour = pattern.has(Field::Hour24) || pattern.has(Field::Hour12);
    let is_complete = has_hour && pattern.has(Field::Minute);
    is_complete.then_some(pattern.text)
}

/// The date field that prints `digits` for the [`reference_time`].
fn date_field(digits: &str) -> Option<Field> {
    match digits {
        "2033" | "33" => Some(Field::Year),
        "11" => Some(Field::Month),
        "22" => Some(Field::Day),
        _ => None,
    }
}

/// The time field that prints `digits` for the [`reference_time`].
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
    /// The year, in two or four digits.
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
    /// AM or PM in the locale's words.
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

/// One piece at the start of the rest of a sample.
#[derive(Debug, Clone, Copy)]
enum Piece {
    /// A recognised field, printed as `length` bytes of the sample.
    Field { field: Field, length: usize },
    /// Any other character, kept as it is.
    Literal(char),
}

impl Piece {
    /// The piece `text` starts with: a run of digits, one of `words`, or a
    /// literal character. `None` for digits that `digit_field` does not
    /// recognise, and for empty `text`.
    fn at_start_of(
        text: &str,
        words: &[(&str, Field)],
        digit_field: fn(&str) -> Option<Field>,
    ) -> Option<Self> {
        let first = text.chars().next()?;
        if first.is_ascii_digit() {
            let length = text
                .find(|character: char| !character.is_ascii_digit())
                .unwrap_or(text.len());
            let field = digit_field(&text[..length])?;
            return Some(Self::Field { field, length });
        }
        if let Some((word, field)) = find_word(text, words) {
            let length = word.len();
            return Some(Self::Field { field, length });
        }
        Some(Self::Literal(first))
    }

    /// The number of sample bytes the piece covers.
    fn length(self) -> usize {
        match self {
            Self::Field { length, .. } => length,
            Self::Literal(character) => character.len_utf8(),
        }
    }
}

/// The first of `words` that `text` starts with; empty words never match.
fn find_word<'a>(text: &str, words: &[(&'a str, Field)]) -> Option<(&'a str, Field)> {
    words
        .iter()
        .copied()
        .find(|(word, _)| !word.is_empty() && text.starts_with(word))
}

/// A `strftime` pattern rebuilt from a sample, and the fields it prints.
struct SamplePattern {
    text: String,
    fields: Vec<Field>,
}

impl SamplePattern {
    /// Splits `sample` into digit runs, known `words` and literal text, and
    /// rebuilds it as a pattern with the fields it found. Literal `%` is
    /// escaped and surrounding spaces are trimmed. `None` when a digit run
    /// is not recognised or a field appears twice.
    fn parse(sample: &str, words: &[(&str, Field)], digit_field: fn(&str) -> Option<Field>) -> Option<Self> {
        let mut pattern = Self {
            text: String::new(),
            fields: Vec::new(),
        };
        let mut rest = sample;
        while !rest.is_empty() {
            let piece = Piece::at_start_of(rest, words, digit_field)?;
            let repeats_a_field = matches!(piece, Piece::Field { field, .. } if pattern.has(field));
            if repeats_a_field {
                return None;
            }
            pattern.push(piece);
            rest = &rest[piece.length()..];
        }
        pattern.text = pattern.text.trim().to_owned();
        Some(pattern)
    }

    /// Whether the pattern prints `field`.
    fn has(&self, field: Field) -> bool {
        self.fields.contains(&field)
    }

    /// Appends the conversion or literal text for `piece`.
    fn push(&mut self, piece: Piece) {
        match piece {
            Piece::Field { field, .. } => {
                self.fields.push(field);
                self.text.push_str(field.conversion());
            }
            Piece::Literal('%') => self.text.push_str("%%"),
            Piece::Literal(character) => self.text.push(character),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A `%x` sample of 2033-11-22 and the pattern derived from it.
    struct DateCase {
        /// Which glibc locale prints the sample, or what it exercises.
        description: &'static str,
        sample: &'static str,
        pattern: &'static str,
    }

    /// A `%X` sample of 13:44:55 UTC with the locale's `%p`, and the
    /// pattern derived from them.
    struct TimeCase {
        /// Which glibc locale prints the sample, or what it exercises.
        description: &'static str,
        sample: &'static str,
        day_period: &'static str,
        pattern: &'static str,
    }

    const DATE_CASES: [DateCase; 8] = [
        DateCase {
            description: "C",
            sample: "11/22/33",
            pattern: "%m/%d/%Y",
        },
        DateCase {
            description: "en_US",
            sample: "11/22/2033",
            pattern: "%m/%d/%Y",
        },
        DateCase {
            description: "en_GB",
            sample: "22/11/33",
            pattern: "%d/%m/%Y",
        },
        DateCase {
            description: "de_DE",
            sample: "22.11.2033",
            pattern: "%d.%m.%Y",
        },
        DateCase {
            description: "en_CA, sv_SE",
            sample: "2033-11-22",
            pattern: "%Y-%m-%d",
        },
        DateCase {
            description: "ja_JP",
            sample: "2033年11月22日",
            pattern: "%Y年%m月%d日",
        },
        DateCase {
            description: "ko_KR",
            sample: "2033. 11. 22.",
            pattern: "%Y. %m. %d.",
        },
        DateCase {
            description: "a literal percent sign",
            sample: "22%11%33",
            pattern: "%d%%%m%%%Y",
        },
    ];

    const TIME_CASES: [TimeCase; 6] = [
        TimeCase {
            description: "C, de_DE",
            sample: "13:44:55",
            day_period: "PM",
            pattern: "%H:%M:%S",
        },
        TimeCase {
            description: "en_US",
            sample: "01:44:55 PM",
            day_period: "PM",
            pattern: "%-I:%M:%S %p",
        },
        TimeCase {
            description: "en_IN",
            sample: "01:44:55 PM UTC",
            day_period: "PM",
            pattern: "%-I:%M:%S %p",
        },
        TimeCase {
            description: "a space-padded hour",
            sample: " 1:44:55 pm",
            day_period: "pm",
            pattern: "%-I:%M:%S %p",
        },
        TimeCase {
            description: "a day period first",
            sample: "午後01時44分55秒",
            day_period: "午後",
            pattern: "%p%-I時%M分%S秒",
        },
        TimeCase {
            description: "dots between the fields",
            sample: "13.44.55",
            day_period: "",
            pattern: "%H.%M.%S",
        },
    ];

    /// parity: LOOK-026
    #[test]
    fn date_patterns_keep_the_locale_order_with_a_full_year() {
        for case in DATE_CASES {
            assert_eq!(
                date_pattern_from_sample(case.sample).as_deref(),
                Some(case.pattern),
                "{}: {}",
                case.description,
                case.sample
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

    /// parity: LOOK-026
    #[test]
    fn time_patterns_follow_the_locale_clock() {
        for case in TIME_CASES {
            assert_eq!(
                time_pattern_from_sample(case.sample, case.day_period, "UTC").as_deref(),
                Some(case.pattern),
                "{}: {}",
                case.description,
                case.sample
            );
        }
    }

    #[test]
    fn incomplete_or_unrecognised_time_samples_fall_back() {
        assert_eq!(time_pattern_from_sample("PM", "PM", "UTC"), None);
        assert_eq!(time_pattern_from_sample("13:44:55:13", "", "UTC"), None);
        assert_eq!(time_pattern_from_sample("13 Uhr", "", "UTC"), None);
    }

    /// The test process never calls `setlocale`, so it runs in the C locale.
    ///
    /// parity: LOOK-026
    #[test]
    fn the_c_locale_gives_the_us_order() {
        assert_eq!(date_pattern(), "%m/%d/%Y");
        assert_eq!(time_pattern(), "%H:%M:%S");
    }
}
