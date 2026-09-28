// SPDX-License-Identifier: AGPL-3.0-only
//! Windows-style type-ahead: typing a filename prefix selects the next match.
//!
//! Ports `desktop/ui/type-select.js` and its tests
//! (`desktop/tests/type_select.test.cjs`). The search runs over the list in
//! display order, wraps around and never activates anything. Apart from
//! `GLib`'s Unicode normalisation this module is plain Rust, so the behaviour
//! is unit-tested without a display; the window's key handling that feeds
//! it lives in `window/input.rs`.

/// How long a typed prefix keeps growing before the next key starts over
/// (`TIMEOUT_MS` in type-select.js).
pub(crate) const TIMEOUT_MS: i64 = 1000;

/// Longest prefix kept, in characters (`MAX_PREFIX` in type-select.js).
const MAX_PREFIX: usize = 256;

/// Case- and composition-insensitive form used for prefix matching
/// (NFC, then lower case), like `fold` in type-select.js.
fn fold(text: &str) -> String {
    let composed = glib::normalize(text, glib::NormalizeMode::DefaultCompose);
    composed.as_str().to_lowercase()
}

/// True when `key` is exactly one printable character, not a named key
/// such as "Enter". Control characters are Unicode category Cc, the
/// `[\u0000-\u001f\u007f-\u009f]` of `isCharacter` in type-select.js.
fn is_character(key: &str) -> bool {
    let mut chars = key.chars();
    let (Some(only), None) = (chars.next(), chars.next()) else {
        return false;
    };
    !only.is_control()
}

/// Where a prefix search starts, relative to the current row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SearchStart {
    /// Just after the current row, so a new prefix or a repeated letter
    /// moves on to the next match.
    AfterCurrent,
    /// At the current row, so a refined or shortened prefix keeps a row
    /// that still matches.
    AtCurrent,
}

impl SearchStart {
    /// The first row searched when row `current` is the current one.
    const fn first_row(self, current: usize) -> usize {
        match self {
            SearchStart::AfterCurrent => current + 1,
            SearchStart::AtCurrent => current,
        }
    }
}

/// Finds the first row whose name starts with `prefix`, searching in
/// display order from `start` and wrapping around (`findPrefix` in
/// type-select.js). `name_at` returns the display name of a row and
/// `count` is the number of rows. Without a current row, or with one past
/// the end, the search starts at the first row.
fn find_prefix<F, S>(
    count: usize,
    name_at: F,
    prefix: &str,
    current: Option<usize>,
    start: SearchStart,
) -> Option<usize>
where
    F: Fn(usize) -> S,
    S: AsRef<str>,
{
    if prefix.is_empty() || count == 0 {
        return None;
    }
    let first_row = match current {
        Some(current) if current < count => start.first_row(current),
        _ => 0,
    };
    let wanted = fold(prefix);
    (0..count)
        .map(|offset| (first_row + offset) % count)
        .find(|&row| fold(name_at(row).as_ref()).starts_with(&wanted))
}

/// Outcome of one key press.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TypeSelect {
    /// The prefix typed so far, shown in the status bar.
    pub text: String,
    /// The row to select, or `None` when nothing matches.
    pub index: Option<usize>,
    /// The same letter was repeated to step through matches.
    pub cycling: bool,
}

/// What a typed character does to the prefix.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PrefixChange {
    /// No prefix is active, so the character starts a new one.
    StartOver,
    /// The character repeats a one-letter prefix, which steps to the next
    /// match rather than seeking "ss".
    Cycle,
    /// The character is appended to the active prefix.
    Extend,
}

impl PrefixChange {
    /// Where the search for the changed prefix starts: only a refined
    /// prefix may keep the current row.
    const fn search_start(self) -> SearchStart {
        match self {
            PrefixChange::Extend => SearchStart::AtCurrent,
            PrefixChange::StartOver | PrefixChange::Cycle => SearchStart::AfterCurrent,
        }
    }
}

/// Accumulates typed characters into a prefix that expires after
/// [`TIMEOUT_MS`] without typing (`Controller` in type-select.js).
#[derive(Debug, Clone, Default)]
pub(crate) struct Controller {
    text: String,
    /// When the last character was typed or erased, in milliseconds.
    last_key_ms: Option<i64>,
}

impl Controller {
    /// The prefix typed so far.
    #[cfg(test)]
    pub fn text(&self) -> &str {
        &self.text
    }

    /// Forgets the typed prefix.
    pub fn reset(&mut self) {
        self.text.clear();
        self.last_key_ms = None;
    }

    /// True while a prefix is being typed at time `now_ms`. A clock that
    /// went backwards counts as expired.
    pub fn active(&self, now_ms: i64) -> bool {
        let Some(last_key_ms) = self.last_key_ms else {
            return false;
        };
        let elapsed = now_ms - last_key_ms;
        !self.text.is_empty() && (0..TIMEOUT_MS).contains(&elapsed)
    }

    /// Adds `key` to the prefix and finds the row to select. Returns `None`
    /// (leaving the prefix unchanged) when `key` is not a printable character.
    ///
    /// `count` and `name_at` describe the rows in display order, and
    /// `current` is the selected row, if any.
    pub fn push<F, S>(
        &mut self,
        key: &str,
        count: usize,
        name_at: F,
        current: Option<usize>,
        now_ms: i64,
    ) -> Option<TypeSelect>
    where
        F: Fn(usize) -> S,
        S: AsRef<str>,
    {
        if !is_character(key) {
            return None;
        }
        let change = self.change_for(key, now_ms);
        if change == PrefixChange::Extend {
            self.append_character(key);
        } else {
            self.text.clear();
            self.text.push_str(key);
        }
        self.last_key_ms = Some(now_ms);
        let index = find_prefix(count, name_at, &self.text, current, change.search_start());
        Some(TypeSelect {
            text: self.text.clone(),
            index,
            cycling: change == PrefixChange::Cycle,
        })
    }

    /// Removes the last typed character. Returns `None` (and resets) when no
    /// prefix is active, so Backspace then has no effect on the selection.
    pub fn backspace<F, S>(
        &mut self,
        count: usize,
        name_at: F,
        current: Option<usize>,
        now_ms: i64,
    ) -> Option<TypeSelect>
    where
        F: Fn(usize) -> S,
        S: AsRef<str>,
    {
        if !self.active(now_ms) {
            self.reset();
            return None;
        }
        self.text.pop();
        self.last_key_ms = Some(now_ms);
        let index = find_prefix(count, name_at, &self.text, current, SearchStart::AtCurrent);
        Some(TypeSelect {
            text: self.text.clone(),
            index,
            cycling: false,
        })
    }

    /// What typing the printable character `key` at `now_ms` does.
    fn change_for(&self, key: &str, now_ms: i64) -> PrefixChange {
        if !self.active(now_ms) {
            return PrefixChange::StartOver;
        }
        if fold(&self.text) == fold(key) {
            PrefixChange::Cycle
        } else {
            PrefixChange::Extend
        }
    }

    /// Appends the one-character `key` unless the prefix already holds
    /// [`MAX_PREFIX`] characters.
    fn append_character(&mut self, key: &str) {
        if self.text.chars().count() < MAX_PREFIX {
            self.text.push_str(key);
        }
    }
}

/// Each test ports the test of the same name in
/// `desktop/tests/type_select.test.cjs`.
#[cfg(test)]
mod tests {
    use super::*;

    /// The display list most tests type into.
    const LIST: [&str; 5] = ["Backups", "Shared documents", "Shipping", "scripts", "work"];

    fn find(names: &[&str], prefix: &str, current: Option<usize>) -> Option<usize> {
        find_prefix(
            names.len(),
            |row| names[row],
            prefix,
            current,
            SearchStart::AfterCurrent,
        )
    }

    fn push(
        controller: &mut Controller,
        key: &str,
        current: Option<usize>,
        now_ms: i64,
    ) -> Option<TypeSelect> {
        controller.push(key, LIST.len(), |row| LIST[row], current, now_ms)
    }

    fn backspace(controller: &mut Controller, current: Option<usize>, now_ms: i64) -> Option<TypeSelect> {
        controller.backspace(LIST.len(), |row| LIST[row], current, now_ms)
    }

    /// The outcome of typing `text` that selects `index`.
    fn found(text: &str, index: Option<usize>) -> TypeSelect {
        TypeSelect {
            text: text.into(),
            index,
            cycling: false,
        }
    }

    /// The outcome of repeating the letter `text` that steps to `index`.
    fn cycled(text: &str, index: Option<usize>) -> TypeSelect {
        TypeSelect {
            cycling: true,
            ..found(text, index)
        }
    }

    /// parity: SEL-020
    #[test]
    fn first_character_finds_the_first_name_when_nothing_is_selected() {
        let found = push(&mut Controller::default(), "s", None, 0).unwrap();
        assert_eq!(found.index, Some(1));
    }

    /// parity: SEL-022
    #[test]
    fn fresh_typing_starts_after_the_current_item() {
        let found = push(&mut Controller::default(), "s", Some(1), 0).unwrap();
        assert_eq!(found.index, Some(2));
    }

    /// parity: SEL-022
    #[test]
    fn search_wraps_around_the_display_list() {
        let found = push(&mut Controller::default(), "s", Some(4), 0).unwrap();
        assert_eq!(found.index, Some(1));
    }

    /// parity: SEL-021, SEL-023
    #[test]
    fn sc_refines_to_scripts() {
        let mut controller = Controller::default();
        let first = push(&mut controller, "S", Some(0), 0).unwrap();
        assert_eq!(
            push(&mut controller, "C", first.index, 100),
            Some(found("SC", Some(3)))
        );
    }

    /// parity: SEL-023
    #[test]
    fn refining_keeps_an_already_matching_selected_row() {
        let mut controller = Controller::default();
        push(&mut controller, "s", Some(2), 0);
        assert_eq!(push(&mut controller, "c", Some(3), 100).unwrap().index, Some(3));
    }

    /// parity: SEL-024
    #[test]
    fn repeated_single_letters_cycle_and_wrap() {
        let mut controller = Controller::default();
        let mut index = None;
        let mut found = Vec::new();
        for now_ms in [0, 100, 200, 300] {
            index = push(&mut controller, "s", index, now_ms).unwrap().index;
            found.push(index);
        }
        assert_eq!(found, [Some(1), Some(2), Some(3), Some(1)]);
    }

    /// parity: SEL-024
    #[test]
    fn repeated_letters_are_case_insensitive() {
        let mut controller = Controller::default();
        push(&mut controller, "s", Some(0), 0);
        assert_eq!(
            push(&mut controller, "S", Some(1), 100),
            Some(cycled("S", Some(2)))
        );
    }

    /// parity: SEL-023
    #[test]
    fn prefix_resets_at_the_timeout_boundary() {
        let mut controller = Controller::default();
        push(&mut controller, "s", Some(0), 0);
        assert_eq!(push(&mut controller, "w", Some(1), TIMEOUT_MS).unwrap().text, "w");
    }

    /// parity: SEL-023
    #[test]
    fn typing_before_timeout_extends_the_prefix() {
        let mut controller = Controller::default();
        push(&mut controller, "s", Some(0), 0);
        assert_eq!(
            push(&mut controller, "c", Some(1), TIMEOUT_MS - 1).unwrap().text,
            "sc"
        );
    }

    /// parity: SEL-025
    #[test]
    fn unmatched_text_is_retained() {
        let mut controller = Controller::default();
        push(&mut controller, "s", Some(0), 0);
        assert_eq!(push(&mut controller, "z", Some(1), 100), Some(found("sz", None)));
    }

    /// parity: SEL-026
    #[test]
    fn backspace_corrects_an_unmatched_prefix() {
        let mut controller = Controller::default();
        push(&mut controller, "s", Some(0), 0);
        push(&mut controller, "z", Some(1), 10);
        let corrected = backspace(&mut controller, Some(1), 20).unwrap();
        assert_eq!(corrected.index, Some(1));
        assert_eq!(controller.text(), "s");
    }

    /// parity: SEL-026
    #[test]
    fn backspace_can_empty_a_prefix_without_choosing_a_row() {
        let mut controller = Controller::default();
        push(&mut controller, "s", Some(0), 0);
        let emptied = backspace(&mut controller, Some(1), 10);
        assert_eq!(emptied, Some(found("", None)));
    }

    /// parity: SEL-026
    #[test]
    fn backspace_after_expiry_has_no_effect() {
        let mut controller = Controller::default();
        push(&mut controller, "s", Some(0), 0);
        assert_eq!(backspace(&mut controller, Some(1), 1100), None);
        assert_eq!(controller.text(), "");
    }

    #[test]
    fn explicit_reset_clears_accumulated_text() {
        let mut controller = Controller::default();
        push(&mut controller, "s", Some(0), 0);
        controller.reset();
        assert!(!controller.active(10));
        assert_eq!(controller.text(), "");
    }

    /// parity: SEL-021
    #[test]
    fn matching_is_by_prefix_not_substring() {
        assert_eq!(find(&["Old scripts", "scripts"], "sc", None), Some(1));
    }

    /// parity: SEL-033
    #[test]
    fn supplied_sort_order_is_preserved() {
        assert_eq!(find(&["scripts-z", "scripts-a"], "sc", None), Some(0));
    }

    /// parity: SEL-022
    #[test]
    fn empty_listings_are_safe() {
        let found = Controller::default().push("s", 0, |_| "", Some(0), 0).unwrap();
        assert_eq!(found.index, None);
    }

    /// parity: SEL-022
    #[test]
    fn out_of_range_anchors_start_at_the_beginning() {
        assert_eq!(find(&LIST, "s", Some(900)), Some(1));
    }

    /// parity: SEL-021
    #[test]
    fn unicode_case_insensitive_selection() {
        assert_eq!(find(&["Документы", "СКРИПТЫ"], "ск", None), Some(1));
    }

    /// parity: SEL-021
    #[test]
    fn canonical_unicode_accents_match_equivalent_names() {
        assert_eq!(find(&["e\u{301}tudes"], "É", None), Some(0));
    }

    /// parity: SEL-021
    #[test]
    fn supplementary_characters_can_be_typed() {
        assert!(is_character("📁"));
        let found = Controller::default()
            .push("📁", 1, |_| "📁 Documents", None, 0)
            .unwrap();
        assert_eq!(found.index, Some(0));
    }

    /// parity: SEL-035
    #[test]
    fn spaces_within_a_filename_are_significant() {
        let mut controller = Controller::default();
        push(&mut controller, "s", Some(0), 0);
        push(&mut controller, "h", Some(1), 10);
        for (now_ms, key) in (20..).zip(["a", "r", "e", "d", " "]) {
            push(&mut controller, key, Some(1), now_ms);
        }
        assert_eq!(controller.text(), "shared ");
        assert_eq!(push(&mut controller, "d", Some(1), 40).unwrap().index, Some(1));
    }

    /// parity: SEL-021
    #[test]
    fn punctuation_and_digits_are_supported() {
        assert_eq!(find(&["0 notes", "_scripts", ".env"], "_", None), Some(1));
        assert!(is_character("."));
        assert!(is_character("1"));
    }

    /// parity: SEL-035
    #[test]
    fn prefix_length_is_bounded() {
        let mut controller = Controller::default();
        for now_ms in 0..1000 {
            // Alternating letters extend the prefix instead of cycling.
            let key = if now_ms % 2 == 1 { "b" } else { "a" };
            controller.push(key, 0, |_| "", None, now_ms);
        }
        assert_eq!(controller.text().chars().count(), MAX_PREFIX);
    }

    /// parity: SEL-023
    #[test]
    fn a_backwards_clock_resets_the_buffer() {
        let mut controller = Controller::default();
        push(&mut controller, "s", None, 100);
        assert_eq!(push(&mut controller, "w", None, 90).unwrap().text, "w");
    }

    /// parity: SEL-029
    #[test]
    fn named_keys_and_control_characters_are_not_prefixes() {
        for key in [
            "Enter",
            "Dead",
            "Backspace",
            "Tab",
            "F2",
            "\n",
            "\u{7f}",
            "\u{85}",
            "",
        ] {
            assert!(!is_character(key), "{key:?}");
        }
    }

    /// parity: SEL-029
    #[test]
    fn invalid_key_input_does_not_alter_the_buffer() {
        let mut controller = Controller::default();
        push(&mut controller, "s", None, 0);
        assert_eq!(push(&mut controller, "Enter", Some(1), 10), None);
        assert_eq!(controller.text(), "s");
    }
}
