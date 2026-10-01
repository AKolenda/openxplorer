// SPDX-License-Identifier: AGPL-3.0-only
//! How scans and live updates write to the cache.
//!
//! Ports `begin`, `put_batch`, `finish`, `replace_directory` and
//! `directories` from `v2.0.0:desktop/search_index.py`.
//!
//! Safety rule "only a complete scan prunes" (SRCH-024): each scan has a
//! [`ScanGeneration`]; entries it did not see are deleted only when it
//! finishes with [`ScanOutcome::Complete`]. A cancelled, offline or partly
//! failed scan keeps the earlier results.

use std::collections::{BTreeMap, HashMap};

use rusqlite::{OptionalExtension, Transaction};

use super::error::SearchError;
use super::index::{begin_immediate, SearchIndex, MAX_ERROR_CHARS};
use super::root::{RootStatus, ScanGeneration};
use super::text::{
    display_path, fold, folder_prefix, is_at_or_below, is_same_folder, parent_uri, truncate_chars, unix_now,
};
use crate::entry::{Entry, EntryKind};
use crate::location::normalise;

/// Longest stored name, in characters.
const MAX_NAME_CHARS: usize = 4096;

/// Inserts an entry, or updates the one a root already has for its URI.
const UPSERT_ENTRY: &str = "
    INSERT INTO entries(root, uri, parent, name, search_text, is_dir, hidden, size, modified, kind, type,
                        generation, seen)
    VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)
    ON CONFLICT(root, uri) DO UPDATE SET
        parent=excluded.parent, name=excluded.name, search_text=excluded.search_text,
        is_dir=excluded.is_dir, hidden=excluded.hidden, size=excluded.size, modified=excluded.modified,
        kind=excluded.kind, type=excluded.type, generation=excluded.generation, seen=excluded.seen";

/// One item of a folder listing, as the index needs it.
///
/// Virtual items (shares and shortcuts, the only ones with a target URI)
/// are never indexed, so no target is kept.
#[derive(Debug, Clone, PartialEq, Eq)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "each flag is a separate GIO attribute that the index rules read on its own"
)]
pub struct ListedItem {
    /// The item's URI.
    pub uri: String,
    /// Display name.
    pub name: String,
    /// What GIO says the item is.
    pub kind: EntryKind,
    /// Opens as a folder.
    pub is_dir: bool,
    /// Hidden by name or by the backend.
    pub is_hidden: bool,
    /// A symbolic link; never indexed or followed.
    pub is_symlink: bool,
    /// A share or shortcut rather than a real item; never indexed.
    pub is_virtual: bool,
    /// Size in bytes; `None` for folders.
    pub size: Option<u64>,
    /// Modification time in seconds since the epoch.
    pub modified: Option<u64>,
    /// The Type column text, for example "PDF document".
    pub type_label: String,
}

impl ListedItem {
    /// Whether the index may store the item at all.
    ///
    /// Safety rule "symlinks and virtual items never enter the index"
    /// (`put_batch` in `search_index.py`, `index_directory` in
    /// `gio_backend.py`): a link or a virtual target could point outside
    /// the folder the user opted in.
    pub(crate) fn is_indexable(&self) -> bool {
        let is_link = self.is_symlink || self.kind == EntryKind::Symlink;
        !is_link && !self.is_virtual
    }
}

impl From<Entry> for ListedItem {
    fn from(entry: Entry) -> Self {
        Self {
            uri: entry.uri,
            name: entry.name,
            kind: entry.kind,
            is_dir: entry.is_dir,
            is_hidden: entry.is_hidden,
            is_symlink: entry.is_symlink,
            is_virtual: entry.is_virtual,
            size: entry.size,
            modified: entry.modified,
            type_label: entry.type_label,
        }
    }
}

/// How a scan ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ScanOutcome {
    /// Every folder was read: entries the scan did not see are pruned.
    Complete,
    /// Cancelled, offline or partly unreadable: earlier results are kept.
    Incomplete {
        /// Why, shown in the Search settings.
        error: String,
    },
}

/// One `entries` row, ready to store.
struct StoredEntry<'a> {
    uri: String,
    parent: String,
    item: &'a ListedItem,
}

impl<'a> StoredEntry<'a> {
    /// The row for `item` in `parent`.
    fn new(uri: String, parent: String, item: &'a ListedItem) -> Self {
        Self { uri, parent, item }
    }

    /// Inserts or updates the row for `root`.
    fn upsert(
        &self,
        transaction: &Transaction<'_>,
        root: &str,
        generation: &str,
        seen: f64,
    ) -> rusqlite::Result<()> {
        let name = truncate_chars(&self.item.name, MAX_NAME_CHARS);
        // Words can match folder names in the path (SRCH-008).
        let search_text = fold(&format!("{name} {}", display_path(&self.parent)));
        let size = self.item.size.and_then(|size| i64::try_from(size).ok());
        #[expect(clippy::cast_precision_loss, reason = "seconds since 1970 fit an f64 exactly")]
        let modified = self.item.modified.unwrap_or_default() as f64;
        transaction.execute(
            UPSERT_ENTRY,
            rusqlite::params![
                root,
                self.uri,
                self.parent,
                name,
                search_text,
                self.item.is_dir,
                self.item.is_hidden,
                size,
                modified,
                self.item.kind.as_str(),
                self.item.type_label,
                generation,
                seen,
            ],
        )?;
        Ok(())
    }
}

impl SearchIndex {
    /// Starts a scan of `root`: a new generation, status "Indexing".
    ///
    /// # Errors
    ///
    /// [`SearchError::NotEnabled`] when `root` is not an enabled root, and
    /// database errors.
    pub(crate) fn begin_scan(&self, root: &str) -> Result<ScanGeneration, SearchError> {
        let generation = ScanGeneration::new();
        let connection = self.connect()?;
        let changed = connection.execute(
            "UPDATE roots SET generation=?1, status=?2, scanned=0, error='' WHERE uri=?3 AND enabled=1",
            (generation.as_str(), RootStatus::Indexing.as_str(), root),
        )?;
        if changed != 1 {
            return Err(SearchError::NotEnabled);
        }
        Ok(generation)
    }

    /// Stores a batch the scan with `generation` read below `root`, and
    /// returns how many entries were stored: none once a newer scan
    /// started or the root was disabled (`put_batch` in Python).
    ///
    /// # Errors
    ///
    /// [`SearchError::Location`] for an item URI that does not normalise,
    /// and database errors.
    pub(crate) fn store_scanned(
        &self,
        root: &str,
        generation: &ScanGeneration,
        items: &[ListedItem],
    ) -> Result<usize, SearchError> {
        let entries = entries_below(root, items)?;
        let mut connection = self.connect()?;
        let transaction = begin_immediate(&mut connection)?;
        // Safety rule "only the current scan writes": a scan that a newer
        // one replaced, or whose root was disabled, stores nothing.
        if current_generation(&transaction, root)?.as_deref() != Some(generation.as_str()) {
            return Ok(0);
        }
        let seen = unix_now();
        for entry in &entries {
            entry.upsert(&transaction, root, generation.as_str(), seen)?;
        }
        let stored = i64::try_from(entries.len()).unwrap_or(i64::MAX);
        transaction.execute("UPDATE roots SET scanned=scanned+?1 WHERE uri=?2", (stored, root))?;
        transaction.commit()?;
        Ok(entries.len())
    }

    /// Ends the scan with `generation`. A superseded scan changes nothing.
    ///
    /// # Errors
    ///
    /// Database errors.
    pub(crate) fn finish_scan(
        &self,
        root: &str,
        generation: &ScanGeneration,
        outcome: &ScanOutcome,
    ) -> Result<(), SearchError> {
        let mut connection = self.connect()?;
        let transaction = begin_immediate(&mut connection)?;
        if current_generation(&transaction, root)?.as_deref() != Some(generation.as_str()) {
            return Ok(());
        }
        match outcome {
            // Safety rule "only a complete scan prunes" (SRCH-024): what
            // this scan did not see is gone only when it read everything.
            ScanOutcome::Complete => {
                transaction.execute(
                    "DELETE FROM entries WHERE root=?1 AND generation<>?2",
                    (root, generation.as_str()),
                )?;
                transaction.execute(
                    "UPDATE roots SET updated=?1, status=?2, error='' WHERE uri=?3",
                    (unix_now(), RootStatus::Ready.as_str(), root),
                )?;
            }
            ScanOutcome::Incomplete { error } => {
                transaction.execute(
                    "UPDATE roots SET status=?1, error=?2 WHERE uri=?3",
                    (
                        RootStatus::Incomplete.as_str(),
                        truncate_chars(error, MAX_ERROR_CHARS),
                        root,
                    ),
                )?;
            }
        }
        transaction.commit()?;
        Ok(())
    }

    /// Makes the cached children of `folder` exactly `items` and returns the
    /// folders among them that were not cached as folders before, for the
    /// caller to read next (`replace_directory` in Python).
    ///
    /// Safety rule "never prune after a failed read": the caller passes
    /// only a complete listing. A deleted or renamed folder, or one that
    /// became a file, loses its cached descendants; the files themselves
    /// are never touched. Items outside `root` or not directly in `folder`
    /// are ignored.
    ///
    /// # Errors
    ///
    /// [`SearchError::Location`] for an item URI that does not normalise,
    /// and database errors.
    pub(crate) fn replace_folder_contents(
        &self,
        root: &str,
        folder: &str,
        items: &[ListedItem],
    ) -> Result<Vec<String>, SearchError> {
        let children = children_of(root, folder, items)?;
        let mut connection = self.connect()?;
        let transaction = begin_immediate(&mut connection)?;
        let Some(generation) = current_generation(&transaction, root)? else {
            return Ok(Vec::new());
        };
        let cached = cached_children(&transaction, root, folder)?;
        for (uri, was_folder) in &cached {
            let still_a_folder = children.get(uri).is_some_and(|item| item.is_dir);
            if !children.contains_key(uri) || (*was_folder && !still_a_folder) {
                delete_subtree(&transaction, root, uri)?;
            }
        }
        let seen = unix_now();
        for (uri, item) in &children {
            let entry = StoredEntry::new(uri.clone(), folder.to_owned(), item);
            entry.upsert(&transaction, root, &generation, seen)?;
        }
        transaction.execute("UPDATE roots SET last_event=?1 WHERE uri=?2", (seen, root))?;
        transaction.commit()?;
        let new_folders = children
            .into_iter()
            .filter(|(uri, item)| item.is_dir && cached.get(uri) != Some(&true))
            .map(|(uri, _)| uri)
            .collect();
        Ok(new_folders)
    }

    /// Up to `limit` cached folders of `root` whose URIs sort after `after`,
    /// for the next batch of network checks (`directories` in Python).
    ///
    /// # Errors
    ///
    /// Database errors.
    pub(crate) fn cached_folders_after(
        &self,
        root: &str,
        after: &str,
        limit: usize,
    ) -> Result<Vec<String>, SearchError> {
        let limit = i64::try_from(limit).unwrap_or(i64::MAX);
        let connection = self.connect()?;
        let mut statement = connection
            .prepare("SELECT uri FROM entries WHERE root=?1 AND is_dir=1 AND uri>?2 ORDER BY uri LIMIT ?3")?;
        let folders = statement.query_map((root, after, limit), |row| row.get(0))?;
        Ok(folders.collect::<rusqlite::Result<_>>()?)
    }
}

/// The rows for the indexable items strictly below `root`.
fn entries_below<'a>(root: &str, items: &'a [ListedItem]) -> Result<Vec<StoredEntry<'a>>, SearchError> {
    let mut entries = Vec::new();
    for item in items.iter().filter(|item| item.is_indexable()) {
        let uri = normalise(&item.uri)?;
        if uri == root || !is_at_or_below(&uri, root) {
            continue;
        }
        let parent = parent_uri(&uri);
        entries.push(StoredEntry::new(uri, parent, item));
    }
    Ok(entries)
}

/// The indexable items directly in `folder` below `root`, by canonical
/// URI. A later item with the same URI replaces an earlier one.
fn children_of<'a>(
    root: &str,
    folder: &str,
    items: &'a [ListedItem],
) -> Result<BTreeMap<String, &'a ListedItem>, SearchError> {
    let mut children = BTreeMap::new();
    for item in items.iter().filter(|item| item.is_indexable()) {
        let uri = normalise(&item.uri)?;
        let is_in_root = uri != root && is_at_or_below(&uri, root);
        if is_in_root && is_same_folder(&parent_uri(&uri), folder) {
            children.insert(uri, item);
        }
    }
    Ok(children)
}

/// The generation of `root` while it is enabled; `None` when it is
/// missing or disabled. An enabled root that was never scanned has the
/// empty generation.
fn current_generation(transaction: &Transaction<'_>, root: &str) -> rusqlite::Result<Option<String>> {
    transaction
        .query_row(
            "SELECT generation FROM roots WHERE uri=?1 AND enabled=1",
            [root],
            |row| row.get(0),
        )
        .optional()
}

/// The cached children of `folder`, with whether each was a folder.
fn cached_children(
    transaction: &Transaction<'_>,
    root: &str,
    folder: &str,
) -> rusqlite::Result<HashMap<String, bool>> {
    let mut statement = transaction.prepare("SELECT uri, is_dir FROM entries WHERE root=?1 AND parent=?2")?;
    let children = statement.query_map((root, folder), |row| Ok((row.get(0)?, row.get(1)?)))?;
    children.collect()
}

/// Deletes the cached entry `uri` and everything cached below it.
fn delete_subtree(transaction: &Transaction<'_>, root: &str, uri: &str) -> rusqlite::Result<()> {
    let prefix = folder_prefix(uri);
    let prefix_chars = i64::try_from(prefix.chars().count()).unwrap_or(i64::MAX);
    transaction.execute(
        "DELETE FROM entries WHERE root=?1 AND (uri=?2 OR substr(uri, 1, ?3)=?4)",
        (root, uri, prefix_chars, &prefix),
    )?;
    Ok(())
}
