// SPDX-License-Identifier: AGPL-3.0-only
//! Which items of a folder are shown: hidden files and the search box.
//!
//! Matches the filter in `filtered()` in `v2.0.0:desktop/ui/app.js`: hidden items
//! only with "Show hidden files", and every whitespace-separated search term
//! must occur somewhere in the name, ignoring case. A term with the
//! wildcards `*`, `?` or `[ ]` must match the whole name instead, as in
//! Dolphin's filter bar (SRCH-004, [`NamePattern`]). While searching,
//! the search options narrow the items by kind and date too (SRCH-037,
//! [`SearchFacets`]). In a window that is choosing files for another
//! application, the dialog's type list narrows the files as well, and a
//! folder dialog lists only folders ([`ChooserListing`], INT-032).

use gtk::glib;
use ox_core::entry::Entry;
use ox_core::integration::FileFilter;
use ox_core::search::{FacetMatcher, NamePattern, SearchFacets};

/// Which items a file dialog lists (INT-032). Folders always pass, so the
/// user can still move between them.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct ChooserListing {
    /// Only folders are listed: a folder dialog, or saving several files.
    pub(crate) folders_only: bool,
    /// The type chosen in the dialog's list, if any.
    pub(crate) filter: Option<FileFilter>,
}

impl ChooserListing {
    /// Whether `entry` is listed.
    pub(crate) fn passes(&self, entry: &Entry) -> bool {
        if entry.is_dir {
            return true;
        }
        if self.folders_only {
            return false;
        }
        self.filter
            .as_ref()
            .is_none_or(|filter| filter.matches(&entry.name, entry.content_type.as_deref()))
    }
}

/// Whether GIO marks an item hidden (a dot file, or one named in its
/// folder's `.hidden` file).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Visibility {
    /// Listed whether or not hidden files are shown.
    Visible,
    /// Listed only with "Show hidden files".
    Hidden,
}

/// The current search text, search options and hidden-file preference.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct FilterState {
    /// The search terms; empty while nothing is searched.
    pattern: NamePattern,
    show_hidden: bool,
    /// The search options as chosen.
    facets: SearchFacets,
    /// `facets` with their date range worked out when they were chosen.
    facet_matcher: FacetMatcher,
    /// The file dialog's narrowing, in a window choosing files.
    chooser: ChooserListing,
}

impl FilterState {
    /// Sets the search text; returns true when the terms changed.
    pub(crate) fn set_query(&mut self, query: &str) -> bool {
        let pattern = NamePattern::new(query);
        let changed = pattern != self.pattern;
        self.pattern = pattern;
        changed
    }

    /// Sets whether hidden items are listed; returns true when it changed.
    pub(crate) fn set_show_hidden(&mut self, show_hidden: bool) -> bool {
        let changed = show_hidden != self.show_hidden;
        self.show_hidden = show_hidden;
        changed
    }

    /// Sets the search options; returns true when they changed. Date
    /// ranges are counted from the local time now.
    pub(crate) fn set_facets(&mut self, facets: SearchFacets) -> bool {
        let changed = facets != self.facets;
        self.facets = facets;
        self.facet_matcher = facets.matcher(&now());
        changed
    }

    /// Whether `entry` passes the search options.
    pub(crate) fn passes_facets(&self, entry: &Entry) -> bool {
        self.facet_matcher.matches(entry)
    }

    /// Sets the file dialog's narrowing; returns true when it changed.
    pub(crate) fn set_chooser(&mut self, chooser: ChooserListing) -> bool {
        let changed = chooser != self.chooser;
        self.chooser = chooser;
        changed
    }

    /// Whether `entry` passes the file dialog's narrowing.
    pub(crate) fn passes_chooser(&self, entry: &Entry) -> bool {
        self.chooser.passes(entry)
    }

    /// True when a search is active.
    #[cfg(test)]
    pub(crate) fn is_searching(&self) -> bool {
        !self.pattern.is_empty()
    }

    /// True when an item of `visibility` is listed at all, searched or
    /// not: hidden items only while "Show hidden files" is on.
    pub(crate) fn lists(&self, visibility: Visibility) -> bool {
        self.show_hidden || visibility == Visibility::Visible
    }

    /// Whether an item is shown: it is listed, and `lowercase_name`, its
    /// lower-cased display name, matches every search term.
    pub(crate) fn accepts(&self, lowercase_name: &str, visibility: Visibility) -> bool {
        self.lists(visibility) && self.pattern.matches_lowercase(lowercase_name, "")
    }
}

/// The local time now; the epoch should the clock be unreadable.
fn now() -> glib::DateTime {
    glib::DateTime::now_local()
        .or_else(|_| glib::DateTime::from_unix_utc(0))
        .expect("the Unix epoch is a valid time")
}

#[cfg(test)]
mod tests {
    use ox_core::integration::FilterPattern;

    use super::*;

    /// An entry called `name`, a folder when `is_dir`.
    fn entry(name: &str, is_dir: bool, content_type: Option<&str>) -> Entry {
        let mut entry = if is_dir {
            crate::test_support::folder_entry(name)
        } else {
            crate::test_support::file_entry(name)
        };
        entry.content_type = content_type.map(str::to_owned);
        entry
    }

    /// parity: INT-032
    #[test]
    fn a_file_dialog_narrows_files_but_never_folders() {
        let images = FileFilter {
            name: "Images".to_owned(),
            patterns: vec![
                FilterPattern::Glob("*.png".to_owned()),
                FilterPattern::MimeType("image/jpeg".to_owned()),
            ],
        };
        let mut filter = FilterState::default();
        assert!(filter.passes_chooser(&entry("notes.txt", false, None)));
        assert!(filter.set_chooser(ChooserListing {
            folders_only: false,
            filter: Some(images)
        }));
        assert!(filter.passes_chooser(&entry("Shot.PNG", false, None)));
        assert!(filter.passes_chooser(&entry("photo", false, Some("image/jpeg"))));
        assert!(!filter.passes_chooser(&entry("notes.txt", false, Some("text/plain"))));
        assert!(filter.passes_chooser(&entry("Pictures", true, None)));
        filter.set_chooser(ChooserListing {
            folders_only: true,
            filter: None,
        });
        assert!(!filter.passes_chooser(&entry("Shot.png", false, None)));
        assert!(filter.passes_chooser(&entry("Pictures", true, None)));
    }

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

    /// parity: SRCH-004
    #[test]
    fn a_wildcard_term_filters_by_the_whole_name() {
        let filter = searching("*.TXT notes");
        assert!(filter.accepts("notes 10.txt", Visibility::Visible));
        assert!(!filter.accepts("notes 10.txt.bak", Visibility::Visible));
        assert!(!filter.accepts("résumé.txt", Visibility::Visible));
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
