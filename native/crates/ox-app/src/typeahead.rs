// SPDX-License-Identifier: AGPL-3.0-only
//! Windows-style type-ahead: typing a filename prefix selects the next match.
//!
//! Ports `desktop/ui/type-select.js` and its tests
//! (`desktop/tests/type_select.test.cjs`). The search runs over the list in
//! display order, wraps around and never activates anything. Apart from
//! `GLib`'s Unicode normalisation this module is plain Rust, so the behaviour
//! is unit-tested without a display; the window's key handling that feeds
//! it lives in `window/input.rs`.

use std::time::Duration;

/// How long a typed prefix keeps growing before the next key starts over
/// (`TIMEOUT_MS` in type-select.js).
pub(crate) const PREFIX_TIMEOUT: Duration = Duration::from_secs(1);

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
    /// The first row searched when row `current` is the current one. The
    /// current row is below the row count, so the row after it still fits
    /// a `u32`; past the last row the search wraps around.
    const fn first_row(self, current: u32) -> u32 {
        match self {
            SearchStart::AfterCurrent => current + 1,
            SearchStart::AtCurrent => current,
        }
    }
}

/// The rows type-ahead searches, in display order.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Rows<F> {
    /// How many rows there are.
    pub count: u32,
    /// Returns the display name of a row.
    pub name_at: F,
    /// The selected row, if any.
    pub current: Option<u32>,
}

impl<F, S> Rows<F>
where
    F: Fn(u32) -> S,
    S: AsRef<str>,
{
    /// The name of `row` in the form prefixes are matched against.
    fn folded_name(&self, row: u32) -> String {
        let name = (self.name_at)(row);
        fold(name.as_ref())
    }

    /// The row a search from `start` begins at. Without a current row, or
    /// with one past the end, it begins at the first row.
    fn first_searched(&self, start: SearchStart) -> u32 {
        match self.current {
            Some(current) if current < self.count => start.first_row(current),
            _ => 0,
        }
    }

    /// Finds the first row whose name starts with `prefix`, searching in
    /// display order from `start` and wrapping around (`findPrefix` in
    /// type-select.js).
    fn find_prefix(&self, prefix: &str, start: SearchStart) -> Option<u32> {
        if prefix.is_empty() {
            return None;
        }
        let first_row = self.first_searched(start);
        let mut wrapped_around = (first_row..self.count).chain(0..first_row);
        let wanted = fold(prefix);
        wrapped_around.find(|&row| self.folded_name(row).starts_with(&wanted))
    }
}

/// What one typed or erased key selects.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PrefixMatch {
    /// The prefix typed so far, shown in the status bar.
    pub prefix: String,
    /// The row to select, or `None` when no name starts with the prefix.
    pub row: Option<u32>,
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
/// [`PREFIX_TIMEOUT`] without typing (`Controller` in type-select.js).
///
/// Times are readings of a monotonic clock, as [`Duration`]s since its
/// start.
#[derive(Debug, Clone, Default)]
pub(crate) struct Controller {
    prefix: String,
    /// When the last character was typed or erased.
    last_key: Option<Duration>,
}

impl Controller {
    /// The prefix typed so far.
    #[cfg(test)]
    pub(crate) fn prefix(&self) -> &str {
        &self.prefix
    }

    /// Forgets the typed prefix.
    pub(crate) fn reset(&mut self) {
        self.prefix.clear();
        self.last_key = None;
    }

    /// True while a prefix is being typed at time `now`: it is not empty
    /// and its last key is less than [`PREFIX_TIMEOUT`] old.
    pub(crate) fn is_active(&self, now: Duration) -> bool {
        let Some(last_key) = self.last_key else {
            return false;
        };
        // A clock that went backwards has no elapsed time and counts as
        // expired.
        let elapsed = now.checked_sub(last_key);
        !self.prefix.is_empty() && elapsed.is_some_and(|elapsed| elapsed < PREFIX_TIMEOUT)
    }

    /// Adds `key` to the prefix at time `now` and finds the row to select
    /// among `rows`. Returns `None` (leaving the prefix unchanged) when
    /// `key` is not a printable character.
    pub(crate) fn push<F, S>(&mut self, key: &str, rows: &Rows<F>, now: Duration) -> Option<PrefixMatch>
    where
        F: Fn(u32) -> S,
        S: AsRef<str>,
    {
        if !is_character(key) {
            return None;
        }
        let change = self.change_for(key, now);
        if change == PrefixChange::Extend {
            self.append_character(key);
        } else {
            self.prefix.clear();
            self.prefix.push_str(key);
        }
        self.last_key = Some(now);
        let row = rows.find_prefix(&self.prefix, change.search_start());
        Some(PrefixMatch {
            prefix: self.prefix.clone(),
            row,
            cycling: change == PrefixChange::Cycle,
        })
    }

    /// Removes the last typed character at time `now` and finds the row to
    /// select among `rows`. Returns `None` (and resets) when no prefix is
    /// active, so Backspace then has no effect on the selection.
    pub(crate) fn backspace<F, S>(&mut self, rows: &Rows<F>, now: Duration) -> Option<PrefixMatch>
    where
        F: Fn(u32) -> S,
        S: AsRef<str>,
    {
        if !self.is_active(now) {
            self.reset();
            return None;
        }
        self.prefix.pop();
        self.last_key = Some(now);
        let row = rows.find_prefix(&self.prefix, SearchStart::AtCurrent);
        Some(PrefixMatch {
            prefix: self.prefix.clone(),
            row,
            cycling: false,
        })
    }

    /// What typing the printable character `key` at `now` does.
    fn change_for(&self, key: &str, now: Duration) -> PrefixChange {
        if !self.is_active(now) {
            return PrefixChange::StartOver;
        }
        if fold(&self.prefix) == fold(key) {
            PrefixChange::Cycle
        } else {
            PrefixChange::Extend
        }
    }

    /// Appends the one-character `key` unless the prefix already holds
    /// [`MAX_PREFIX`] characters.
    fn append_character(&mut self, key: &str) {
        if self.prefix.chars().count() < MAX_PREFIX {
            self.prefix.push_str(key);
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

    /// `milliseconds` after the test clock's start.
    fn millis(milliseconds: u64) -> Duration {
        Duration::from_millis(milliseconds)
    }

    /// The rows of `names`, with row `current` selected.
    fn rows<'a>(names: &'a [&'a str], current: Option<u32>) -> Rows<impl Fn(u32) -> &'a str> {
        Rows {
            count: u32::try_from(names.len()).expect("test lists are short"),
            name_at: |row| names[usize::try_from(row).expect("rows index the list")],
            current,
        }
    }

    fn find(names: &[&str], prefix: &str, current: Option<u32>) -> Option<u32> {
        rows(names, current).find_prefix(prefix, SearchStart::AfterCurrent)
    }

    fn push(
        controller: &mut Controller,
        key: &str,
        current: Option<u32>,
        now: Duration,
    ) -> Option<PrefixMatch> {
        controller.push(key, &rows(&LIST, current), now)
    }

    fn backspace(controller: &mut Controller, current: Option<u32>, now: Duration) -> Option<PrefixMatch> {
        controller.backspace(&rows(&LIST, current), now)
    }

    /// The outcome of typing `prefix` that selects `row`.
    fn found(prefix: &str, row: Option<u32>) -> PrefixMatch {
        PrefixMatch {
            prefix: prefix.into(),
            row,
            cycling: false,
        }
    }

    /// The outcome of repeating the letter `prefix` that steps to `row`.
    fn cycled(prefix: &str, row: Option<u32>) -> PrefixMatch {
        PrefixMatch {
            cycling: true,
            ..found(prefix, row)
        }
    }

    /// parity: SEL-020
    #[test]
    fn first_character_finds_the_first_name_when_nothing_is_selected() {
        let matched = push(&mut Controller::default(), "s", None, millis(0)).unwrap();
        assert_eq!(matched.row, Some(1));
    }

    /// parity: SEL-022
    #[test]
    fn fresh_typing_starts_after_the_current_item() {
        let matched = push(&mut Controller::default(), "s", Some(1), millis(0)).unwrap();
        assert_eq!(matched.row, Some(2));
    }

    /// parity: SEL-022
    #[test]
    fn search_wraps_around_the_display_list() {
        let matched = push(&mut Controller::default(), "s", Some(4), millis(0)).unwrap();
        assert_eq!(matched.row, Some(1));
    }

    /// parity: SEL-021, SEL-023
    #[test]
    fn sc_refines_to_scripts() {
        let mut controller = Controller::default();
        let first = push(&mut controller, "S", Some(0), millis(0)).unwrap();
        assert_eq!(
            push(&mut controller, "C", first.row, millis(100)),
            Some(found("SC", Some(3)))
        );
    }

    /// parity: SEL-023
    #[test]
    fn refining_keeps_an_already_matching_selected_row() {
        let mut controller = Controller::default();
        push(&mut controller, "s", Some(2), millis(0));
        let refined = push(&mut controller, "c", Some(3), millis(100)).unwrap();
        assert_eq!(refined.row, Some(3));
    }

    /// parity: SEL-024
    #[test]
    fn repeated_single_letters_cycle_and_wrap() {
        let mut controller = Controller::default();
        let mut row = None;
        let mut selected_rows = Vec::new();
        for now in [0, 100, 200, 300] {
            row = push(&mut controller, "s", row, millis(now)).unwrap().row;
            selected_rows.push(row);
        }
        assert_eq!(selected_rows, [Some(1), Some(2), Some(3), Some(1)]);
    }

    /// parity: SEL-024
    #[test]
    fn repeated_letters_are_case_insensitive() {
        let mut controller = Controller::default();
        push(&mut controller, "s", Some(0), millis(0));
        assert_eq!(
            push(&mut controller, "S", Some(1), millis(100)),
            Some(cycled("S", Some(2)))
        );
    }

    /// parity: SEL-023
    #[test]
    fn prefix_resets_at_the_timeout_boundary() {
        let mut controller = Controller::default();
        push(&mut controller, "s", Some(0), millis(0));
        let restarted = push(&mut controller, "w", Some(1), PREFIX_TIMEOUT).unwrap();
        assert_eq!(restarted.prefix, "w");
    }

    /// parity: SEL-023
    #[test]
    fn typing_before_timeout_extends_the_prefix() {
        let mut controller = Controller::default();
        push(&mut controller, "s", Some(0), millis(0));
        let just_before_timeout = PREFIX_TIMEOUT.saturating_sub(millis(1));
        let extended = push(&mut controller, "c", Some(1), just_before_timeout).unwrap();
        assert_eq!(extended.prefix, "sc");
    }

    /// parity: SEL-025
    #[test]
    fn unmatched_text_is_retained() {
        let mut controller = Controller::default();
        push(&mut controller, "s", Some(0), millis(0));
        assert_eq!(
            push(&mut controller, "z", Some(1), millis(100)),
            Some(found("sz", None))
        );
    }

    /// parity: SEL-026
    #[test]
    fn backspace_corrects_an_unmatched_prefix() {
        let mut controller = Controller::default();
        push(&mut controller, "s", Some(0), millis(0));
        push(&mut controller, "z", Some(1), millis(10));
        let corrected = backspace(&mut controller, Some(1), millis(20)).unwrap();
        assert_eq!(corrected.row, Some(1));
        assert_eq!(controller.prefix(), "s");
    }

    /// parity: SEL-026
    #[test]
    fn backspace_can_empty_a_prefix_without_choosing_a_row() {
        let mut controller = Controller::default();
        push(&mut controller, "s", Some(0), millis(0));
        let emptied = backspace(&mut controller, Some(1), millis(10));
        assert_eq!(emptied, Some(found("", None)));
    }

    /// parity: SEL-026
    #[test]
    fn backspace_after_expiry_has_no_effect() {
        let mut controller = Controller::default();
        push(&mut controller, "s", Some(0), millis(0));
        assert_eq!(backspace(&mut controller, Some(1), millis(1100)), None);
        assert_eq!(controller.prefix(), "");
    }

    #[test]
    fn explicit_reset_clears_accumulated_text() {
        let mut controller = Controller::default();
        push(&mut controller, "s", Some(0), millis(0));
        controller.reset();
        assert!(!controller.is_active(millis(10)));
        assert_eq!(controller.prefix(), "");
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
        let matched = Controller::default()
            .push("s", &rows(&[], Some(0)), millis(0))
            .unwrap();
        assert_eq!(matched.row, None);
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
        let matched = Controller::default()
            .push("📁", &rows(&["📁 Documents"], None), millis(0))
            .unwrap();
        assert_eq!(matched.row, Some(0));
    }

    /// parity: SEL-035
    #[test]
    fn spaces_within_a_filename_are_significant() {
        let mut controller = Controller::default();
        push(&mut controller, "s", Some(0), millis(0));
        push(&mut controller, "h", Some(1), millis(10));
        for (now, key) in (20..).zip(["a", "r", "e", "d", " "]) {
            push(&mut controller, key, Some(1), millis(now));
        }
        assert_eq!(controller.prefix(), "shared ");
        let with_space = push(&mut controller, "d", Some(1), millis(40)).unwrap();
        assert_eq!(with_space.row, Some(1));
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
        let no_rows = rows(&[], None);
        for now in 0..1000 {
            // Alternating letters extend the prefix instead of cycling.
            let key = if now % 2 == 1 { "b" } else { "a" };
            controller.push(key, &no_rows, millis(now));
        }
        assert_eq!(controller.prefix().chars().count(), MAX_PREFIX);
    }

    /// parity: SEL-023
    #[test]
    fn a_backwards_clock_resets_the_buffer() {
        let mut controller = Controller::default();
        push(&mut controller, "s", None, millis(100));
        let restarted = push(&mut controller, "w", None, millis(90)).unwrap();
        assert_eq!(restarted.prefix, "w");
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
        push(&mut controller, "s", None, millis(0));
        assert_eq!(push(&mut controller, "Enter", Some(1), millis(10)), None);
        assert_eq!(controller.prefix(), "s");
    }
}
