// SPDX-License-Identifier: AGPL-3.0-only
//! Search options typed into the search box (SRCH-044): `kind:`,
//! `datemodified:` (or `modified:`) and `size:` choose what the strip's
//! Kind, Date modified and Size lists choose, and the other words are
//! searched for as before.
//!
//! The keywords and values are those Microsoft documents for Windows
//! Search's Advanced Query Syntax
//! (<https://learn.microsoft.com/en-us/windows/win32/lwef/-search-2x-wds-aqsreference>:
//! "Date modified | datemodified or modified | modified:last week",
//! "date:today", "date:yesterday", "date:this week", "date:last week",
//! and the same for "Week/Month/Year",
//! "kind:docs", "kind:pics", "kind:videos", "kind:folders", "kind:music",
//! "size:>500", "size:<500") and the `System.Kind` values
//! (<https://learn.microsoft.com/en-us/windows/win32/properties/props-system-kind>:
//! document, folder, picture, music, video) and `System.Size` groups
//! (<https://learn.microsoft.com/en-us/windows/win32/properties/props-system-size>:
//! Empty, Tiny, Small, Medium, Large, Huge, Gigantic).
//!
//! `date:` names a date as `datemodified:` does ("Date | date |
//! date:last week"), since the date modified is the only date searched.
//! A date or size may also be compared or given as a range, as Windows
//! Search's Advanced Query Syntax documents
//! (<https://learn.microsoft.com/en-us/windows/win32/search/-search-3x-advancedquerysyntax>:
//! `System.Size:<1kb`, `System.Size:>=1kb`, `System.Size:5kb..10kb`,
//! `System.ItemDate:11/05/04..11/10/04`, ranges "inclusive", and ISO 8601
//! dates; the 2.x reference: `date:>11/13/21` "after MM/DD/YY",
//! `date:11/13/21..11/15/21`). Dates are read as ISO 8601
//! (`2026-01-31`) or month/day/year (`1/31/2026`, `1/31/26`); sizes as
//! bytes or with `kb`, `mb` or `gb` (1,024 bytes a kilobyte).
//!
//! Any other value, such as `next month`, stays a word to search for, as
//! does every other `name:value`. Quoted text is never an option, and an
//! option after `NOT` or next to `OR` stays a word too, since the options
//! narrow every result and cannot be left out or offered as one choice.

use super::facets::{CalendarDay, DateFacet, KindFacet, SearchFacets, SizeFacet};

/// The search box's text, read as words and typed options.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TypedQuery {
    /// The words to search for, in the order typed, one space apart.
    pub words: String,
    /// The options typed; the last of each kind counts.
    pub facets: SearchFacets,
}

impl TypedQuery {
    /// Reads `text`. Keywords and values ignore case; a date of two words
    /// may be typed as they are (`datemodified:this week`) or quoted
    /// (`datemodified:"this week"`).
    pub fn parse(text: &str) -> Self {
        let tokens = blank_separated(text);
        let mut words: Vec<&str> = Vec::new();
        let mut facets = SearchFacets::default();
        let mut index = 0;
        while index < tokens.len() {
            let token = tokens[index];
            let next = tokens.get(index + 1).copied();
            let after = |used: usize| tokens.get(index + used).copied();
            let is_operand = index
                .checked_sub(1)
                .and_then(|before| tokens.get(before))
                .is_some_and(|before| matches!(*before, "NOT" | "OR"));
            let option =
                read_option(token, next).filter(|(_, used)| !is_operand && after(*used) != Some("OR"));
            if let Some((option, used)) = option {
                match option {
                    Typed::Kind(kind) => facets.kind = kind,
                    Typed::Date(date) => facets.date = date,
                    Typed::Size(size) => facets.size = size,
                }
                index += used;
            } else {
                words.push(token);
                index += 1;
            }
        }
        Self {
            words: words.join(" "),
            facets,
        }
    }
}

/// The pieces of `text` between blanks, where quoted text, blanks and
/// all, stays in one piece.
fn blank_separated(text: &str) -> Vec<&str> {
    let mut pieces = Vec::new();
    let mut start = None;
    let mut in_quotes = false;
    for (at, character) in text.char_indices() {
        if character == '"' {
            in_quotes = !in_quotes;
        }
        if character.is_whitespace() && !in_quotes {
            if let Some(from) = start.take() {
                pieces.push(&text[from..at]);
            }
        } else if start.is_none() {
            start = Some(at);
        }
    }
    if let Some(from) = start {
        pieces.push(&text[from..]);
    }
    pieces
}

/// One typed option.
enum Typed {
    Kind(KindFacet),
    Date(DateFacet),
    Size(SizeFacet),
}

/// The option `token` types, with `next` when a date takes two words, and
/// how many tokens it used; `None` when it is a word to search for.
fn read_option(token: &str, next: Option<&str>) -> Option<(Typed, usize)> {
    let (keyword, value) = token.split_once(':')?;
    let value = value.to_lowercase();
    match keyword.to_lowercase().as_str() {
        "kind" => kind(&value).map(|kind| (Typed::Kind(kind), 1)),
        "size" => size(&value).map(|size| (Typed::Size(size), 1)),
        "datemodified" | "modified" | "date" => {
            let two_words = |first: &str| {
                let second = next?.to_lowercase();
                let second = if value.starts_with('"') {
                    second.strip_suffix('"')?.to_owned()
                } else {
                    second
                };
                date(&format!("{first} {second}")).map(|date| (Typed::Date(date), 2))
            };
            if let Some(inner) = value.strip_prefix('"') {
                return match inner.strip_suffix('"') {
                    Some(whole) => date(whole).map(|date| (Typed::Date(date), 1)),
                    None => two_words(inner),
                };
            }
            date(&value)
                .map(|date| (Typed::Date(date), 1))
                .or_else(|| two_words(&value))
        }
        _ => None,
    }
}

/// The kind a `kind:` value names.
fn kind(value: &str) -> Option<KindFacet> {
    Some(match value {
        "folder" | "folders" => KindFacet::Folders,
        "document" | "docs" => KindFacet::Documents,
        "picture" | "pics" => KindFacet::Images,
        "music" => KindFacet::Audio,
        "video" | "videos" => KindFacet::Videos,
        _ => return None,
    })
}

/// The range a `datemodified:` value names: a range the lists offer, a
/// day, a day compared with `>`, `>=`, `<`, `<=` or `=`, or two days
/// joined by `..`.
fn date(value: &str) -> Option<DateFacet> {
    let days = |from, until| Some(DateFacet::Days { from, until });
    if let Some((first, last)) = value.split_once("..") {
        return days(Some(day(first)?), Some(day(last)?));
    }
    let (comparison, rest) = comparison(value);
    if let Some(comparison) = comparison {
        let day = day(rest)?;
        return match comparison {
            Comparison::Above => days(Some(day.next()), None),
            Comparison::AtLeast => days(Some(day), None),
            Comparison::Below => days(None, Some(day.previous())),
            Comparison::AtMost => days(None, Some(day)),
            Comparison::Equal => days(Some(day), Some(day)),
        };
    }
    if let Some(day) = day(value) {
        return days(Some(day), Some(day));
    }
    Some(match value {
        "today" => DateFacet::Today,
        "yesterday" => DateFacet::Yesterday,
        "this week" => DateFacet::ThisWeek,
        "last week" => DateFacet::LastWeek,
        "this month" => DateFacet::ThisMonth,
        "last month" => DateFacet::LastMonth,
        "this year" => DateFacet::ThisYear,
        "last year" => DateFacet::LastYear,
        _ => return None,
    })
}

/// The size a `size:` value names: a group, a size compared with `>`,
/// `>=`, `<`, `<=` or `=`, two sizes joined by `..`, or one size.
fn size(value: &str) -> Option<SizeFacet> {
    if let Some((first, last)) = value.split_once("..") {
        return Some(SizeFacet::Between(bytes(first)?, bytes(last)?));
    }
    let (comparison, rest) = comparison(value);
    if let Some(comparison) = comparison {
        let bytes = bytes(rest)?;
        return Some(match comparison {
            Comparison::Above => SizeFacet::Larger(bytes),
            Comparison::AtLeast => SizeFacet::Between(bytes, u64::MAX),
            Comparison::Below => SizeFacet::Smaller(bytes),
            Comparison::AtMost => SizeFacet::Between(0, bytes),
            Comparison::Equal => SizeFacet::Between(bytes, bytes),
        });
    }
    if let Some(bytes) = bytes(value) {
        return Some(SizeFacet::Between(bytes, bytes));
    }
    Some(match value {
        "empty" => SizeFacet::Empty,
        "tiny" => SizeFacet::Tiny,
        "small" => SizeFacet::Small,
        "medium" => SizeFacet::Medium,
        "large" => SizeFacet::Large,
        "huge" => SizeFacet::Huge,
        "gigantic" => SizeFacet::Gigantic,
        _ => return None,
    })
}

/// How a typed value compares.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Comparison {
    Above,
    AtLeast,
    Below,
    AtMost,
    Equal,
}

/// The comparison `value` starts with, if any, and the rest.
fn comparison(value: &str) -> (Option<Comparison>, &str) {
    const PREFIXES: [(&str, Comparison); 7] = [
        (">=", Comparison::AtLeast),
        ("\u{2265}", Comparison::AtLeast),
        ("<=", Comparison::AtMost),
        ("\u{2264}", Comparison::AtMost),
        (">", Comparison::Above),
        ("<", Comparison::Below),
        ("=", Comparison::Equal),
    ];
    PREFIXES
        .iter()
        .find_map(|(prefix, comparison)| Some((Some(*comparison), value.strip_prefix(prefix)?)))
        .unwrap_or((None, value))
}

/// The day `text` names: `2026-01-31` (ISO 8601, which Windows reads
/// since Windows 8), or `1/31/2026` or `1/31/26`, month first, as
/// Microsoft's examples are; a two-digit year is of this century.
fn day(text: &str) -> Option<CalendarDay> {
    let number = |digits: &str| -> Option<u32> {
        (!digits.is_empty() && digits.len() <= 4 && digits.chars().all(|digit| digit.is_ascii_digit()))
            .then(|| digits.parse().ok())
            .flatten()
    };
    let year = |digits: &str| -> Option<i32> {
        let value = i32::try_from(number(digits)?).ok()?;
        match digits.len() {
            2 => Some(2000 + value),
            4 => Some(value),
            _ => None,
        }
    };
    let parts: Vec<&str> = text.split('-').collect();
    if let [first, month, day] = parts.as_slice() {
        if first.len() == 4 {
            return CalendarDay::new(year(first)?, number(month)?, number(day)?);
        }
    }
    let parts: Vec<&str> = text.split('/').collect();
    if let [month, day, last] = parts.as_slice() {
        return CalendarDay::new(year(last)?, number(month)?, number(day)?);
    }
    None
}

/// The bytes `text` names: a whole or decimal number, followed by `b`,
/// `kb`, `mb` or `gb` or by nothing, for bytes.
fn bytes(text: &str) -> Option<u64> {
    let split = text
        .find(|character: char| !(character.is_ascii_digit() || character == '.'))
        .unwrap_or(text.len());
    let (number, unit) = text.split_at(split);
    let multiplier: u64 = match unit {
        "" | "b" => 1,
        "kb" | "k" => 1 << 10,
        "mb" | "m" => 1 << 20,
        "gb" | "g" => 1 << 30,
        _ => return None,
    };
    if number.is_empty() || number.starts_with('.') || number.matches('.').count() > 1 {
        return None;
    }
    let (whole, fraction) = number.split_once('.').unwrap_or((number, ""));
    let whole: u64 = whole.parse().ok()?;
    let mut total = whole.checked_mul(multiplier)?;
    if !fraction.is_empty() {
        let digits = u32::try_from(fraction.len()).ok().filter(|digits| *digits <= 9)?;
        let fraction: u64 = fraction.parse().ok()?;
        total = total.checked_add(fraction.checked_mul(multiplier)? / 10_u64.pow(digits))?;
    }
    Some(total)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::search::{CalendarDay, DateFacet, KindFacet, SearchFacets, SizeFacet};

    fn parsed(text: &str) -> (String, SearchFacets) {
        let query = TypedQuery::parse(text);
        (query.words, query.facets)
    }

    /// `kind:`, `datemodified:` (or `modified:`) and `size:` choose the
    /// same options as the strip's lists; the other words are searched
    /// for as before.
    ///
    /// parity: SRCH-044
    #[test]
    fn typed_options_are_taken_out_of_the_words() {
        let kind = |kind| SearchFacets {
            kind,
            ..SearchFacets::default()
        };
        assert_eq!(
            parsed("report kind:folder"),
            ("report".to_owned(), kind(KindFacet::Folders))
        );
        assert_eq!(parsed("kind:folders").1, kind(KindFacet::Folders));
        assert_eq!(parsed("kind:document").1, kind(KindFacet::Documents));
        assert_eq!(parsed("kind:docs").1, kind(KindFacet::Documents));
        assert_eq!(parsed("KIND:Picture").1, kind(KindFacet::Images));
        assert_eq!(parsed("kind:pics").1, kind(KindFacet::Images));
        assert_eq!(parsed("kind:music").1, kind(KindFacet::Audio));
        assert_eq!(parsed("kind:video").1, kind(KindFacet::Videos));
        assert_eq!(parsed("kind:videos").1, kind(KindFacet::Videos));

        let date = |date| SearchFacets {
            date,
            ..SearchFacets::default()
        };
        assert_eq!(parsed("datemodified:today").1, date(DateFacet::Today));
        assert_eq!(parsed("modified:yesterday").1, date(DateFacet::Yesterday));
        assert_eq!(
            parsed("notes datemodified:this week draft"),
            ("notes draft".to_owned(), date(DateFacet::ThisWeek))
        );
        assert_eq!(parsed("modified:last week").1, date(DateFacet::LastWeek));
        assert_eq!(parsed("datemodified:\"last week\"").1, date(DateFacet::LastWeek));
        assert_eq!(parsed("datemodified:this month").1, date(DateFacet::ThisMonth));
        assert_eq!(parsed("modified:last month").1, date(DateFacet::LastMonth));
        assert_eq!(parsed("datemodified:this year").1, date(DateFacet::ThisYear));
        assert_eq!(parsed("datemodified:\"last year\"").1, date(DateFacet::LastYear));

        let size = |size| SearchFacets {
            size,
            ..SearchFacets::default()
        };
        assert_eq!(parsed("size:empty").1, size(SizeFacet::Empty));
        assert_eq!(parsed("size:tiny").1, size(SizeFacet::Tiny));
        assert_eq!(parsed("size:small").1, size(SizeFacet::Small));
        assert_eq!(parsed("size:medium").1, size(SizeFacet::Medium));
        assert_eq!(parsed("size:large").1, size(SizeFacet::Large));
        assert_eq!(parsed("size:huge").1, size(SizeFacet::Huge));
        assert_eq!(parsed("Size:Gigantic").1, size(SizeFacet::Gigantic));
        assert_eq!(parsed("size:>500").1, size(SizeFacet::Larger(500)));
        assert_eq!(parsed("size:<500").1, size(SizeFacet::Smaller(500)));

        assert_eq!(
            parsed("kind:picture size:tiny datemodified:today holiday"),
            (
                "holiday".to_owned(),
                SearchFacets {
                    kind: KindFacet::Images,
                    date: DateFacet::Today,
                    size: SizeFacet::Tiny,
                }
            )
        );
    }

    /// `date:` is the date modified, and dates and sizes may be compared
    /// or given as ranges, with `kb`, `mb` or `gb`, as Advanced Query
    /// Syntax allows
    /// (<https://learn.microsoft.com/en-us/windows/win32/search/-search-3x-advancedquerysyntax>).
    ///
    /// parity: SRCH-047
    #[test]
    fn dates_and_sizes_may_be_compared_or_ranged() {
        let day = |year, month, day| CalendarDay::new(year, month, day).expect("a real day");
        let days = |from, until| DateFacet::Days { from, until };
        assert_eq!(parsed("date:today report").1.date, DateFacet::Today);
        assert_eq!(parsed("date:\"last week\"").1.date, DateFacet::LastWeek);
        let jan31 = day(2026, 1, 31);
        assert_eq!(parsed("date:2026-01-31").1.date, days(Some(jan31), Some(jan31)));
        assert_eq!(
            parsed("datemodified:1/31/2026").1.date,
            days(Some(jan31), Some(jan31))
        );
        assert_eq!(parsed("modified:1/31/26").1.date, days(Some(jan31), Some(jan31)));
        assert_eq!(
            parsed("date:>2026-01-31").1.date,
            days(Some(day(2026, 2, 1)), None)
        );
        assert_eq!(parsed("date:>=2026-01-31").1.date, days(Some(jan31), None));
        assert_eq!(
            parsed("date:<2026-01-31").1.date,
            days(None, Some(day(2026, 1, 30)))
        );
        assert_eq!(parsed("date:<=1/31/2026").1.date, days(None, Some(jan31)));
        assert_eq!(
            parsed("date:11/13/21..11/15/21").1.date,
            days(Some(day(2021, 11, 13)), Some(day(2021, 11, 15)))
        );
        assert_eq!(parsed("date:2026-02-30").0, "date:2026-02-30", "no such day");
        assert_eq!(parsed("date:13/1/2026").0, "date:13/1/2026", "month first");

        let size = |text: &str| parsed(text).1.size;
        assert_eq!(size("size:>1MB"), SizeFacet::Larger(1 << 20));
        assert_eq!(size("size:<1kb"), SizeFacet::Smaller(1024));
        assert_eq!(size("size:>=1kb"), SizeFacet::Between(1024, u64::MAX));
        assert_eq!(size("size:<=2GB"), SizeFacet::Between(0, 2 << 30));
        assert_eq!(size("size:5kb..10kb"), SizeFacet::Between(5 * 1024, 10 * 1024));
        assert_eq!(size("size:=0"), SizeFacet::Between(0, 0));
        assert_eq!(size("size:1.5mb"), SizeFacet::Between(1_572_864, 1_572_864));
        assert_eq!(size("size:>500"), SizeFacet::Larger(500));
        assert_eq!(parsed("size:>1tb").0, "size:>1tb");
    }

    /// Quoted text is a phrase to search for, never an option, and an
    /// option after `NOT` or next to `OR` stays a word, since the options
    /// narrow every result.
    ///
    /// parity: SRCH-047
    #[test]
    fn quoted_text_and_operands_of_operators_stay_words() {
        assert_eq!(
            parsed("\"kind:folder\""),
            ("\"kind:folder\"".to_owned(), SearchFacets::default())
        );
        assert_eq!(
            parsed("\"annual  report\" kind:document"),
            (
                "\"annual  report\"".to_owned(),
                SearchFacets {
                    kind: KindFacet::Documents,
                    ..SearchFacets::default()
                }
            )
        );
        assert_eq!(parsed("report NOT kind:folder").0, "report NOT kind:folder");
        assert_eq!(parsed("kind:music OR kind:video").0, "kind:music OR kind:video");
        assert!(parsed("kind:music OR kind:video").1.is_empty());
    }

    /// What is not a known option stays a word to search for, so a name
    /// such as `kind:of magic.txt` is still found.
    ///
    /// parity: SRCH-044
    #[test]
    fn unknown_options_stay_words() {
        assert_eq!(
            parsed("kind:of magic"),
            ("kind:of magic".to_owned(), SearchFacets::default())
        );
        assert_eq!(
            parsed("size:>lots datemodified:this"),
            ("size:>lots datemodified:this".to_owned(), SearchFacets::default())
        );
        assert_eq!(
            parsed("datemodified:next month"),
            ("datemodified:next month".to_owned(), SearchFacets::default()),
            "only the values the lists offer are read"
        );
        assert_eq!(parsed("notes"), ("notes".to_owned(), SearchFacets::default()));
        assert!(TypedQuery::parse("notes").facets.is_empty());
        assert!(!TypedQuery::parse("kind:folder").facets.is_empty());
    }
}
