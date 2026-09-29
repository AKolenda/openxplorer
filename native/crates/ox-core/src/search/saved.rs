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

use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::contents::SearchIn;
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
    /// Whether names or names and contents are searched; names in a file
    /// saved before the choice was kept.
    #[serde(default)]
    pub search_in: SearchIn,
    /// Whether every cached folder is searched rather than the folder's
    /// tree.
    #[serde(default)]
    pub all_cached_folders: bool,
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
            search_in: SearchIn::default(),
            all_cached_folders: false,
        })
    }

    /// `read`, as read back from the file, with its fields checked.
    fn checked_copy(read: &SavedSearch) -> Result<Self, SearchError> {
        let checked = Self::checked(&read.folder, &read.text, &read.label)?;
        Ok(Self {
            search_in: read.search_in,
            all_cached_folders: read.all_cached_folders,
            ..checked
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
        self.read_for_change().unwrap_or_default()
    }

    /// The saved searches, for a change that writes them back; invalid
    /// entries are dropped one by one.
    ///
    /// # Errors
    ///
    /// Why a file that is there could not be read, or that it is not a
    /// list, so a change never overwrites searches it could not read.
    fn read_for_change(&self) -> Result<Vec<SavedSearch>, SearchError> {
        let path = self.directory.join(FILE_NAME);
        let Some(file) = private_file_if_present(&path, PrivateFileOptions::default())? else {
            return Ok(Vec::new());
        };
        let text = read_limited_text(file, &path, READ_LIMIT)?;
        let entries: Vec<serde_json::Value> =
            serde_json::from_str(&text).map_err(|error| SearchError::Io {
                path: path.clone(),
                error: io::Error::new(io::ErrorKind::InvalidData, error),
            })?;
        let searches = entries
            .into_iter()
            .filter_map(|entry| serde_json::from_value::<SavedSearch>(entry).ok())
            .filter_map(|search| SavedSearch::checked_copy(&search).ok())
            .take(MAX_SAVED_SEARCHES)
            .collect();
        Ok(searches)
    }

    /// Adds `search`, replacing an earlier save of the same search, and
    /// returns the searches now saved.
    ///
    /// # Errors
    ///
    /// [`SearchError::Storage`] when the file cannot be saved, or the file
    /// there cannot be read.
    pub fn add(&self, search: SavedSearch) -> Result<Vec<SavedSearch>, SearchError> {
        let mut searches = self.read_for_change()?;
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
    /// [`SearchError::Storage`] when the file cannot be saved, or the file
    /// there cannot be read.
    pub fn remove(&self, search: &SavedSearch) -> Result<Vec<SavedSearch>, SearchError> {
        let mut searches = self.read_for_change()?;
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
    use std::fs;

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

    /// An invalid entry is dropped alone, and a file that is not a list
    /// is never overwritten by a change.
    ///
    /// parity: SRCH-038
    #[test]
    fn a_damaged_file_loses_no_search() {
        let settings = tempfile::tempdir().unwrap();
        let saved = SavedSearches::new(settings.path());
        let mut contents = SavedSearch::new("file:///home/demo/Work", "budget", "Work").unwrap();
        contents.search_in = SearchIn::NamesAndContents;
        saved.add(contents.clone()).unwrap();
        let path = settings.path().join(FILE_NAME);
        let text = fs::read_to_string(&path).unwrap();
        fs::write(&path, text.replacen('[', "[{\"folder\": 7},", 1)).unwrap();

        assert_eq!(saved.read(), [contents.clone()], "the options are kept too");

        fs::write(&path, "{ not a list").unwrap();
        let report = SavedSearch::new("file:///home/demo/Work", "report", "Work").unwrap();
        assert!(saved.add(report).is_err());
        assert_eq!(fs::read_to_string(&path).unwrap(), "{ not a list");
    }
}
