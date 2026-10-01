// SPDX-License-Identifier: AGPL-3.0-only
//! Indexed folders (roots) and the cache status the Search settings show.
//!
//! Ports the `roots` table of `v2.0.0:desktop/search_index.py` and the status
//! words of `search_index.py` and `v2.0.0:desktop/index_service.py`. The words
//! are stored in the database the Python app shares, so each is spelled
//! exactly as Python spells it, in one place: the `as_str` of its enum.

use std::path::PathBuf;

use rusqlite::Row;

use super::text::whole_seconds;

/// Whether a root is searched and kept up to date.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Caching {
    /// The root is indexed and searched.
    Enabled,
    /// The root is listed as "Disabled" and holds no entries.
    Disabled,
}

/// Whether hidden items are indexed below a root, and shown in results.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HiddenItems {
    /// Hidden items are indexed or shown.
    Include,
    /// Hidden items are left out.
    Skip,
}

impl HiddenItems {
    /// The value of the `include_hidden` column.
    pub(crate) fn from_stored(include_hidden: bool) -> Self {
        if include_hidden {
            Self::Include
        } else {
            Self::Skip
        }
    }
}

/// Who chose to index a root.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RootOrigin {
    /// The user chose it in a menu or in the Search settings.
    User,
    /// It was added because the folder is pinned to Quick access
    /// (SRCH-040); the folder list shows it with a "Pinned" tag.
    Pin,
}

impl RootOrigin {
    /// The origin the `pin_added` column records.
    pub(crate) fn from_stored(pin_added: bool) -> Self {
        if pin_added {
            Self::Pin
        } else {
            Self::User
        }
    }
}

/// What a root's last scan achieved. Stored as the Python app's words.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RootStatus {
    /// Never scanned, or cleared.
    NotIndexed,
    /// Waiting for a scan. Only older versions stored it; it is recovered
    /// like [`RootStatus::Indexing`].
    Queued,
    /// A scan is running.
    Indexing,
    /// The last scan completed.
    Ready,
    /// The last scan was cancelled or could not read everything; earlier
    /// results are kept.
    Incomplete,
    /// The application stopped during a scan.
    Interrupted,
    /// Caching is switched off for this root.
    Disabled,
}

impl RootStatus {
    const ALL: [Self; 7] = [
        Self::NotIndexed,
        Self::Queued,
        Self::Indexing,
        Self::Ready,
        Self::Incomplete,
        Self::Interrupted,
        Self::Disabled,
    ];

    /// The stored and displayed word.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::NotIndexed => "Not indexed",
            Self::Queued => "Queued",
            Self::Indexing => "Indexing",
            Self::Ready => "Ready",
            Self::Incomplete => "Incomplete / offline",
            Self::Interrupted => "Interrupted",
            Self::Disabled => "Disabled",
        }
    }

    /// The status a stored word names. A word neither app writes reads as
    /// [`RootStatus::NotIndexed`], which a refresh replaces.
    pub(crate) fn from_stored(word: &str) -> Self {
        Self::ALL
            .into_iter()
            .find(|status| status.as_str() == word)
            .unwrap_or(Self::NotIndexed)
    }
}

/// How changes below a root reach the cache.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UpdateMode {
    /// No scan has set up updates yet.
    NotWatching,
    /// Local folder, every directory watched with inotify.
    LiveLocalEvents,
    /// Local folder where some directories could not be watched; those are
    /// checked on a timer (SRCH-029).
    LiveWithTimedFallback,
    /// Network folder, checked on a timer and never pushed (SRCH-030).
    IncrementalNetworkChecks,
    /// Automatic updates are switched off (the Auto-index setting).
    Paused,
    /// The last check failed; the earlier results are kept.
    OfflineChecks,
}

impl UpdateMode {
    const ALL: [Self; 6] = [
        Self::NotWatching,
        Self::LiveLocalEvents,
        Self::LiveWithTimedFallback,
        Self::IncrementalNetworkChecks,
        Self::Paused,
        Self::OfflineChecks,
    ];

    /// The stored and displayed words.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::NotWatching => "Not watching",
            Self::LiveLocalEvents => "Live local events",
            Self::LiveWithTimedFallback => "Live + timed fallback",
            Self::IncrementalNetworkChecks => "Incremental network checks (not push)",
            Self::Paused => "Paused",
            Self::OfflineChecks => "Offline / incomplete checks",
        }
    }

    /// The mode stored words name; words neither app writes read as
    /// [`UpdateMode::NotWatching`].
    pub(crate) fn from_stored(words: &str) -> Self {
        Self::ALL
            .into_iter()
            .find(|mode| mode.as_str() == words)
            .unwrap_or(Self::NotWatching)
    }
}

/// The token of one scan. Only the scan that holds the current token may
/// store entries or prune old ones (`generation` in `search_index.py`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScanGeneration(String);

impl ScanGeneration {
    /// A new random token.
    pub(crate) fn new() -> Self {
        let uuid = glib::uuid_string_random();
        Self(uuid.replace('-', ""))
    }

    /// The token as stored.
    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }

    /// The stored token; `None` when no scan ever started.
    fn from_stored(token: String) -> Option<Self> {
        (!token.is_empty()).then_some(Self(token))
    }
}

/// One indexed folder and the state of its cache.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexRoot {
    /// Canonical URI of the folder.
    pub uri: String,
    /// Label in the folder list.
    pub label: String,
    /// Whether the root is searched.
    pub caching: Caching,
    /// What the last scan achieved.
    pub status: RootStatus,
    /// When the last complete scan finished, in seconds since the epoch.
    pub updated: Option<u64>,
    /// Entries stored by the current or last scan.
    pub scanned: u64,
    /// Why the last scan was incomplete.
    pub error: Option<String>,
    /// The token of the current or last scan.
    pub generation: Option<ScanGeneration>,
    /// Whether hidden items are indexed.
    pub hidden_items: HiddenItems,
    /// How changes reach the cache.
    pub update_mode: UpdateMode,
    /// Directories watched with inotify.
    pub watch_count: u64,
    /// Why some directories could not be watched, or the last check failed.
    pub watch_error: Option<String>,
    /// When a live update last changed the cache, in seconds since the
    /// epoch.
    pub last_event: Option<u64>,
    /// Entries cached for this root.
    pub entry_count: u64,
    /// Who chose to index it.
    pub origin: RootOrigin,
}

impl IndexRoot {
    /// Whether the root is searched and kept up to date.
    pub fn is_enabled(&self) -> bool {
        self.caching == Caching::Enabled
    }

    /// Reads a row of the roots query in `SearchIndex::roots`.
    ///
    /// # Errors
    ///
    /// The database's error for a column that is missing or of another
    /// type.
    pub(crate) fn from_row(row: &Row<'_>) -> rusqlite::Result<Self> {
        let caching = if row.get("enabled")? {
            Caching::Enabled
        } else {
            Caching::Disabled
        };
        Ok(Self {
            uri: row.get("uri")?,
            label: row.get("label")?,
            caching,
            status: RootStatus::from_stored(&row.get::<_, String>("status")?),
            updated: whole_seconds(row.get("updated")?),
            scanned: stored_count(row, "scanned")?,
            error: non_empty(row.get("error")?),
            generation: ScanGeneration::from_stored(row.get("generation")?),
            hidden_items: HiddenItems::from_stored(row.get("include_hidden")?),
            update_mode: UpdateMode::from_stored(&row.get::<_, String>("update_mode")?),
            watch_count: stored_count(row, "watch_count")?,
            watch_error: non_empty(row.get("watch_error")?),
            last_event: whole_seconds(row.get("last_event")?),
            entry_count: stored_count(row, "entry_count")?,
            origin: RootOrigin::from_stored(row.get("pin_added")?),
        })
    }
}

/// Which SQLite matching the cache uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SearchEngine {
    /// FTS5 with the trigram tokenizer narrows words of three or more
    /// characters before the substring check.
    Trigram,
    /// This SQLite has no FTS5 trigram tokenizer: plain substring matching.
    SubstringFallback,
}

impl SearchEngine {
    /// The name the Search settings show.
    pub fn label(self) -> &'static str {
        match self {
            Self::Trigram => "SQLite FTS5 trigram",
            Self::SubstringFallback => "SQLite substring fallback",
        }
    }
}

/// Everything the Search cache settings show (`snapshot` in
/// `search_index.py`). The cache always holds names and metadata only,
/// never file contents.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CacheStatus {
    /// Every root, enabled or not, by label.
    pub roots: Vec<IndexRoot>,
    /// Entries cached for the enabled roots.
    pub entry_count: u64,
    /// Which SQLite matching is used.
    pub engine: SearchEngine,
    /// Where the database is.
    pub database: PathBuf,
}

/// Monitoring facts recorded for a root after each scan or check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Monitoring {
    /// How changes reach the cache.
    pub(crate) mode: UpdateMode,
    /// Directories watched with inotify.
    pub(crate) watch_count: usize,
    /// Why watching or the last check failed; empty when it did not.
    pub(crate) error: String,
}

/// A count column. SQLite has no unsigned integers; a negative value,
/// which neither app writes, reads as 0.
fn stored_count(row: &Row<'_>, column: &str) -> rusqlite::Result<u64> {
    let count: i64 = row.get(column)?;
    Ok(u64::try_from(count).unwrap_or_default())
}

/// `None` for the empty text the database stores for "no message".
fn non_empty(text: String) -> Option<String> {
    (!text.is_empty()).then_some(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stored_words_read_back_as_the_same_status_and_mode() {
        for status in RootStatus::ALL {
            assert_eq!(RootStatus::from_stored(status.as_str()), status);
        }
        for mode in UpdateMode::ALL {
            assert_eq!(UpdateMode::from_stored(mode.as_str()), mode);
        }
    }

    #[test]
    fn unknown_words_read_as_the_starting_state() {
        assert_eq!(
            RootStatus::from_stored("Paused by a newer app"),
            RootStatus::NotIndexed
        );
        assert_eq!(UpdateMode::from_stored(""), UpdateMode::NotWatching);
    }

    #[test]
    fn scan_generations_are_unique_python_style_hex() {
        let first = ScanGeneration::new();
        let second = ScanGeneration::new();
        assert_ne!(first, second);
        assert_eq!(first.as_str().len(), 32);
        assert!(first.as_str().bytes().all(|byte| byte.is_ascii_hexdigit()));
    }
}
