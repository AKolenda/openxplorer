// SPDX-License-Identifier: AGPL-3.0-only
//! The database layout, shared with the Python app, and its upgrades.
//!
//! Ports the schema of `SearchIndex.__init__` in `desktop/search_index.py`.
//! Both apps open the same `search.sqlite3`, so the tables, columns and
//! stored words are Python's. The additions for pinned folders (SRCH-040),
//! the `roots.pin_added` column with a default and the `index_migrations`
//! table, are ones the Python app reads past.

use rusqlite::{Connection, ErrorCode, Transaction, TransactionBehavior};

use super::root::{RootStatus, SearchEngine};

/// The tables and indexes of the first release.
const TABLES: &str = "
    CREATE TABLE IF NOT EXISTS roots(
        uri TEXT PRIMARY KEY, label TEXT NOT NULL, enabled INTEGER NOT NULL DEFAULT 1,
        status TEXT NOT NULL DEFAULT 'Not indexed', updated REAL NOT NULL DEFAULT 0,
        scanned INTEGER NOT NULL DEFAULT 0, error TEXT NOT NULL DEFAULT '',
        generation TEXT NOT NULL DEFAULT '', include_hidden INTEGER NOT NULL DEFAULT 0);
    CREATE TABLE IF NOT EXISTS entries(
        id INTEGER PRIMARY KEY, root TEXT NOT NULL, uri TEXT NOT NULL,
        parent TEXT NOT NULL, name TEXT NOT NULL, search_text TEXT NOT NULL,
        is_dir INTEGER NOT NULL, hidden INTEGER NOT NULL, size INTEGER,
        modified REAL NOT NULL, kind TEXT NOT NULL, type TEXT NOT NULL,
        generation TEXT NOT NULL, seen REAL NOT NULL,
        UNIQUE(root,uri));
    CREATE INDEX IF NOT EXISTS entries_root_uri ON entries(root,uri);
    CREATE INDEX IF NOT EXISTS entries_parent ON entries(parent);
    CREATE TABLE IF NOT EXISTS index_requests (key TEXT PRIMARY KEY, kind TEXT, uri TEXT, created REAL);
    CREATE TABLE IF NOT EXISTS index_migrations(name TEXT PRIMARY KEY, applied REAL NOT NULL);
";

/// Columns added to `roots` after the first release, with their
/// declarations (SRCH-034). `pin_added` marks roots added because their
/// folder is pinned (SRCH-040).
const ADDED_ROOT_COLUMNS: [(&str, &str); 5] = [
    ("update_mode", "TEXT NOT NULL DEFAULT 'Not watching'"),
    ("watch_count", "INTEGER NOT NULL DEFAULT 0"),
    ("watch_error", "TEXT NOT NULL DEFAULT ''"),
    ("last_event", "REAL NOT NULL DEFAULT 0"),
    ("pin_added", "INTEGER NOT NULL DEFAULT 0"),
];

/// The trigram full-text index over `entries.search_text`.
const FULL_TEXT_TABLE: &str = "CREATE VIRTUAL TABLE IF NOT EXISTS names_fts USING \
    fts5(search_text,content='entries',content_rowid='id',tokenize='trigram')";

/// Triggers that keep the full-text index in step with `entries`.
const FULL_TEXT_TRIGGERS: &str = "
    CREATE TRIGGER IF NOT EXISTS names_insert AFTER INSERT ON entries BEGIN
        INSERT INTO names_fts(rowid,search_text) VALUES(new.id,new.search_text); END;
    CREATE TRIGGER IF NOT EXISTS names_delete AFTER DELETE ON entries BEGIN
        INSERT INTO names_fts(names_fts,rowid,search_text) VALUES('delete',old.id,old.search_text); END;
    CREATE TRIGGER IF NOT EXISTS names_update AFTER UPDATE OF search_text ON entries
        WHEN old.search_text<>new.search_text BEGIN
        INSERT INTO names_fts(names_fts,rowid,search_text) VALUES('delete',old.id,old.search_text);
        INSERT INTO names_fts(rowid,search_text) VALUES(new.id,new.search_text); END;
";

/// Fills the full-text index from every row of `entries`.
const REBUILD_FULL_TEXT: &str = "INSERT INTO names_fts(names_fts) VALUES('rebuild')";

/// The message a recovered root shows.
const INTERRUPTED_MESSAGE: &str = "Refresh to finish the interrupted scan.";

/// Creates missing tables, adds missing columns and returns the matching
/// engine this SQLite supports.
///
/// Runs in one immediate transaction, so two processes opening an older
/// database at once cannot both add the same column.
///
/// # Errors
///
/// Any SQLite error other than a missing FTS5 module or trigram tokenizer.
pub(super) fn prepare(connection: &mut Connection) -> rusqlite::Result<SearchEngine> {
    // WAL gives readers their own snapshot, so a search never waits for a
    // scan that is writing (`search_index.py`: SMB latency cannot block a
    // query). The mode is stored in the file, so it is set once here.
    connection.pragma_update_and_check(None, "journal_mode", "WAL", |_| Ok(()))?;
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    transaction.execute_batch(TABLES)?;
    add_missing_root_columns(&transaction)?;
    let engine = create_full_text_index(&transaction)?;
    transaction.commit()?;
    Ok(engine)
}

/// Marks roots left "Indexing" or "Queued" by a stopped application as
/// "Interrupted" (SRCH-025).
///
/// Safety rule "only the index owner recovers": a second process opening
/// a folder must not mark a live scan interrupted, so only the elected
/// owner calls this (`recover_interrupted` in `search_index.py`).
///
/// # Errors
///
/// The database's error.
pub(super) fn recover_interrupted(connection: &Connection) -> rusqlite::Result<()> {
    connection.execute(
        "UPDATE roots SET status=?1, error=?2 WHERE status IN (?3, ?4)",
        (
            RootStatus::Interrupted.as_str(),
            INTERRUPTED_MESSAGE,
            RootStatus::Indexing.as_str(),
            RootStatus::Queued.as_str(),
        ),
    )?;
    Ok(())
}

/// Adds the [`ADDED_ROOT_COLUMNS`] an older database lacks.
fn add_missing_root_columns(transaction: &Transaction<'_>) -> rusqlite::Result<()> {
    let existing = root_columns(transaction)?;
    for (column, declaration) in ADDED_ROOT_COLUMNS {
        if existing.iter().any(|name| name == column) {
            continue;
        }
        transaction.execute(
            &format!("ALTER TABLE roots ADD COLUMN {column} {declaration}"),
            (),
        )?;
    }
    Ok(())
}

/// The names of the columns `roots` has now.
fn root_columns(transaction: &Transaction<'_>) -> rusqlite::Result<Vec<String>> {
    let mut statement = transaction.prepare("SELECT name FROM pragma_table_info('roots')")?;
    let names = statement.query_map((), |row| row.get(0))?;
    names.collect()
}

/// Creates the trigram index and its triggers; falls back to substring
/// matching when this SQLite has no FTS5 or no trigram tokenizer.
///
/// A database created where FTS5 was missing already holds entries a new
/// index does not know. The triggers index rows only as they change, and
/// a rescan leaves unchanged names unchanged, so those entries would never
/// match a word of three or more characters again. Python had this gap;
/// here a new index is built from the existing entries at once.
fn create_full_text_index(transaction: &Transaction<'_>) -> rusqlite::Result<SearchEngine> {
    let is_new = !has_full_text_table(transaction)?;
    match transaction.execute(FULL_TEXT_TABLE, ()) {
        Ok(_) => {}
        Err(error) if is_missing_feature(&error) => return Ok(SearchEngine::SubstringFallback),
        Err(error) => return Err(error),
    }
    transaction.execute_batch(FULL_TEXT_TRIGGERS)?;
    if is_new {
        transaction.execute(REBUILD_FULL_TEXT, ())?;
    }
    Ok(SearchEngine::Trigram)
}

/// Whether the database has the full-text table already.
fn has_full_text_table(transaction: &Transaction<'_>) -> rusqlite::Result<bool> {
    transaction.query_row(
        "SELECT count(*) > 0 FROM sqlite_master WHERE type='table' AND name='names_fts'",
        (),
        |row| row.get(0),
    )
}

/// "no such module: fts5" and "no such tokenizer: trigram" are generic
/// SQLite errors; busy or I/O errors are not and are reported instead.
fn is_missing_feature(error: &rusqlite::Error) -> bool {
    error.sqlite_error_code() == Some(ErrorCode::Unknown)
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use crate::search::fixtures::{found_names, root_state, SHARE};
    use crate::search::index::SearchIndex;
    use crate::search::requests::IndexRequest;
    use crate::search::root::UpdateMode;

    /// A database of the first release: no monitoring columns, no request
    /// table and no full-text index, holding one cached file.
    const FIRST_RELEASE_DATABASE: &str = r"
        CREATE TABLE roots(
            uri TEXT PRIMARY KEY, label TEXT NOT NULL, enabled INTEGER NOT NULL DEFAULT 1,
            status TEXT NOT NULL DEFAULT 'Not indexed', updated REAL NOT NULL DEFAULT 0,
            scanned INTEGER NOT NULL DEFAULT 0, error TEXT NOT NULL DEFAULT '',
            generation TEXT NOT NULL DEFAULT '', include_hidden INTEGER NOT NULL DEFAULT 0);
        CREATE TABLE entries(
            id INTEGER PRIMARY KEY, root TEXT NOT NULL, uri TEXT NOT NULL,
            parent TEXT NOT NULL, name TEXT NOT NULL, search_text TEXT NOT NULL,
            is_dir INTEGER NOT NULL, hidden INTEGER NOT NULL, size INTEGER,
            modified REAL NOT NULL, kind TEXT NOT NULL, type TEXT NOT NULL,
            generation TEXT NOT NULL, seen REAL NOT NULL,
            UNIQUE(root,uri));
        INSERT INTO roots(uri, label, status, updated) VALUES('smb://nas/share', 'share', 'Ready', 1700000000);
        INSERT INTO entries(root, uri, parent, name, search_text, is_dir, hidden, size, modified, kind, type,
                            generation, seen)
            VALUES('smb://nas/share', 'smb://nas/share/bank.pdf', 'smb://nas/share', 'bank.pdf',
                   'bank.pdf \\nas\share', 0, 0, 12, 1, 'file', 'File', 'first', 1700000000);
    ";

    /// A temporary cache directory holding [`FIRST_RELEASE_DATABASE`].
    fn first_release_cache() -> TempDir {
        let directory = tempfile::tempdir().unwrap();
        let connection = rusqlite::Connection::open(directory.path().join("search.sqlite3")).unwrap();
        connection.execute_batch(FIRST_RELEASE_DATABASE).unwrap();
        directory
    }

    /// parity: SRCH-034
    #[test]
    fn an_older_database_is_upgraded_in_place() {
        let directory = first_release_cache();

        let index = SearchIndex::open(directory.path()).unwrap();

        let root = root_state(&index, SHARE);
        assert_eq!(root.update_mode, UpdateMode::NotWatching);
        assert_eq!(root.watch_count, 0);
        assert_eq!(root.entry_count, 1);
        let request = IndexRequest::Refresh {
            root: SHARE.to_owned(),
        };
        index.enqueue(&request).unwrap();
        assert_eq!(index.drain_requests().unwrap(), [request]);
    }

    /// A new full-text index is built from the entries already cached, so
    /// they match words of three or more characters at once.
    ///
    /// parity: SRCH-010, SRCH-034
    #[test]
    fn entries_cached_before_the_full_text_index_are_found() {
        let directory = first_release_cache();

        let index = SearchIndex::open(directory.path()).unwrap();

        assert_eq!(found_names(&index, "bank"), ["bank.pdf"]);
        assert_eq!(found_names(&index, "ba"), ["bank.pdf"]);
    }

    /// Opening an upgraded database again changes nothing.
    ///
    /// parity: SRCH-034
    #[test]
    fn opening_an_upgraded_database_again_keeps_it() {
        let directory = first_release_cache();
        drop(SearchIndex::open(directory.path()).unwrap());

        let index = SearchIndex::open(directory.path()).unwrap();

        assert_eq!(found_names(&index, "bank"), ["bank.pdf"]);
        assert_eq!(root_state(&index, SHARE).entry_count, 1);
    }
}
