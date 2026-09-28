// SPDX-License-Identifier: AGPL-3.0-only
//! Which items of a folder are shown: hidden files and the search box.
//!
//! Matches the filter in `filtered()` in `desktop/ui/app.js`: hidden items
//! only with "Show hidden files", and every whitespace-separated search term
//! must occur somewhere in the name, ignoring case.

/// Whether GIO marks an item hidden (a dot file, or one named in its
/// folder's `.hidden` file).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Visibility {
    /// Listed whether or not hidden files are shown.
    Visible,
    /// Listed only with "Show hidden files".
    Hidden,
}

/// The current search text and hidden-file preference.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct FilterState {
    /// The lower-cased search terms; empty while nothing is searched.
    terms: Vec<String>,
    show_hidden: bool,
}

impl FilterState {
    /// Sets the search text; returns true when the terms changed.
    pub(crate) fn set_query(&mut self, query: &str) -> bool {
        let terms = query_terms(query);
        let changed = terms != self.terms;
        self.terms = terms;
        changed
    }

    /// Sets whether hidden items are listed; returns true when it changed.
    pub(crate) fn set_show_hidden(&mut self, show_hidden: bool) -> bool {
        let changed = show_hidden != self.show_hidden;
        self.show_hidden = show_hidden;
        changed
    }

    /// True when a search is active.
    #[cfg(test)]
    pub(crate) fn is_searching(&self) -> bool {
        !self.terms.is_empty()
    }

    /// True when an item of `visibility` is listed at all, searched or
    /// not: hidden items only while "Show hidden files" is on.
    pub(crate) fn lists(&self, visibility: Visibility) -> bool {
        self.show_hidden || visibility == Visibility::Visible
    }

    /// Whether an item is shown: it is listed, and `lowercase_name`, its
    /// lower-cased display name, holds every search term.
    pub(crate) fn accepts(&self, lowercase_name: &str, visibility: Visibility) -> bool {
        self.lists(visibility) && self.matches_every_term(lowercase_name)
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
        assert!(filter.accepts("quarterly report 2026.docx", Visibility::Visible));
        assert!(!filter.accepts("quarterly report 2025.docx", Visibility::Visible));
    }

    /// parity: SRCH-003, VIEW-023
    #[test]
    fn empty_search_shows_everything_visible() {
        let filter = searching("");
        assert!(filter.accepts("anything", Visibility::Visible));
        assert!(!filter.accepts(".cache", Visibility::Hidden));
        let mut showing_hidden = searching("");
        showing_hidden.set_show_hidden(true);
        assert!(showing_hidden.accepts(".cache", Visibility::Hidden));
    }

    /// parity: VIEW-023
    #[test]
    fn hidden_items_are_listed_only_while_hidden_files_are_shown() {
        let mut filter = searching("report");
        assert!(
            filter.lists(Visibility::Visible),
            "a search does not unlist items"
        );
        assert!(!filter.lists(Visibility::Hidden));
        filter.set_show_hidden(true);
        assert!(filter.lists(Visibility::Hidden));
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
