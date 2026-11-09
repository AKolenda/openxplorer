// SPDX-License-Identifier: AGPL-3.0-only
//! The words a search looks for, marked in the names of its results, as
//! Windows 11 File Explorer marks them (SRCH-046).

use gtk::pango;

/// The mark's background: a yellow, as Windows 11 marks a match.
const MARK_BACKGROUND: (u16, u16, u16) = (0xFFFF, 0xD8D8, 0x4D4D);
/// The marked text's colour: black, which reads on the yellow in the light
/// and the dark theme alike.
const MARK_TEXT: (u16, u16, u16) = (0, 0, 0);

/// The attributes that mark each of `words`, lower-cased, wherever it
/// occurs in `name`, ignoring case; `None` when nothing is marked. A name
/// whose lower-cased form has another length in bytes is left unmarked,
/// rather than marked in the wrong place.
pub(super) fn marked_words(name: &str, words: &[String]) -> Option<pango::AttrList> {
    let ranges = found_ranges(name, words);
    if ranges.is_empty() {
        return None;
    }
    let marks = pango::AttrList::new();
    for (start, end) in ranges {
        let (Ok(start), Ok(end)) = (u32::try_from(start), u32::try_from(end)) else {
            continue;
        };
        let (red, green, blue) = MARK_BACKGROUND;
        let mut background = pango::AttrColor::new_background(red, green, blue);
        background.set_start_index(start);
        background.set_end_index(end);
        marks.insert(background);
        let (red, green, blue) = MARK_TEXT;
        let mut text = pango::AttrColor::new_foreground(red, green, blue);
        text.set_start_index(start);
        text.set_end_index(end);
        marks.insert(text);
    }
    Some(marks)
}

/// The byte ranges of `name` where `words` occur, ignoring case, in order
/// and merged where they overlap.
fn found_ranges(name: &str, words: &[String]) -> Vec<(usize, usize)> {
    let lower = name.to_lowercase();
    if lower.len() != name.len() {
        return Vec::new();
    }
    let mut ranges: Vec<(usize, usize)> = words
        .iter()
        .filter(|word| !word.is_empty())
        .flat_map(|word| {
            lower
                .match_indices(word.as_str())
                .map(|(start, found)| (start, start + found.len()))
                .collect::<Vec<_>>()
        })
        .collect();
    ranges.sort_unstable();
    let mut merged: Vec<(usize, usize)> = Vec::new();
    for (start, end) in ranges {
        match merged.last_mut() {
            Some(last) if start <= last.1 => last.1 = last.1.max(end),
            _ => merged.push((start, end)),
        }
    }
    merged
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(list: &[&str]) -> Vec<String> {
        list.iter().map(|word| (*word).to_owned()).collect()
    }

    /// Every place a word occurs is marked, ignoring case; overlapping
    /// words are marked once.
    ///
    /// parity: SRCH-046
    #[test]
    fn each_word_is_marked_wherever_it_occurs() {
        assert_eq!(found_ranges("Notes 2", &words(&["notes"])), [(0, 5)]);
        assert_eq!(
            found_ranges("notes about Notes", &words(&["notes"])),
            [(0, 5), (12, 17)]
        );
        assert_eq!(
            found_ranges("Budget plan", &words(&["plan", "bud"])),
            [(0, 3), (7, 11)]
        );
        assert_eq!(found_ranges("abcd", &words(&["abc", "bcd"])), [(0, 4)]);
        assert!(found_ranges("Budget", &words(&["notes"])).is_empty());
        assert!(found_ranges("Budget", &[]).is_empty());
        assert!(marked_words("Budget", &words(&["notes"])).is_none());
        assert!(marked_words("Notes", &words(&["notes"])).is_some());
        // "İ" lower-cases to a longer string: left unmarked.
        assert!(found_ranges("İstanbul notes", &words(&["notes"])).is_empty());
    }
}
