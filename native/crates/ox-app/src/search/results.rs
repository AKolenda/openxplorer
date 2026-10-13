// SPDX-License-Identifier: AGPL-3.0-only
//! The rows a cached search shows: the folder's own matches first, then
//! the cached ones, once each and at most 500.
//!
//! Ports `currentFolderMatches` and the merge in `runSearch` in
//! `v2.0.0:desktop/ui/app.js` (SRCH-007, and the 1.1.0 fix for a folder whose
//! only indexed folder is a child: a cached child does not cover its
//! parent, so the listing's own matches come first).

use std::collections::HashSet;

use gtk::gio;
use gtk::prelude::*;
use ox_core::search::{display_path, NamePattern, SearchResults};

use super::report::RESULT_LIMIT;
use crate::folder_view::filter::Visibility;
use crate::folder_view::item::FileItem;

/// The listing a search of the current folder merges in.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Listing<'a> {
    /// The folder's items, as its tab holds them.
    pub items: &'a gio::ListStore,
    /// The folder's URI.
    pub folder: &'a str,
    /// Whether hidden items are shown.
    pub shows_hidden: bool,
}

/// The bounded rows of a cached or live search.
#[derive(Debug, Default)]
pub(crate) struct MergedResults {
    /// The rows, at most [`RESULT_LIMIT`].
    pub items: Vec<FileItem>,
    /// More matched than are shown.
    pub is_truncated: bool,
}

/// The rows of a search for `text` that found `found` in the cache: the
/// matches in `listing`, when the search is of the current folder, then
/// the cached items it does not list, without repeating a URI.
pub(crate) fn merge_results(listing: Option<Listing<'_>>, text: &str, found: SearchResults) -> MergedResults {
    let mut merged = listing
        .map(|listing| listing_matches(listing, text))
        .unwrap_or_default();
    merged.is_truncated |= found.is_truncated;
    if merged.items.len() == RESULT_LIMIT && merged.is_truncated {
        return merged;
    }
    let mut shown: HashSet<String> = merged.items.iter().map(|item| item.entry().uri.clone()).collect();
    for hit in found.hits {
        if shown.insert(hit.uri.clone()) {
            if merged.items.len() == RESULT_LIMIT {
                merged.is_truncated = true;
                break;
            }
            merged.items.push(FileItem::new(hit.into_entry()));
        }
    }
    merged
}

/// The listed items whose names match `text` as the folder filter
/// matches them (SRCH-003, SRCH-004): what a live search of the folder
/// shows before its walk adds the subfolders' matches, at most
/// [`RESULT_LIMIT`] with a flag when more matched.
pub(crate) fn listed_name_matches(listing: Listing<'_>, text: &str) -> MergedResults {
    matching_items(listing, text, "")
}

/// The listed items whose name and folder hold every word of `text`,
/// ignoring case (`currentFolderMatches`); a wildcard word matches the
/// whole name (SRCH-004).
fn listing_matches(listing: Listing<'_>, text: &str) -> MergedResults {
    let folder = display_path(listing.folder).to_lowercase();
    matching_items(listing, text, &folder)
}

/// Stops after the first excess match, retaining at most the displayed
/// rows even when the folder already lists many thousands of matches.
fn matching_items(listing: Listing<'_>, text: &str, folder: &str) -> MergedResults {
    let pattern = NamePattern::new(text);
    let items = listing.items.iter::<FileItem>().filter_map(Result::ok);
    let mut matching = items.filter(|item| {
        let is_listed = listing.shows_hidden || item.visibility() == Visibility::Visible;
        is_listed && pattern.matches_lowercase(item.lowercase_name(), folder)
    });
    let items = matching.by_ref().take(RESULT_LIMIT).collect();
    let is_truncated = matching.next().is_some();
    MergedResults { items, is_truncated }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use ox_core::search::{RootStatus, SearchHit};

    use super::*;
    use crate::folder_view::item::store_of_files;

    const FOLDER: &str = "file:///home/demo/Work";

    fn hit(name: &str) -> SearchHit {
        SearchHit {
            uri: format!("{FOLDER}/2026/{name}"),
            parent_uri: format!("{FOLDER}/2026"),
            name: name.to_owned(),
            is_dir: false,
            size: Some(1),
            modified: Some(1),
            type_label: "Text document".to_owned(),
            kind: ox_core::entry::EntryKind::File,
            is_hidden: false,
            path: format!("/home/demo/Work/2026/{name}"),
            cached_at: None,
            root_updated: None,
            root_status: RootStatus::Ready,
        }
    }

    fn found(hits: Vec<SearchHit>, is_truncated: bool) -> SearchResults {
        SearchResults {
            hits,
            is_truncated,
            limit: RESULT_LIMIT,
            elapsed: Duration::ZERO,
        }
    }

    fn names(merged: &MergedResults) -> Vec<String> {
        merged
            .items
            .iter()
            .map(|item| item.entry().name.clone())
            .collect()
    }

    /// Ported from `runSearch` and `currentFolderMatches` in
    /// `v2.0.0:desktop/ui/app.js`.
    ///
    /// parity: SRCH-007
    #[test]
    fn the_folders_own_matches_come_first_then_cached_ones_once() {
        let store = store_of_files(&["report.txt", "notes.txt"]);
        let listing = Listing {
            items: &store,
            folder: FOLDER,
            shows_hidden: false,
        };
        let listed_again = SearchHit {
            uri: store
                .item(0)
                .and_downcast::<FileItem>()
                .unwrap()
                .entry()
                .uri
                .clone(),
            ..hit("report.txt")
        };

        let merged = merge_results(
            Some(listing),
            "report",
            found(vec![listed_again, hit("report 2026.txt")], false),
        );

        assert_eq!(names(&merged), ["report.txt", "report 2026.txt"]);
        assert!(!merged.is_truncated);
    }

    /// A word may name the folder: `currentFolderMatches` matched the name
    /// and the folder's path together.
    ///
    /// parity: SRCH-007, SRCH-008
    #[test]
    fn words_match_the_name_or_the_folder() {
        let store = store_of_files(&["Plan.txt"]);
        let listing = Listing {
            items: &store,
            folder: "file:///home/demo/Work",
            shows_hidden: false,
        };

        let matches = |text| listing_matches(listing, text).items.len();

        assert_eq!(matches("work plan"), 1);
        assert_eq!(matches("budget"), 0);
    }

    /// parity: SRCH-007, PERF-005
    #[test]
    fn at_most_500_rows_are_shown() {
        let hits: Vec<SearchHit> = (0..501)
            .map(|number| hit(&format!("file {number}.txt")))
            .collect();

        let merged = merge_results(None, "file", found(hits, false));

        assert_eq!(merged.items.len(), RESULT_LIMIT);
        assert!(merged.is_truncated);
    }

    #[test]
    fn listed_matches_report_truncation_only_after_the_first_excess_match() {
        for count in [RESULT_LIMIT - 1, RESULT_LIMIT, RESULT_LIMIT + 1, RESULT_LIMIT * 4] {
            let names: Vec<String> = (0..count).map(|number| format!("file {number}.txt")).collect();
            let refs: Vec<&str> = names.iter().map(String::as_str).collect();
            let store = store_of_files(&refs);
            let listing = Listing {
                items: &store,
                folder: FOLDER,
                shows_hidden: false,
            };

            let live = listed_name_matches(listing, "file");
            let cached = merge_results(Some(listing), "work file", found(Vec::new(), false));

            for matches in [live, cached] {
                assert_eq!(matches.items.len(), count.min(RESULT_LIMIT));
                assert_eq!(matches.is_truncated, count > RESULT_LIMIT);
                assert_eq!(self::names(&matches), names[..count.min(RESULT_LIMIT)]);
            }
        }
    }

    #[test]
    fn cached_duplicates_at_the_limit_do_not_claim_extra_results() {
        let names: Vec<String> = (0..RESULT_LIMIT)
            .map(|number| format!("file {number}.txt"))
            .collect();
        let refs: Vec<&str> = names.iter().map(String::as_str).collect();
        let store = store_of_files(&refs);
        let listing = Listing {
            items: &store,
            folder: FOLDER,
            shows_hidden: false,
        };
        let duplicate = SearchHit {
            uri: store
                .item(0)
                .and_downcast::<FileItem>()
                .unwrap()
                .entry()
                .uri
                .clone(),
            ..hit("file 0.txt")
        };

        let exact = merge_results(Some(listing), "file", found(vec![duplicate.clone()], false));
        assert_eq!(exact.items.len(), RESULT_LIMIT);
        assert!(!exact.is_truncated);

        let extra = merge_results(
            Some(listing),
            "file",
            found(vec![duplicate, hit("file extra.txt")], false),
        );
        assert_eq!(extra.items.len(), RESULT_LIMIT);
        assert!(extra.is_truncated);
        assert_eq!(self::names(&extra), names);
    }

    #[test]
    fn a_search_of_every_cached_folder_shows_only_cached_rows() {
        let merged = merge_results(None, "notes", found(vec![hit("notes.txt")], true));

        assert_eq!(names(&merged), ["notes.txt"]);
        assert!(merged.is_truncated, "the cache found more");
    }
}
