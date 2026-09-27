// SPDX-License-Identifier: AGPL-3.0-only
//! Which items of a folder are shown: hidden files and the search box.
//!
//! Matches the filter in `filtered()` in `desktop/ui/app.js`: hidden items
//! only with "Show hidden files", and every whitespace-separated search term
//! must occur somewhere in the name, ignoring case.

/// The current search text and hidden-file preference.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FilterState {
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

    /// True when a search is active.
    pub fn is_searching(&self) -> bool {
        !self.terms.is_empty()
    }

    /// Whether an item is shown. `lower_name` is the lower-cased display name.
    pub fn accepts(&self, lower_name: &str, hidden: bool) -> bool {
        let visible = self.show_hidden || !hidden;
        visible && self.terms.iter().all(|term| lower_name.contains(term.as_str()))
    }
}

/// Splits search text into lower-cased terms.
pub fn query_terms(query: &str) -> Vec<String> {
    query
        .to_lowercase()
        .split_whitespace()
        .map(str::to_string)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state(query: &str, show_hidden: bool) -> FilterState {
        let mut state = FilterState::default();
        state.set_query(query);
        state.set_show_hidden(show_hidden);
        state
    }

    #[test]
    fn every_term_must_match_somewhere() {
        let filter = state("  Report  2026 ", false);
        assert!(filter.accepts("quarterly report 2026.docx", false));
        assert!(!filter.accepts("quarterly report 2025.docx", false));
    }

    #[test]
    fn empty_search_shows_everything_visible() {
        let filter = state("", false);
        assert!(filter.accepts("anything", false));
        assert!(!filter.accepts(".cache", true));
        assert!(state("", true).accepts(".cache", true));
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
