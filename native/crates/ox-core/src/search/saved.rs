// SPDX-License-Identifier: AGPL-3.0-only
//! Searches saved to the sidebar: `saved-searches.json` in the settings
//! directory.
//!
//! New in the native app, from Dolphin's search box, whose save button
//! adds the query as a place named "Search for <text> in <folder>" that
//! runs the search again when opened (SRCH-038). The Python app rewrites
//! `settings.json` with the keys it knows only, so the saved searches live
//! in a file of their own, written with the same private-storage checks and
//! atomic replace as the snapshot sources.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::error::SearchError;
use super::query::MAX_QUERY_CHARS;
use crate::location::{normalise, safe_label};
use crate::private_storage::{
    private_directory, private_file_if_present, read_limited_text, replace_file_atomically,
    PrivateFileOptions,
};

/// The most searches kept; the oldest go first.
pub const MAX_SAVED_SEARCHES: usize = 64;

/// The name of the file inside the settings directory.
const FILE_NAME: &str = "saved-searches.json";

/// The start of the temporary file a save writes before renaming it.
const TEMPORARY_PREFIX: &str = ".saved-searches-";

/// The largest file that is read.
const READ_LIMIT: u64 = 1024 * 1024;

/// A search the user saved.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SavedSearch {
    /// The folder searched, a canonical location.
    pub folder: String,
    /// The words searched for.
    pub text: String,
    /// What the sidebar shows, "Search for <text> in <folder>".
    pub label: String,
}

impl SavedSearch {
    /// A search for `text` in `folder`, whose name is `folder_name`.
    ///
    /// # Errors
    ///
    /// [`SearchError::Location`] when `folder` is not a valid location,
    /// [`SearchError::EmptySearch`] and [`SearchError::QueryTooLong`] for
    /// search text that is blank or too long.
    pub fn new(folder: &str, text: &str, folder_name: &str) -> Result<Self, SearchError> {
        Self::checked(
            folder,
            text,
            &format!("Search for {} in {folder_name}", text.trim()),
        )
    }

    /// A search as given or read back, with its folder, text and label
    /// checked.
    fn checked(folder: &str, text: &str, label: &str) -> Result<Self, SearchError> {
        let folder = normalise(folder)?;
        let text = text.trim();
        if text.is_empty() {
            return Err(SearchError::EmptySearch);
        }
        if text.chars().count() > MAX_QUERY_CHARS {
            return Err(SearchError::QueryTooLong);
        }
        Ok(Self {
            folder,
            text: text.to_owned(),
            label: safe_label(label, "Saved search")?,
        })
    }

    /// Whether this search looks for the same words in the same folder.
    pub fn is_same_search(&self, other: &SavedSearch) -> bool {
        self.folder == other.folder && self.text == other.text
    }
}

/// The saved searches of one settings directory.
#[derive(Debug, Clone)]
pub struct SavedSearches {
    /// The settings directory holding the file.
    directory: PathBuf,
}

impl SavedSearches {
    /// The saved searches in `settings_directory`.
    pub fn new(settings_directory: &Path) -> Self {
        Self {
            directory: settings_directory.to_path_buf(),
        }
    }

    /// The saved searches, oldest first. A missing, refused or malformed
    /// file holds none, and invalid entries are dropped.
    pub fn read(&self) -> Vec<SavedSearch> {
        let path = self.directory.join(FILE_NAME);
        let Ok(Some(file)) = private_file_if_present(&path, PrivateFileOptions::default()) else {
            return Vec::new();
        };
        let Ok(text) = read_limited_text(file, &path, READ_LIMIT) else {
            return Vec::new();
        };
        let saved: Vec<SavedSearch> = serde_json::from_str(&text).unwrap_or_default();
        saved
            .into_iter()
            .take(MAX_SAVED_SEARCHES)
            .filter_map(|search| SavedSearch::checked(&search.folder, &search.text, &search.label).ok())
            .collect()
    }

    /// Adds `search`, replacing an earlier save of the same search, and
    /// returns the searches now saved.
    ///
    /// # Errors
    ///
    /// [`SearchError::Storage`] when the file cannot be saved.
    pub fn add(&self, search: SavedSearch) -> Result<Vec<SavedSearch>, SearchError> {
        let mut searches = self.read();
        searches.retain(|saved| !saved.is_same_search(&search));
        searches.push(search);
        let excess = searches.len().saturating_sub(MAX_SAVED_SEARCHES);
        searches.drain(..excess);
        self.save(&searches)?;
        Ok(searches)
    }

    /// Removes `search` and returns the searches now saved.
    ///
    /// # Errors
    ///
    /// [`SearchError::Storage`] when the file cannot be saved.
    pub fn remove(&self, search: &SavedSearch) -> Result<Vec<SavedSearch>, SearchError> {
        let mut searches = self.read();
        searches.retain(|saved| !saved.is_same_search(search));
        self.save(&searches)?;
        Ok(searches)
    }

    /// Saves `searches` atomically in a private file.
    fn save(&self, searches: &[SavedSearch]) -> Result<(), SearchError> {
        let contents = serde_json::to_vec(searches).expect("saved searches are strings, which serialise");
        private_directory(&self.directory)?;
        replace_file_atomically(&self.directory.join(FILE_NAME), TEMPORARY_PREFIX, &contents)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// parity: SRCH-038
    #[test]
    fn a_saved_search_is_kept_once_and_can_be_removed() {
        let settings = tempfile::tempdir().unwrap();
        let saved = SavedSearches::new(settings.path());
        let report = SavedSearch::new("file:///home/demo/Work", " report ", "Work").unwrap();

        saved.add(report.clone()).unwrap();
        let again = SavedSearch::new("file:///home/demo/Work", "report", "Work (2)").unwrap();
        saved.add(again).unwrap();

        let read = saved.read();
        assert_eq!(read.len(), 1, "the same search is saved once");
        assert_eq!(read[0].label, "Search for report in Work (2)");
        assert_eq!(read[0].text, "report");
        assert!(SavedSearch::new("file:///home/demo/Work", "  ", "Work").is_err());
        saved.remove(&report).unwrap();
        assert!(saved.read().is_empty());
    }
}
