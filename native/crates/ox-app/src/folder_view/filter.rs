// SPDX-License-Identifier: AGPL-3.0-only
//! Which items of a folder are shown: hidden files and the search box.
//!
//! Matches the filter in `filtered()` in `desktop/ui/app.js`: hidden items
//! only with "Show hidden files", and every whitespace-separated search term
//! must occur somewhere in the name, ignoring case.

/// The current search text and hidden-file preference.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct FilterState {
    /// The lower-cased search terms; empty while nothing is searched.
    terms: Vec<String>,
    show_hidden: bool,
}

impl FilterState {
    /// Sets the search text; returns true when the terms changed.
    pub fn set_query(&mut self, query: &str) -> bool {
        let terms = query_terms(query);
        let changed = terms != self.terms;
        self.terms = terms;
        changed
    }

    /// Sets whether hidden items are listed; returns true when it changed.
    pub fn set_show_hidden(&mut self, show_hidden: bool) -> bool {
        let changed = show_hidden != self.show_hidden;
        self.show_hidden = show_hidden;
        changed
    }

    /// True when hidden items are listed.
    pub fn shows_hidden(&self) -> bool {
        self.show_hidden
    }

    /// True when a search is active.
    pub fn is_searching(&self) -> bool {
        !self.terms.is_empty()
    }

    /// Whether an item is shown: `lowercase_name` is its lower-cased display
    /// name and `is_hidden` whether GIO marks it hidden.
    pub fn accepts(&self, lowercase_name: &str, is_hidden: bool) -> bool {
        let is_listed = self.show_hidden || !is_hidden;
        is_listed && self.matches_every_term(lowercase_name)
    }

    /// True when every search term occurs in `lowercase_name`.
    fn matches_every_term(&self, lowercase_name: &str) -> bool {
        self.terms
            .iter()
            .all(|term| lowercase_name.contains(term.as_str()))
    }
}

/// Splits search text into lower-cased terms.
fn query_terms(query: &str) -> Vec<String> {
    query
        .to_lowercase()
        .split_whitespace()
        .map(str::to_string)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A name GIO does not mark hidden, for [`FilterState::accepts`].
    const LISTED: bool = false;
    /// A name GIO marks hidden, for [`FilterState::accepts`].
    const HIDDEN: bool = true;

    /// A filter searching for `query` with hidden files not shown.
    fn searching(query: &str) -> FilterState {
        let mut filter = FilterState::default();
        filter.set_query(query);
        filter
    }

    /// parity: SRCH-003
    #[test]
    fn every_term_must_match_somewhere() {
        let filter = searching("  Report  2026 ");
        assert!(filter.accepts("quarterly report 2026.docx", LISTED));
        assert!(!filter.accepts("quarterly report 2025.docx", LISTED));
    }

    /// parity: SRCH-003, VIEW-023
    #[test]
    fn empty_search_shows_everything_visible() {
        let filter = searching("");
        assert!(filter.accepts("anything", LISTED));
        assert!(!filter.accepts(".cache", HIDDEN));
        let mut showing_hidden = searching("");
        showing_hidden.set_show_hidden(true);
        assert!(showing_hidden.accepts(".cache", HIDDEN));
    }

    #[test]
    fn changes_are_reported_only_when_terms_differ() {
        let mut filter = FilterState::default();
        assert!(filter.set_query("brand"));
        assert!(!filter.set_query(" Brand "));
        assert!(filter.is_searching());
        assert!(filter.set_query(""));
        assert!(!filter.is_searching());
    }
}
