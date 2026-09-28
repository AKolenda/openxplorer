// SPDX-License-Identifier: AGPL-3.0-only
//! The search cache database: opening it safely and managing its roots.
//!
//! Ports `SearchIndex` in `desktop/search_index.py`: `__init__`, `connect`,
//! `roots`, `snapshot`, `configure`, `clear`, `remove` and `monitoring`.
//! Scans write through `scan.rs`, searches read through `query.rs`, and
//! other processes' requests pass through `requests.rs`.
//!
//! Every method opens its own connection, as Python does, so any thread
//! can use a [`SearchIndex`] and a slow share never holds a lock another
//! reader needs. Changes run in immediate transactions: SQLite then
//! serialises writers across threads and processes, which replaces the
//! in-process lock of the Python class.

use std::path::{Path, PathBuf};
use std::time::Duration;

use rusqlite::{Connection, Transaction, TransactionBehavior};

use super::error::SearchError;
use super::root::{CacheStatus, Caching, HiddenItems, IndexRoot, Monitoring, RootStatus, SearchEngine};
use super::schema;
use super::text::{display_path, truncate_chars};
use crate::location::{is_device_location, is_smb_server, normalise, python_strip};
use crate::private_storage::{private_directory, private_file, validate_sqlite_files, PrivateFileOptions};

/// The database file inside the cache directory.
const DATABASE_FILE: &str = "search.sqlite3";

/// The cache directory inside the user's cache directory, shared with the
/// Python app.
const CACHE_DIRECTORY: &str = "winspace";

/// How long a statement waits for another process's write.
const BUSY_TIMEOUT: Duration = Duration::from_secs(8);

/// Longest root label, in characters.
const MAX_LABEL_CHARS: usize = 200;

/// Longest stored error message, in characters.
pub(super) const MAX_ERROR_CHARS: usize = 500;

/// The metadata-only search cache (`SearchIndex` in `search_index.py`).
///
/// A cheap handle: cloning it opens nothing. Every method blocks on SQLite,
/// so the app calls them off the main thread.
#[derive(Debug, Clone)]
pub struct SearchIndex {
    directory: PathBuf,
    database: PathBuf,
    engine: SearchEngine,
}

impl SearchIndex {
    /// Opens the cache in `$XDG_CACHE_HOME/winspace`, where the Python app
    /// keeps it too.
    ///
    /// # Errors
    ///
    /// As [`SearchIndex::open`].
    pub fn open_default() -> Result<Self, SearchError> {
        Self::open(&glib::user_cache_dir().join(CACHE_DIRECTORY))
    }

    /// Opens or creates the cache in `directory` and upgrades an older
    /// database in place (SRCH-034).
    ///
    /// Opening never marks scans interrupted; only the index owner does
    /// (SRCH-025).
    ///
    /// # Errors
    ///
    /// [`SearchError::Refused`] or [`SearchError::Io`] when the directory
    /// or a database file is not private (for example a symlink), and
    /// [`SearchError::Database`] when SQLite fails.
    pub fn open(directory: &Path) -> Result<Self, SearchError> {
        private_directory(directory)?;
        let database = directory.join(DATABASE_FILE);
        // Safety rule "paths are private too": the database is created with
        // mode 0600 before SQLite opens it, because file names can reveal
        // private information even without contents (`search_index.py`).
        let create = PrivateFileOptions {
            create: true,
            writable: true,
            allow_unlinked: false,
        };
        drop(private_file(&database, create)?);
        let mut index = Self {
            directory: directory.to_path_buf(),
            database,
            engine: SearchEngine::SubstringFallback,
        };
        let mut connection = index.connect()?;
        index.engine = schema::prepare(&mut connection)?;
        Ok(index)
    }

    /// The private directory that holds the database.
    pub fn directory(&self) -> &Path {
        &self.directory
    }

    /// The database file.
    pub fn database_path(&self) -> &Path {
        &self.database
    }

    /// Which SQLite matching searches use.
    pub fn engine(&self) -> SearchEngine {
        self.engine
    }

    /// Every root with its entry count, by label (`roots` in Python).
    ///
    /// # Errors
    ///
    /// As [`SearchIndex::open`].
    pub fn roots(&self) -> Result<Vec<IndexRoot>, SearchError> {
        let connection = self.connect()?;
        let mut statement = connection.prepare(
            "SELECT r.*, (SELECT count(*) FROM entries e WHERE e.root=r.uri) AS entry_count
             FROM roots r ORDER BY r.label COLLATE NOCASE",
        )?;
        let roots = statement.query_map((), IndexRoot::from_row)?;
        Ok(roots.collect::<rusqlite::Result<_>>()?)
    }

    /// The root `uri` if it is enabled.
    ///
    /// # Errors
    ///
    /// As [`SearchIndex::open`].
    pub(crate) fn enabled_root(&self, uri: &str) -> Result<Option<IndexRoot>, SearchError> {
        let roots = self.roots()?;
        let root = roots
            .into_iter()
            .find(|root| root.uri == uri && root.is_enabled());
        Ok(root)
    }

    /// Everything the Search cache settings show (`snapshot` in Python).
    ///
    /// # Errors
    ///
    /// As [`SearchIndex::open`].
    pub fn status(&self) -> Result<CacheStatus, SearchError> {
        let roots = self.roots()?;
        let entry_count = roots
            .iter()
            .filter(|root| root.is_enabled())
            .map(|root| root.entry_count)
            .sum();
        Ok(CacheStatus {
            roots,
            entry_count,
            engine: self.engine,
            database: self.database.clone(),
        })
    }

    /// Chooses whether `uri` is indexed (SRCH-019). The user's choice makes
    /// the root theirs: unpinning its folder no longer removes it
    /// (SRCH-040). The caller starts a scan after enabling.
    ///
    /// Disabling deletes the root's cached entries and marks it
    /// "Disabled". An empty `label` becomes the location's display path.
    ///
    /// # Errors
    ///
    /// [`SearchError::Location`] for an invalid address,
    /// [`SearchError::DeviceLocation`] for a phone or camera,
    /// [`SearchError::ServerList`] for an SMB server's share list, and
    /// the errors of [`SearchIndex::open`].
    pub(crate) fn configure(
        &self,
        uri: &str,
        caching: Caching,
        label: &str,
        hidden_items: HiddenItems,
    ) -> Result<(), SearchError> {
        let uri = indexable_root(uri)?;
        let label = root_label(label, &uri);
        let mut connection = self.connect()?;
        let transaction = begin_immediate(&mut connection)?;
        transaction.execute(
            "INSERT INTO roots(uri, label, enabled, include_hidden, pin_added) VALUES(?1, ?2, ?3, ?4, 0)
             ON CONFLICT(uri) DO UPDATE SET enabled=excluded.enabled, label=excluded.label,
                 include_hidden=excluded.include_hidden, pin_added=0",
            (
                &uri,
                &label,
                caching == Caching::Enabled,
                hidden_items == HiddenItems::Include,
            ),
        )?;
        if caching == Caching::Disabled {
            transaction.execute("DELETE FROM entries WHERE root=?1", [&uri])?;
            transaction.execute(
                "UPDATE roots SET status=?1, scanned=0, error='', generation='' WHERE uri=?2",
                (RootStatus::Disabled.as_str(), &uri),
            )?;
        }
        transaction.commit()?;
        Ok(())
    }

    /// Deletes a root's cached entries and marks it "Not indexed" (SRCH-023).
    ///
    /// # Errors
    ///
    /// [`SearchError::Location`] for an invalid address, and the errors of
    /// [`SearchIndex::open`].
    pub(crate) fn clear(&self, uri: &str) -> Result<(), SearchError> {
        let uri = normalise(uri)?;
        let mut connection = self.connect()?;
        let transaction = begin_immediate(&mut connection)?;
        transaction.execute("DELETE FROM entries WHERE root=?1", [&uri])?;
        transaction.execute(
            "UPDATE roots SET status=?1, updated=0, scanned=0, generation='', error='' WHERE uri=?2",
            (RootStatus::NotIndexed.as_str(), &uri),
        )?;
        transaction.commit()?;
        Ok(())
    }

    /// Deletes a root and its cached entries (SRCH-023).
    ///
    /// # Errors
    ///
    /// [`SearchError::Location`] for an invalid address, and the errors of
    /// [`SearchIndex::open`].
    pub(crate) fn remove(&self, uri: &str) -> Result<(), SearchError> {
        let uri = normalise(uri)?;
        let mut connection = self.connect()?;
        let transaction = begin_immediate(&mut connection)?;
        transaction.execute("DELETE FROM entries WHERE root=?1", [&uri])?;
        transaction.execute("DELETE FROM roots WHERE uri=?1", [&uri])?;
        transaction.commit()?;
        Ok(())
    }

    /// Records how changes below `root` reach the cache (`monitoring` in
    /// Python).
    ///
    /// # Errors
    ///
    /// As [`SearchIndex::open`].
    pub(crate) fn set_monitoring(&self, root: &str, monitoring: &Monitoring) -> Result<(), SearchError> {
        let watch_count = i64::try_from(monitoring.watch_count).unwrap_or(i64::MAX);
        let error = truncate_chars(&monitoring.error, MAX_ERROR_CHARS);
        let connection = self.connect()?;
        connection.execute(
            "UPDATE roots SET update_mode=?1, watch_count=?2, watch_error=?3 WHERE uri=?4",
            (monitoring.mode.as_str(), watch_count, error, root),
        )?;
        Ok(())
    }

    /// Marks scans a stopped application left running as interrupted.
    /// Only the index owner calls this; see [`schema::recover_interrupted`].
    ///
    /// # Errors
    ///
    /// As [`SearchIndex::open`].
    pub(crate) fn recover_interrupted(&self) -> Result<(), SearchError> {
        let connection = self.connect()?;
        schema::recover_interrupted(&connection)?;
        Ok(())
    }

    /// Opens a connection after checking the directory and database files.
    ///
    /// Safety rule "every open re-checks private storage" (`connect` in
    /// `search_index.py`): the directory must still be an owned 0700
    /// directory, and the database and its sidecars owned regular files
    /// with one link, so a symlink planted after start-up is refused
    /// before SQLite follows it.
    ///
    /// # Errors
    ///
    /// A storage refusal of the directory or a database file, or the
    /// database's error.
    pub(super) fn connect(&self) -> Result<Connection, SearchError> {
        private_directory(&self.directory)?;
        validate_sqlite_files(&self.database)?;
        let connection = Connection::open(&self.database)?;
        connection.busy_timeout(BUSY_TIMEOUT)?;
        Ok(connection)
    }
}

/// Starts a transaction that takes the write lock at once, so its reads
/// and writes see no other writer in between.
///
/// # Errors
///
/// The database's error, for example when another writer holds the lock
/// past the busy timeout.
pub(super) fn begin_immediate(connection: &mut Connection) -> rusqlite::Result<Transaction<'_>> {
    connection.transaction_with_behavior(TransactionBehavior::Immediate)
}

/// The canonical URI of a folder that may be indexed.
///
/// Safety rules "no devices, no server lists" (SRCH-021): a phone or camera
/// is never crawled in the background, and a server's share list holds no
/// files.
///
/// # Errors
///
/// The location error of an invalid address, [`SearchError::DeviceLocation`]
/// and [`SearchError::ServerList`].
pub(super) fn indexable_root(uri: &str) -> Result<String, SearchError> {
    let uri = normalise(uri)?;
    if is_device_location(&uri) {
        return Err(SearchError::DeviceLocation);
    }
    if is_smb_server(&uri) {
        return Err(SearchError::ServerList);
    }
    Ok(uri)
}

/// `label` without surrounding spaces, or the display path when empty, at
/// most [`MAX_LABEL_CHARS`] characters.
pub(super) fn root_label(label: &str, uri: &str) -> String {
    let stripped = python_strip(label);
    let label = if stripped.is_empty() {
        display_path(uri)
    } else {
        stripped.to_owned()
    };
    truncate_chars(&label, MAX_LABEL_CHARS).to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::search::fixtures::{found_names, listed_file, open_index, root_state, ScannedShare, SHARE};
    use crate::search::root::{RootOrigin, UpdateMode};

    /// parity: SRCH-021
    #[test]
    fn phones_cameras_and_server_share_lists_cannot_be_indexed() {
        let directory = tempfile::tempdir().unwrap();
        let index = open_index(&directory);
        let device_message = "Connected-device search caching is not supported. \
                              Copy files to local storage before indexing them.";

        for device in [
            "mtp://Pixel_7/Internal%20storage",
            "gphoto2://Camera/",
            "afc://iPhone/",
        ] {
            let refused = index.configure(device, Caching::Enabled, "", HiddenItems::Skip);
            assert_eq!(refused.expect_err(device).to_string(), device_message);
        }
        let server = index.configure("smb://nas/", Caching::Enabled, "", HiddenItems::Skip);

        let server_message = "Open a share first. Cache a shared folder, not the server’s share list.";
        assert_eq!(server.expect_err("a server list").to_string(), server_message);
        assert!(index.roots().unwrap().is_empty());
    }

    /// An empty label becomes the display path, a UNC path for SMB; a
    /// label keeps at most 200 characters.
    ///
    /// parity: SRCH-019, SRCH-022
    #[test]
    fn a_root_is_labelled_with_its_path_unless_named() {
        let directory = tempfile::tempdir().unwrap();
        let index = open_index(&directory);
        let long_label = "x".repeat(300);

        index
            .configure(SHARE, Caching::Enabled, "  ", HiddenItems::Include)
            .unwrap();
        index
            .configure(
                "file:///srv/data",
                Caching::Enabled,
                &long_label,
                HiddenItems::Skip,
            )
            .unwrap();

        let share = root_state(&index, SHARE);
        assert_eq!(share.label, "\\\\nas\\share");
        assert_eq!(share.hidden_items, HiddenItems::Include);
        assert_eq!(share.status, RootStatus::NotIndexed);
        assert_eq!(share.update_mode, UpdateMode::NotWatching);
        assert_eq!(share.origin, RootOrigin::User);
        assert!(share.is_enabled());
        assert_eq!(root_state(&index, "file:///srv/data").label.chars().count(), 200);
    }

    /// parity: SRCH-022
    #[test]
    fn the_status_counts_the_entries_of_enabled_roots() {
        let share = ScannedShare::new();
        share.store(&[listed_file(SHARE, "a.pdf"), listed_file(SHARE, "b.pdf")]);
        share
            .index
            .configure(
                "smb://nas/archive",
                Caching::Disabled,
                "archive",
                HiddenItems::Skip,
            )
            .unwrap();

        let status = share.index.status().unwrap();

        assert_eq!(status.entry_count, 2);
        assert_eq!(status.engine, SearchEngine::Trigram);
        assert_eq!(status.database, share.directory.path().join("search.sqlite3"));
        let labels: Vec<&str> = status.roots.iter().map(|root| root.label.as_str()).collect();
        assert_eq!(labels, ["\\\\nas\\share", "archive"]);
    }

    /// parity: SRCH-023
    #[test]
    fn clearing_a_root_keeps_it_as_not_indexed() {
        let share = ScannedShare::new();
        share.store(&[listed_file(SHARE, "bank.pdf")]);

        share.index.clear(SHARE).unwrap();

        assert!(share.found_names("bank").is_empty());
        let root = root_state(&share.index, SHARE);
        assert_eq!(root.status, RootStatus::NotIndexed);
        assert_eq!(root.updated, None);
        assert_eq!(root.generation, None);
        assert!(root.is_enabled());
    }

    /// parity: SRCH-023
    #[test]
    fn removing_a_root_deletes_it_and_its_entries() {
        let share = ScannedShare::new();
        share.store(&[listed_file(SHARE, "bank.pdf")]);

        share.index.remove(SHARE).unwrap();

        assert!(found_names(&share.index, "bank").is_empty());
        assert!(share.index.roots().unwrap().is_empty());
    }

    /// Monitoring errors are cut to 500 characters before they are stored.
    ///
    /// parity: SRCH-022
    #[test]
    fn the_update_mode_and_watch_error_are_recorded() {
        let share = ScannedShare::new();
        let monitoring = Monitoring {
            mode: UpdateMode::LiveWithTimedFallback,
            watch_count: 3,
            error: "e".repeat(600),
        };

        share.index.set_monitoring(SHARE, &monitoring).unwrap();

        let root = root_state(&share.index, SHARE);
        assert_eq!(root.update_mode, UpdateMode::LiveWithTimedFallback);
        assert_eq!(root.watch_count, 3);
        assert_eq!(root.watch_error.map(|error| error.len()), Some(MAX_ERROR_CHARS));
    }
}
