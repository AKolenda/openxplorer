// SPDX-License-Identifier: AGPL-3.0-only
//! Searching a folder and its subfolders without the cache, by walking
//! them live.
//!
//! New in the native app, from Dolphin's search of a folder that no
//! desktop index covers (SRCH-035): the Python app filtered such a folder
//! only, and searched subfolders only in its opt-in cache. The walk reads
//! each folder once, level by level from the searched folder, and hands
//! the items whose names match over in batches as it finds them. It never
//! follows a symbolic link, never enters a share or shortcut, skips the
//! kernel's `/proc` and `/sys`, and goes at most 128 levels deep, as a scan
//! of the cache does. A subfolder that cannot be read is left out; only
//! the searched folder itself failing is an error. A search of names and
//! contents also reads the text of the files whose names do not match
//! ([`super::contents`], SRCH-036). The caller runs it off the main
//! thread and cancels it when the search changes.

use std::collections::VecDeque;
use std::mem;

use gio::prelude::*;

use super::contents::{lowercase_text, SearchIn};
use super::error::{check_cancelled, SearchError};
use super::pattern::NamePattern;
use super::root::HiddenItems;
use crate::entry::{entry_from_info, Entry, EntryError, EntryKind, ATTRIBUTES};
use crate::location::normalise;

/// Deepest level walked below the searched folder (`MAX_DEPTH` of a scan).
const MAX_DEPTH: usize = 128;

/// Matches handed over at once, at most; a folder's matches are handed
/// over when it has been read, so they appear as the walk goes.
const BATCH_SIZE: usize = 128;

/// Kernel file systems a walk from `/` does not enter.
const SKIPPED_FOLDERS: [&str; 2] = ["file:///proc", "file:///sys"];

/// One live search.
#[derive(Debug, Clone)]
pub struct LiveSearch {
    /// The folder whose tree is searched.
    pub folder: String,
    /// What a name must match.
    pub pattern: NamePattern,
    /// Whether hidden items are searched and entered.
    pub hidden_items: HiddenItems,
    /// Most matches to find; the walk stops at one more.
    pub limit: usize,
    /// Whether the text of files is searched too.
    pub search_in: SearchIn,
}

impl LiveSearch {
    /// Whether `entry` matches: by name, or, in a search of contents, by
    /// its name and its text together.
    fn matches(&self, entry: &Entry) -> bool {
        let name = entry.name.to_lowercase();
        if self.pattern.matches_lowercase(&name, "") {
            return true;
        }
        self.search_in == SearchIn::NamesAndContents
            && lowercase_text(entry).is_some_and(|text| self.pattern.matches_lowercase(&name, &text))
    }
}

/// How a walk that was not cancelled ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LiveSearchEnd {
    /// More items matched than the limit; the walk stopped early.
    pub is_truncated: bool,
}

/// A folder waiting to be read, and how deep it is.
struct PendingFolder {
    uri: String,
    depth: usize,
}

/// Walks `search.folder` and calls `found` with each batch of matching
/// items, at most `search.limit` in all.
///
/// # Errors
///
/// Why the searched folder could not be read, and
/// [`SearchError::Cancelled`] once `cancellable` is cancelled.
pub fn walk_search(
    search: &LiveSearch,
    cancellable: &gio::Cancellable,
    found: &mut dyn FnMut(Vec<Entry>),
) -> Result<LiveSearchEnd, SearchError> {
    let mut walk = Walk {
        search,
        cancellable,
        found,
        batch: Vec::new(),
        matched: 0,
    };
    let mut pending = VecDeque::from([PendingFolder {
        uri: normalise(&search.folder)?,
        depth: 0,
    }]);
    let mut is_first = true;
    while let Some(folder) = pending.pop_front() {
        check_cancelled(cancellable)?;
        match walk.read(&folder) {
            Ok(Some(subfolders)) => pending.extend(subfolders),
            Ok(None) => return Ok(walk.end(true)),
            Err(error) if is_first || matches!(error, SearchError::Cancelled) => return Err(error),
            // A subfolder that cannot be read is left out.
            Err(_) => {}
        }
        walk.hand_over();
        is_first = false;
    }
    Ok(walk.end(false))
}

/// The state of one walk.
struct Walk<'a> {
    search: &'a LiveSearch,
    cancellable: &'a gio::Cancellable,
    found: &'a mut dyn FnMut(Vec<Entry>),
    batch: Vec<Entry>,
    matched: usize,
}

impl Walk<'_> {
    /// Reads `folder`, keeping its matches. Returns its subfolders to walk,
    /// or `None` once more than the limit matched.
    fn read(&mut self, folder: &PendingFolder) -> Result<Option<Vec<PendingFolder>>, SearchError> {
        let file = gio::File::for_uri(&folder.uri);
        let enumerator = file
            .enumerate_children(ATTRIBUTES, gio::FileQueryInfoFlags::NONE, Some(self.cancellable))
            .map_err(EntryError::from)?;
        let mut subfolders = Vec::new();
        let outcome = loop {
            let info = match enumerator.next_file(Some(self.cancellable)) {
                Ok(Some(info)) => info,
                Ok(None) => break Ok(Some(())),
                Err(error) => break Err(SearchError::from(EntryError::from(error))),
            };
            if self.search.hidden_items == HiddenItems::Skip && info.is_hidden() {
                continue;
            }
            let entry = entry_from_info(&enumerator.child(&info), &info);
            if is_walked(&entry) && folder.depth < MAX_DEPTH {
                subfolders.push(PendingFolder {
                    uri: entry.uri.clone(),
                    depth: folder.depth + 1,
                });
            }
            if self.search.matches(&entry) && !self.keep(entry) {
                break Ok(None);
            }
        };
        // Closing only releases the enumerator.
        let _ = enumerator.close(gio::Cancellable::NONE);
        Ok(outcome?.map(|()| subfolders))
    }

    /// Keeps a match; false once more than the limit matched.
    fn keep(&mut self, entry: Entry) -> bool {
        self.matched += 1;
        if self.matched > self.search.limit {
            return false;
        }
        self.batch.push(entry);
        if self.batch.len() >= BATCH_SIZE {
            self.hand_over();
        }
        true
    }

    /// Hands the matches kept so far over.
    fn hand_over(&mut self) {
        if !self.batch.is_empty() {
            (self.found)(mem::take(&mut self.batch));
        }
    }

    /// Hands the last matches over and says how the walk ended.
    fn end(mut self, is_truncated: bool) -> LiveSearchEnd {
        self.hand_over();
        LiveSearchEnd { is_truncated }
    }
}

/// Whether the walk enters `entry`: a real folder, not a link, a share or
/// shortcut, or a kernel file system.
fn is_walked(entry: &Entry) -> bool {
    let is_real_folder = entry.kind == EntryKind::Directory && !entry.is_symlink && !entry.is_virtual;
    is_real_folder && !SKIPPED_FOLDERS.contains(&entry.uri.as_str())
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::os::unix::fs::symlink;

    use super::*;

    fn search_of(folder: &std::path::Path, text: &str, limit: usize) -> LiveSearch {
        LiveSearch {
            folder: gio::File::for_path(folder).uri().into(),
            pattern: NamePattern::new(text),
            hidden_items: HiddenItems::Skip,
            limit,
            search_in: SearchIn::Names,
        }
    }

    fn names_found(search: &LiveSearch) -> (Vec<String>, LiveSearchEnd) {
        let mut names = Vec::new();
        let end = walk_search(search, &gio::Cancellable::new(), &mut |batch| {
            names.extend(batch.into_iter().map(|entry| entry.name));
        })
        .expect("the folder can be walked");
        names.sort();
        (names, end)
    }

    /// parity: SRCH-035
    #[test]
    fn subfolders_are_searched_without_following_links_or_hidden_folders() {
        let base = tempfile::tempdir().unwrap();
        let root = base.path();
        fs::create_dir_all(root.join("Work/2026")).unwrap();
        fs::create_dir_all(root.join(".cache/report")).unwrap();
        fs::write(root.join("report.txt"), "x").unwrap();
        fs::write(root.join("Work/2026/Report Q1.PDF"), "x").unwrap();
        fs::write(root.join("Work/notes.txt"), "x").unwrap();
        symlink(root.join("Work"), root.join("report link")).unwrap();

        let (names, end) = names_found(&search_of(root, "report", 500));
        let (pdfs, _) = names_found(&search_of(root, "*.pdf", 500));

        assert_eq!(names, ["Report Q1.PDF", "report link", "report.txt"]);
        assert_eq!(pdfs, ["Report Q1.PDF"]);
        assert!(!end.is_truncated);
    }

    /// parity: SRCH-036
    #[test]
    fn a_search_of_contents_finds_files_by_their_text_too() {
        let base = tempfile::tempdir().unwrap();
        fs::create_dir(base.path().join("Work")).unwrap();
        fs::write(base.path().join("Work/minutes.txt"), "The BUDGET was approved").unwrap();
        fs::write(base.path().join("budget.ods"), b"PK\0\0binary").unwrap();
        fs::write(base.path().join("notes.txt"), "nothing here").unwrap();
        let mut search = search_of(base.path(), "budget", 500);

        let (by_name, _) = names_found(&search);
        search.search_in = SearchIn::NamesAndContents;
        let (by_contents, _) = names_found(&search);
        search.pattern = NamePattern::new("*.ods approved");
        let (with_wildcard, _) = names_found(&search);

        assert_eq!(by_name, ["budget.ods"]);
        assert_eq!(by_contents, ["budget.ods", "minutes.txt"]);
        assert!(with_wildcard.is_empty(), "a wildcard word still matches the name");
    }

    #[test]
    fn the_walk_stops_after_the_limit() {
        let base = tempfile::tempdir().unwrap();
        for number in 0..5 {
            fs::write(base.path().join(format!("file {number}.txt")), "x").unwrap();
        }

        let (names, end) = names_found(&search_of(base.path(), "file", 3));

        assert_eq!(names.len(), 3);
        assert!(end.is_truncated);
    }

    #[test]
    fn a_missing_folder_is_an_error_and_a_cancelled_walk_stops() {
        let base = tempfile::tempdir().unwrap();
        let missing = search_of(&base.path().join("gone"), "x", 10);
        assert!(walk_search(&missing, &gio::Cancellable::new(), &mut |_| {}).is_err());

        let cancelled = gio::Cancellable::new();
        cancelled.cancel();
        let outcome = walk_search(&search_of(base.path(), "x", 10), &cancelled, &mut |_| {});
        assert!(matches!(outcome, Err(SearchError::Cancelled)));
    }
}
