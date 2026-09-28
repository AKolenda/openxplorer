// SPDX-License-Identifier: AGPL-3.0-only
//! The rows a cached search shows: the folder's own matches first, then
//! the cached ones, once each and at most 500.
//!
//! Ports `currentFolderMatches` and the merge in `runSearch` in
//! `desktop/ui/app.js` (SRCH-007, and the 1.1.0 fix for a folder whose
//! only indexed folder is a child: a cached child does not cover its
//! parent, so the listing's own matches come first).

use std::collections::HashSet;

use gtk::gio;
use gtk::prelude::*;
use ox_core::search::{display_path, SearchResults};

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

/// The rows of a cached search.
#[derive(Debug)]
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
    let mut items = listing
        .map(|listing| listing_matches(listing, text))
        .unwrap_or_default();
    let mut shown: HashSet<String> = items.iter().map(|item| item.entry().uri.clone()).collect();
    for hit in found.hits {
        if shown.insert(hit.uri.clone()) {
            items.push(FileItem::new(hit.into_entry()));
        }
    }
    let is_truncated = found.is_truncated || items.len() > RESULT_LIMIT;
    items.truncate(RESULT_LIMIT);
    MergedResults { items, is_truncated }
}

/// The listed items whose name and folder hold every word of `text`,
/// ignoring case (`currentFolderMatches`).
fn listing_matches(listing: Listing<'_>, text: &str) -> Vec<FileItem> {
    let words: Vec<String> = text
        .to_lowercase()
        .split_whitespace()
        .map(str::to_owned)
        .collect();
    let folder = display_path(listing.folder);
    let items = listing.items.iter::<FileItem>().filter_map(Result::ok);
    let matching = items.filter(|item| {
        let is_listed = listing.shows_hidden || item.visibility() == Visibility::Visible;
        is_listed && holds_every_word(&item.entry().name, &folder, &words)
    });
    matching.collect()
}

/// Whether `name` followed by `folder` holds every one of `words`, which
/// are lower case.
fn holds_every_word(name: &str, folder: &str, words: &[String]) -> bool {
    let text = format!("{name} {folder}").to_lowercase();
    words.iter().all(|word| text.contains(word.as_str()))
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
    /// `desktop/ui/app.js`.
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
        assert!(holds_every_word(
            "Plan.txt",
            "/home/demo/Work",
            &["work".into(), "plan".into()]
        ));
        assert!(!holds_every_word(
            "Plan.txt",
            "/home/demo/Work",
            &["budget".into()]
        ));
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
    fn a_search_of_every_cached_folder_shows_only_cached_rows() {
        let merged = merge_results(None, "notes", found(vec![hit("notes.txt")], true));

        assert_eq!(names(&merged), ["notes.txt"]);
        assert!(merged.is_truncated, "the cache found more");
    }
}
