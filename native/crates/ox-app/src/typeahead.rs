// SPDX-License-Identifier: AGPL-3.0-only
//! Windows-style type-ahead: typing a filename prefix selects the next match.
//!
//! Ports `desktop/ui/type-select.js` and its tests
//! (`desktop/tests/type_select.test.cjs`). The search runs over the list in
//! display order, wraps around and never activates anything. Apart from
//! GLib's Unicode normalisation this module is plain Rust, so the behaviour
//! is unit-tested without a display.

/// How long a typed prefix keeps growing before the next key starts over.
pub const TIMEOUT_MS: i64 = 1000;

/// Longest prefix kept, in characters.
pub const MAX_PREFIX: usize = 256;

/// Case- and composition-insensitive form used for prefix matching
/// (NFC, then lower case), like `fold` in the JavaScript module.
pub fn fold(value: &str) -> String {
    let composed = glib::normalize(value, glib::NormalizeMode::DefaultCompose);
    composed.as_str().to_lowercase()
}

/// True when `key` is exactly one printable character (not a control
/// character and not a named key such as "Enter").
pub fn is_character(key: &str) -> bool {
    let mut chars = key.chars();
    let (Some(only), None) = (chars.next(), chars.next()) else {
        return false;
    };
    !is_control(only)
}

fn is_control(c: char) -> bool {
    matches!(u32::from(c), 0x00..=0x1f | 0x7f..=0x9f)
}

/// Finds the first name starting with `prefix`, searching in display order
/// from just after `current` (or from `current` itself when
/// `include_current` is set) and wrapping around. `name_at` returns the
/// display name of row `index`; `count` is the number of rows.
pub fn find_prefix<F, S>(
    count: usize,
    name_at: F,
    prefix: &str,
    current: Option<usize>,
    include_current: bool,
) -> Option<usize>
where
    F: Fn(usize) -> S,
    S: AsRef<str>,
{
    if prefix.is_empty() || count == 0 {
        return None;
    }
    let start = match current {
        Some(index) if index < count => index + usize::from(!include_current),
        _ => 0,
    };
    let wanted = fold(prefix);
    (0..count)
        .map(|offset| (start + offset) % count)
        .find(|&index| fold(name_at(index).as_ref()).starts_with(&wanted))
}

/// Outcome of one key press.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TypeSelect {
    /// The prefix typed so far, shown in the status bar.
    pub text: String,
    /// The row to select, or `None` when nothing matches.
    pub index: Option<usize>,
    /// The same letter was repeated to step through matches.
    pub cycling: bool,
}

/// Accumulates typed characters into a prefix that expires after a pause.
#[derive(Debug, Clone)]
pub struct Controller {
    timeout_ms: i64,
    text: String,
    last_at: Option<i64>,
}

impl Default for Controller {
    fn default() -> Self {
        Self {
            timeout_ms: TIMEOUT_MS,
            text: String::new(),
            last_at: None,
        }
    }
}

impl Controller {
    /// A controller with a custom timeout; `None` for a zero or negative one.
    pub fn with_timeout(timeout_ms: i64) -> Option<Self> {
        (timeout_ms > 0).then(|| Self {
            timeout_ms,
            ..Self::default()
        })
    }

    /// The prefix typed so far.
    pub fn text(&self) -> &str {
        &self.text
    }

    /// Forgets the typed prefix.
    pub fn reset(&mut self) {
        self.text.clear();
        self.last_at = None;
    }

    /// True while a prefix is being typed at time `now_ms`. A clock that
    /// went backwards counts as expired.
    pub fn active(&self, now_ms: i64) -> bool {
        let Some(last_at) = self.last_at else {
            return false;
        };
        let elapsed = now_ms - last_at;
        !self.text.is_empty() && (0..self.timeout_ms).contains(&elapsed)
    }

    /// Adds `key` to the prefix and finds the row to select. Returns `None`
    /// (leaving the prefix unchanged) when `key` is not a printable character.
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
        let extending = self.active(now_ms);
        // Repeating a single letter cycles through matches rather than
        // seeking "sss".
        let cycling = extending && fold(&self.text) == fold(key);
        if extending && !cycling {
            self.text.push_str(key);
            self.text = self.text.chars().take(MAX_PREFIX).collect();
        } else {
            self.text = key.to_string();
        }
        self.last_at = Some(now_ms);
        let include_current = extending && !cycling;
        let index = find_prefix(count, name_at, &self.text, current, include_current);
        Some(TypeSelect {
            text: self.text.clone(),
            index,
            cycling,
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
        self.last_at = Some(now_ms);
        let index = find_prefix(count, name_at, &self.text, current, true);
        Some(TypeSelect {
            text: self.text.clone(),
            index,
            cycling: false,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LIST: [&str; 5] = ["Backups", "Shared documents", "Shipping", "scripts", "work"];

    fn find(names: &[&str], prefix: &str, current: Option<usize>) -> Option<usize> {
        find_prefix(names.len(), |i| names[i], prefix, current, false)
    }

    fn push(c: &mut Controller, key: &str, current: Option<usize>, now: i64) -> Option<TypeSelect> {
        c.push(key, LIST.len(), |i| LIST[i], current, now)
    }

    fn result(text: &str, index: Option<usize>, cycling: bool) -> Option<TypeSelect> {
        Some(TypeSelect {
            text: text.into(),
            index,
            cycling,
        })
    }

    #[test]
    fn first_character_finds_the_first_name_when_nothing_is_selected() {
        let found = push(&mut Controller::default(), "s", None, 0).unwrap();
        assert_eq!(found.index, Some(1));
    }

    #[test]
    fn fresh_typing_starts_after_the_current_item() {
        let found = push(&mut Controller::default(), "s", Some(1), 0).unwrap();
        assert_eq!(found.index, Some(2));
    }

    #[test]
    fn search_wraps_around_the_display_list() {
        let found = push(&mut Controller::default(), "s", Some(4), 0).unwrap();
        assert_eq!(found.index, Some(1));
    }

    #[test]
    fn sc_refines_to_scripts() {
        let mut c = Controller::default();
        let first = push(&mut c, "S", Some(0), 0).unwrap();
        assert_eq!(push(&mut c, "C", first.index, 100), result("SC", Some(3), false));
    }

    #[test]
    fn refining_keeps_an_already_matching_selected_row() {
        let mut c = Controller::default();
        push(&mut c, "s", Some(2), 0);
        assert_eq!(push(&mut c, "c", Some(3), 100).unwrap().index, Some(3));
    }

    #[test]
    fn repeated_single_letters_cycle_and_wrap() {
        let mut c = Controller::default();
        let mut index = None;
        let mut found = Vec::new();
        for now in [0, 100, 200, 300] {
            index = push(&mut c, "s", index, now).unwrap().index;
            found.push(index);
        }
        assert_eq!(found, [Some(1), Some(2), Some(3), Some(1)]);
    }

    #[test]
    fn repeated_letters_are_case_insensitive() {
        let mut c = Controller::default();
        push(&mut c, "s", Some(0), 0);
        assert_eq!(push(&mut c, "S", Some(1), 100), result("S", Some(2), true));
    }

    #[test]
    fn prefix_resets_at_the_timeout_boundary() {
        let mut c = Controller::default();
        push(&mut c, "s", Some(0), 0);
        assert_eq!(push(&mut c, "w", Some(1), TIMEOUT_MS).unwrap().text, "w");
    }

    #[test]
    fn typing_before_timeout_extends_the_prefix() {
        let mut c = Controller::default();
        push(&mut c, "s", Some(0), 0);
        assert_eq!(push(&mut c, "c", Some(1), TIMEOUT_MS - 1).unwrap().text, "sc");
    }

    #[test]
    fn unmatched_text_is_retained() {
        let mut c = Controller::default();
        push(&mut c, "s", Some(0), 0);
        assert_eq!(push(&mut c, "z", Some(1), 100), result("sz", None, false));
    }

    #[test]
    fn backspace_corrects_an_unmatched_prefix() {
        let mut c = Controller::default();
        push(&mut c, "s", Some(0), 0);
        push(&mut c, "z", Some(1), 10);
        let corrected = c.backspace(LIST.len(), |i| LIST[i], Some(1), 20).unwrap();
        assert_eq!(corrected.index, Some(1));
        assert_eq!(c.text(), "s");
    }

    #[test]
    fn backspace_can_empty_a_prefix_without_choosing_a_row() {
        let mut c = Controller::default();
        push(&mut c, "s", Some(0), 0);
        let emptied = c.backspace(LIST.len(), |i| LIST[i], Some(1), 10);
        assert_eq!(emptied, result("", None, false));
    }

    #[test]
    fn backspace_after_expiry_has_no_effect() {
        let mut c = Controller::default();
        push(&mut c, "s", Some(0), 0);
        assert_eq!(c.backspace(LIST.len(), |i| LIST[i], Some(1), 1100), None);
        assert_eq!(c.text(), "");
    }

    #[test]
    fn explicit_reset_clears_accumulated_text() {
        let mut c = Controller::default();
        push(&mut c, "s", Some(0), 0);
        c.reset();
        assert!(!c.active(10));
        assert_eq!(c.text(), "");
    }

    #[test]
    fn matching_is_by_prefix_not_substring() {
        assert_eq!(find(&["Old scripts", "scripts"], "sc", None), Some(1));
    }

    #[test]
    fn supplied_sort_order_is_preserved() {
        assert_eq!(find(&["scripts-z", "scripts-a"], "sc", None), Some(0));
    }

    #[test]
    fn empty_listings_are_safe() {
        let found = Controller::default().push("s", 0, |_| "", Some(0), 0).unwrap();
        assert_eq!(found.index, None);
    }

    #[test]
    fn out_of_range_anchors_start_at_the_beginning() {
        assert_eq!(find(&LIST, "s", Some(900)), Some(1));
    }

    #[test]
    fn unicode_case_insensitive_selection() {
        assert_eq!(find(&["Документы", "СКРИПТЫ"], "ск", None), Some(1));
    }

    #[test]
    fn canonical_unicode_accents_match_equivalent_names() {
        assert_eq!(find(&["e\u{301}tudes"], "É", None), Some(0));
    }

    #[test]
    fn supplementary_characters_can_be_typed() {
        assert!(is_character("📁"));
        let found = Controller::default()
            .push("📁", 1, |_| "📁 Documents", None, 0)
            .unwrap();
        assert_eq!(found.index, Some(0));
    }

    #[test]
    fn spaces_within_a_filename_are_significant() {
        let mut c = Controller::default();
        push(&mut c, "s", Some(0), 0);
        push(&mut c, "h", Some(1), 10);
        for (i, key) in ["a", "r", "e", "d", " "].iter().enumerate() {
            push(&mut c, key, Some(1), 20 + i as i64);
        }
        assert_eq!(c.text(), "shared ");
        assert_eq!(push(&mut c, "d", Some(1), 40).unwrap().index, Some(1));
    }

    #[test]
    fn punctuation_and_digits_are_supported() {
        assert_eq!(find(&["0 notes", "_scripts", ".env"], "_", None), Some(1));
        assert!(is_character("."));
        assert!(is_character("1"));
    }

    #[test]
    fn prefix_length_is_bounded() {
        let mut c = Controller::default();
        for i in 0..1000 {
            let key = if i % 2 == 1 { "b" } else { "a" };
            c.push(key, 0, |_| "", None, i);
        }
        assert_eq!(c.text().chars().count(), MAX_PREFIX);
    }

    #[test]
    fn a_backwards_clock_resets_the_buffer() {
        let mut c = Controller::default();
        push(&mut c, "s", None, 100);
        assert_eq!(push(&mut c, "w", None, 90).unwrap().text, "w");
    }

    #[test]
    fn named_keys_and_control_characters_are_not_prefixes() {
        for key in ["Enter", "Dead", "Backspace", "Tab", "F2", "\n", "\u{7f}", ""] {
            assert!(!is_character(key), "{key:?}");
        }
    }

    #[test]
    fn invalid_key_input_does_not_alter_the_buffer() {
        let mut c = Controller::default();
        push(&mut c, "s", None, 0);
        assert_eq!(push(&mut c, "Enter", Some(1), 10), None);
        assert_eq!(c.text(), "s");
    }

    #[test]
    fn invalid_timeout_is_rejected() {
        assert!(Controller::with_timeout(0).is_none());
        assert!(Controller::with_timeout(-1).is_none());
        assert!(Controller::with_timeout(500).is_some());
    }
}
