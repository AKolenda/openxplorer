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
//! Empty, Tiny, Small, Medium, Large, Huge, Gigantic). Only the values the
//! Search options lists offer are read: any other value, such as `next
//! month` or a date, stays a word to search for, as does every other
//! `name:value`.

use super::facets::{DateFacet, KindFacet, SearchFacets, SizeFacet};

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
        let tokens: Vec<&str> = text.split_whitespace().collect();
        let mut words: Vec<&str> = Vec::new();
        let mut facets = SearchFacets::default();
        let mut index = 0;
        while index < tokens.len() {
            let token = tokens[index];
            let next = tokens.get(index + 1).copied();
            if let Some((option, used)) = read_option(token, next) {
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
        "datemodified" | "modified" => {
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

/// The range a `datemodified:` value names.
fn date(value: &str) -> Option<DateFacet> {
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

/// The size a `size:` value names: a group, or more or fewer bytes.
fn size(value: &str) -> Option<SizeFacet> {
    let bytes = |digits: &str| -> Option<u64> {
        digits
            .chars()
            .all(|digit| digit.is_ascii_digit())
            .then(|| digits.parse().ok())
            .flatten()
    };
    if let Some(digits) = value.strip_prefix('>') {
        return bytes(digits).map(SizeFacet::Larger);
    }
    if let Some(digits) = value.strip_prefix('<') {
        return bytes(digits).map(SizeFacet::Smaller);
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::search::{DateFacet, KindFacet, SearchFacets, SizeFacet};

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
