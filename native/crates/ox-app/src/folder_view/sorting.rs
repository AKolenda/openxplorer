// SPDX-License-Identifier: AGPL-3.0-only
//! Sort order of the file list.
//!
//! Folders sort first, then the chosen column. Name keys ignore case and
//! combining accents and compare ASCII digit runs by value ("file 2"
//! before "file 10"). Unlike the web interface's `Intl.Collator`, this
//! ordering is locale-independent. Ties fall back to ascending raw names.

use std::cmp::Ordering;
use std::iter::Peekable;
use std::str::Chars;

/// A sortable column of the details view.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortColumn {
    /// Display name in natural order.
    Name,
    /// Modification timestamp.
    Modified,
    /// Human-readable content type.
    Type,
    /// File size in bytes.
    Size,
}

impl SortColumn {
    /// Every column, in display order.
    pub const ALL: [SortColumn; 4] = [
        SortColumn::Name,
        SortColumn::Modified,
        SortColumn::Type,
        SortColumn::Size,
    ];

    /// Key used in action targets (`win.sort-by::modified`), as in app.js.
    pub const fn key(self) -> &'static str {
        match self {
            SortColumn::Name => "name",
            SortColumn::Modified => "modified",
            SortColumn::Type => "type",
            SortColumn::Size => "size",
        }
    }

    /// The column for an action target key.
    pub fn from_key(key: &str) -> Option<SortColumn> {
        Self::ALL.into_iter().find(|column| column.key() == key)
    }

    /// Column header and Sort menu label.
    pub const fn label(self) -> &'static str {
        match self {
            SortColumn::Name => "Name",
            SortColumn::Modified => "Date modified",
            SortColumn::Type => "Type",
            SortColumn::Size => "Size",
        }
    }
}

/// The folded form of a name used for natural ordering: decomposed, with
/// combining accents removed and lower-cased. Computed once per item.
pub fn sort_key(name: &str) -> String {
    let decomposed = glib::normalize(name, glib::NormalizeMode::Default);
    decomposed
        .chars()
        .filter(|c| !is_combining_mark(*c))
        .flat_map(char::to_lowercase)
        .collect()
}

fn is_combining_mark(c: char) -> bool {
    matches!(
        u32::from(c),
        0x0300..=0x036f | 0x1ab0..=0x1aff | 0x1dc0..=0x1dff | 0x20d0..=0x20ff | 0xfe20..=0xfe2f
    )
}

/// Compares two keys from [`sort_key`] in natural order: runs of ASCII
/// digits compare by numeric value, punctuation sorts before digits and
/// digits before letters.
pub fn natural_cmp(a: &str, b: &str) -> Ordering {
    let mut left = a.chars().peekable();
    let mut right = b.chars().peekable();
    loop {
        let (x, y) = match (left.peek(), right.peek()) {
            (None, None) => return Ordering::Equal,
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(&x), Some(&y)) => (x, y),
        };
        let order = if x.is_ascii_digit() && y.is_ascii_digit() {
            let left_number = take_digits(&mut left);
            let right_number = take_digits(&mut right);
            compare_numbers(&left_number, &right_number)
        } else {
            left.next();
            right.next();
            char_rank(x).cmp(&char_rank(y)).then(x.cmp(&y))
        };
        if order != Ordering::Equal {
            return order;
        }
    }
}

/// Orders two names: natural order of their keys, then the raw names so the
/// order is total and stable.
pub fn compare_names(a_key: &str, a_name: &str, b_key: &str, b_name: &str) -> Ordering {
    natural_cmp(a_key, b_key).then_with(|| a_name.cmp(b_name))
}

fn take_digits(chars: &mut Peekable<Chars<'_>>) -> String {
    let mut digits = String::new();
    while let Some(c) = chars.next_if(char::is_ascii_digit) {
        digits.push(c);
    }
    digits
}

/// Compares digit strings by value without overflowing on long runs.
fn compare_numbers(a: &str, b: &str) -> Ordering {
    let a = a.trim_start_matches('0');
    let b = b.trim_start_matches('0');
    a.len().cmp(&b.len()).then_with(|| a.cmp(b))
}

/// Punctuation and spaces sort first, then digits, then letters.
fn char_rank(c: char) -> u8 {
    if c.is_numeric() {
        1
    } else if c.is_alphabetic() {
        2
    } else {
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sorted(names: &[&str]) -> Vec<String> {
        let mut keyed: Vec<(String, String)> = names
            .iter()
            .map(|name| (sort_key(name), name.to_string()))
            .collect();
        keyed.sort_by(|(ak, an), (bk, bn)| compare_names(ak, an, bk, bn));
        keyed.into_iter().map(|(_, name)| name).collect()
    }

    #[test]
    fn digit_runs_compare_by_value() {
        assert_eq!(
            sorted(&["file 10.txt", "file 2.txt", "file 1.txt"]),
            ["file 1.txt", "file 2.txt", "file 10.txt"]
        );
    }

    #[test]
    fn case_is_ignored() {
        assert_eq!(
            sorted(&["cherry", "Banana", "apple"]),
            ["apple", "Banana", "cherry"]
        );
    }

    #[test]
    fn accents_are_ignored() {
        assert_eq!(sorted(&["ezra", "école", "eagle"]), ["eagle", "école", "ezra"]);
    }

    #[test]
    fn punctuation_sorts_before_digits_and_letters() {
        assert_eq!(
            sorted(&["abc", "0 notes", "_scripts"]),
            ["_scripts", "0 notes", "abc"]
        );
    }

    #[test]
    fn long_numbers_do_not_overflow() {
        let big = "photo 123456789012345678901234567890.jpg";
        assert_eq!(sorted(&[big, "photo 9.jpg"]), ["photo 9.jpg", big]);
    }

    #[test]
    fn equal_keys_fall_back_to_the_raw_name() {
        assert_eq!(sorted(&["a1", "a01"]), ["a01", "a1"]);
        assert_eq!(sorted(&["Report", "report"]), ["Report", "report"]);
    }

    #[test]
    fn columns_round_trip_through_their_keys() {
        for column in SortColumn::ALL {
            assert_eq!(SortColumn::from_key(column.key()), Some(column));
        }
        assert_eq!(SortColumn::from_key("colour"), None);
    }
}
