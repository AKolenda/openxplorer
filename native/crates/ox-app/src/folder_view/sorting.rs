// SPDX-License-Identifier: AGPL-3.0-only
//! Sort order of the file list: the sortable columns, the direction and
//! natural ordering of names.
//!
//! Ports the comparison in `filtered()` in `v2.0.0:desktop/ui/app.js`. Folders
//! sort first, then the chosen column. Name keys ignore case and combining
//! accents and compare ASCII digit runs by value ("file 2" before
//! "file 10"). Unlike the web interface's `Intl.Collator`, this ordering is
//! locale-independent. Ties fall back to ascending raw names.

use std::cmp::Ordering;
use std::iter::Peekable;
use std::str::Chars;

/// A sortable column of the details view.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SortColumn {
    /// Display name in natural order.
    Name,
    /// Modification timestamp.
    Modified,
    /// The folder a search result is in, shown instead of Date modified
    /// while searching (VIEW-042).
    FolderPath,
    /// Human-readable content type.
    Type,
    /// File size in bytes.
    Size,
}

impl SortColumn {
    /// Every column, in display order. A folder shows Date modified and a
    /// search Folder path, in the same place (`columnFields` in app.js).
    pub(crate) const ALL: [SortColumn; 5] = [
        SortColumn::Name,
        SortColumn::Modified,
        SortColumn::FolderPath,
        SortColumn::Type,
        SortColumn::Size,
    ];

    /// The columns the Sort menu offers, as app.js's Sort menu does; a
    /// search sorts by Folder path through its column title.
    pub(crate) const IN_SORT_MENU: [SortColumn; 4] = [
        SortColumn::Name,
        SortColumn::Modified,
        SortColumn::Type,
        SortColumn::Size,
    ];

    /// Key used in action targets (`win.sort::modified`), as in app.js.
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            SortColumn::Name => "name",
            SortColumn::Modified => "modified",
            SortColumn::FolderPath => "parentUri",
            SortColumn::Type => "type",
            SortColumn::Size => "size",
        }
    }

    /// The column for an action target key.
    pub(crate) fn from_key(key: &str) -> Option<SortColumn> {
        Self::ALL.into_iter().find(|column| column.as_str() == key)
    }

    /// Column header and Sort menu label.
    pub(crate) const fn label(self) -> &'static str {
        match self {
            SortColumn::Name => "Name",
            SortColumn::Modified => "Date modified",
            SortColumn::FolderPath => "Folder path",
            SortColumn::Type => "Type",
            SortColumn::Size => "Size",
        }
    }
}

/// Which way the chosen column sorts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SortDirection {
    /// A to Z, oldest first, smallest first.
    Ascending,
    /// Z to A, newest first, largest first.
    Descending,
}

impl SortDirection {
    /// Both directions, ascending first.
    pub(crate) const ALL: [SortDirection; 2] = [SortDirection::Ascending, SortDirection::Descending];

    /// Key used in action targets (`win.direction::descending`).
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            SortDirection::Ascending => "ascending",
            SortDirection::Descending => "descending",
        }
    }

    /// The direction for an action target key.
    pub(crate) fn from_key(key: &str) -> Option<SortDirection> {
        Self::ALL.into_iter().find(|direction| direction.as_str() == key)
    }

    /// CSS class of a column title's sort arrow pointing this way;
    /// `resources/skin/folder-views.css` turns `.sort-caret.ascending` up.
    pub(crate) const fn css_class(self) -> &'static str {
        match self {
            SortDirection::Ascending => "ascending",
            SortDirection::Descending => "descending",
        }
    }

    /// GTK's sort type for this direction.
    pub(crate) fn to_sort_type(self) -> gtk::SortType {
        match self {
            SortDirection::Ascending => gtk::SortType::Ascending,
            SortDirection::Descending => gtk::SortType::Descending,
        }
    }

    /// The direction of GTK's sort type. GTK's enum is open, so anything
    /// but descending counts as ascending.
    pub(crate) fn from_sort_type(sort_type: gtk::SortType) -> SortDirection {
        match sort_type {
            gtk::SortType::Descending => SortDirection::Descending,
            _ => SortDirection::Ascending,
        }
    }
}

/// What the details view sorts by: a column and the way it sorts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct SortOrder {
    /// The column whose values order the items.
    pub column: SortColumn,
    /// Which way that column orders them.
    pub direction: SortDirection,
}

impl SortOrder {
    /// Name ascending: how a new window sorts, and what an unsorted view
    /// reports.
    pub(crate) const DEFAULT: SortOrder = SortOrder {
        column: SortColumn::Name,
        direction: SortDirection::Ascending,
    };
}

/// A name or type label folded for natural ordering: decomposed, with
/// combining accents removed and lower-cased.
///
/// Items compute their keys once, so sorting a large folder does not fold
/// every name on every comparison. Keys compare with
/// [`SortKey::natural_cmp`]; the type keeps raw text out of that
/// comparison.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SortKey(String);

impl SortKey {
    /// The key of `text`.
    pub(crate) fn new(text: &str) -> Self {
        let decomposed = glib::normalize(text, glib::NormalizeMode::Default);
        let folded = decomposed
            .chars()
            .filter(|character| !is_combining_mark(*character))
            .flat_map(char::to_lowercase)
            .collect();
        Self(folded)
    }

    /// Compares two keys in natural order: runs of ASCII digits compare by
    /// numeric value, punctuation sorts before digits and digits before
    /// letters.
    pub(crate) fn natural_cmp(&self, other: &SortKey) -> Ordering {
        natural_cmp(&self.0, &other.0)
    }
}

/// A display name with its [`SortKey`], borrowed from an item.
#[derive(Debug, Clone, Copy)]
pub(crate) struct SortName<'a> {
    /// The folded name.
    pub key: &'a SortKey,
    /// The name as shown.
    pub name: &'a str,
}

/// Orders two names: natural order of their keys, then the raw names so the
/// order is total and stable.
pub(crate) fn compare_names(a: SortName<'_>, b: SortName<'_>) -> Ordering {
    a.key.natural_cmp(b.key).then_with(|| a.name.cmp(b.name))
}

/// True for the combining accents that [`SortKey::new`] drops once a name
/// is decomposed.
fn is_combining_mark(character: char) -> bool {
    matches!(
        u32::from(character),
        0x0300..=0x036f | 0x1ab0..=0x1aff | 0x1dc0..=0x1dff | 0x20d0..=0x20ff | 0xfe20..=0xfe2f
    )
}

/// Natural order of two folded texts, character by character, with each
/// run of ASCII digits compared as one number.
fn natural_cmp(left: &str, right: &str) -> Ordering {
    let mut left = left.chars().peekable();
    let mut right = right.chars().peekable();
    loop {
        let (left_char, right_char) = match (left.peek(), right.peek()) {
            (None, None) => return Ordering::Equal,
            // A text that is the start of the other sorts first.
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(&left_char), Some(&right_char)) => (left_char, right_char),
        };
        let order = if left_char.is_ascii_digit() && right_char.is_ascii_digit() {
            let left_number = take_digits(&mut left);
            let right_number = take_digits(&mut right);
            compare_numbers(&left_number, &right_number)
        } else {
            left.next();
            right.next();
            compare_chars(left_char, right_char)
        };
        if order != Ordering::Equal {
            return order;
        }
    }
}

/// Takes the run of ASCII digits at the front of `chars`.
fn take_digits(chars: &mut Peekable<Chars<'_>>) -> String {
    let mut digits = String::new();
    while let Some(digit) = chars.next_if(char::is_ascii_digit) {
        digits.push(digit);
    }
    digits
}

/// Compares digit strings by value without overflowing on long runs.
fn compare_numbers(left: &str, right: &str) -> Ordering {
    let left = left.trim_start_matches('0');
    let right = right.trim_start_matches('0');
    left.len().cmp(&right.len()).then_with(|| left.cmp(right))
}

/// Compares two characters by [`CharClass`], then by code point.
fn compare_chars(left: char, right: char) -> Ordering {
    let by_class = CharClass::of(left).cmp(&CharClass::of(right));
    by_class.then(left.cmp(&right))
}

/// The classes natural order ranks characters by, first to last.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum CharClass {
    /// Punctuation, spaces and every other character that is neither a
    /// digit nor a letter.
    Punctuation,
    /// Numeric characters, including those outside ASCII.
    Digit,
    /// Alphabetic characters.
    Letter,
}

impl CharClass {
    /// The class of `character`.
    fn of(character: char) -> CharClass {
        if character.is_numeric() {
            CharClass::Digit
        } else if character.is_alphabetic() {
            CharClass::Letter
        } else {
            CharClass::Punctuation
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `names` in the order the Name column sorts them ascending.
    fn sorted(names: &[&str]) -> Vec<String> {
        let mut keyed: Vec<(SortKey, &str)> = names.iter().map(|name| (SortKey::new(name), *name)).collect();
        keyed.sort_by(|(a_key, a_name), (b_key, b_name)| {
            let a = SortName {
                key: a_key,
                name: a_name,
            };
            let b = SortName {
                key: b_key,
                name: b_name,
            };
            compare_names(a, b)
        });
        keyed.into_iter().map(|(_, name)| name.to_owned()).collect()
    }

    /// parity: VIEW-015
    #[test]
    fn digit_runs_compare_by_value() {
        assert_eq!(
            sorted(&["file 10.txt", "file 2.txt", "file 1.txt"]),
            ["file 1.txt", "file 2.txt", "file 10.txt"]
        );
    }

    /// parity: VIEW-015
    #[test]
    fn case_is_ignored() {
        assert_eq!(
            sorted(&["cherry", "Banana", "apple"]),
            ["apple", "Banana", "cherry"]
        );
    }

    /// parity: VIEW-015
    #[test]
    fn accents_are_ignored() {
        assert_eq!(sorted(&["ezra", "école", "eagle"]), ["eagle", "école", "ezra"]);
    }

    /// parity: VIEW-015
    #[test]
    fn punctuation_sorts_before_digits_and_letters() {
        assert_eq!(
            sorted(&["abc", "0 notes", "_scripts"]),
            ["_scripts", "0 notes", "abc"]
        );
    }

    /// parity: VIEW-015
    #[test]
    fn long_numbers_do_not_overflow() {
        let big = "photo 123456789012345678901234567890.jpg";
        assert_eq!(sorted(&[big, "photo 9.jpg"]), ["photo 9.jpg", big]);
    }

    /// parity: VIEW-015
    #[test]
    fn equal_keys_fall_back_to_the_raw_name() {
        assert_eq!(sorted(&["a1", "a01"]), ["a01", "a1"]);
        assert_eq!(sorted(&["Report", "report"]), ["Report", "report"]);
    }

    #[test]
    fn columns_round_trip_through_their_keys() {
        for column in SortColumn::ALL {
            assert_eq!(SortColumn::from_key(column.as_str()), Some(column));
        }
        assert_eq!(SortColumn::from_key("colour"), None);
    }

    #[test]
    fn directions_round_trip_through_their_keys() {
        for direction in SortDirection::ALL {
            assert_eq!(SortDirection::from_key(direction.as_str()), Some(direction));
            let sort_type = direction.to_sort_type();
            assert_eq!(SortDirection::from_sort_type(sort_type), direction);
        }
        assert_eq!(SortDirection::from_key("sideways"), None);
    }
}
