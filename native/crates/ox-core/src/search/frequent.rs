// SPDX-License-Identifier: AGPL-3.0-only
//! Frequently opened folders are indexed automatically (experimental).
//!
//! A new behaviour; neither the Python app nor the file managers it ports
//! have it. While the "Index frequently opened folders" switch is on, the
//! app tells the service each time the user opens a local folder
//! ([`IndexService::folder_opened`]). Each folder keeps a score that
//! counts its opens and halves every [`HALF_LIFE_SECONDS`], so it says how
//! often the folder is opened lately rather than ever. A folder whose
//! score reaches [`INDEX_SCORE`] is added to the search cache as a root
//! marked as added by frequent use (the `frequent_added` column, shown as
//! [`RootOrigin::Frequent`]), scanned at once and watched from then on,
//! so searching it is instant. It is removed again once its score falls
//! below [`KEEP_SCORE`], so a folder is not added and removed on
//! alternate days, and when more than [`MAX_FREQUENT_ROOTS`] folders
//! qualify, the least used go.
//!
//! Privacy and safety rules, on top of those of the cache:
//!
//! - Off by default. Nothing is recorded while it is off, and turning it
//!   off forgets every recorded open and skipped folder and removes the
//!   roots it added.
//! - Only folders on this computer's own disks count: a share, or a
//!   CIFS, NFS or sshfs mount, is never crawled because it was opened, as
//!   opening one is no choice to index it. Without the mount table, no
//!   folder counts.
//! - Nothing is recorded or added while Auto-index is paused.
//! - The home folder, the folders above it, and system folders such as
//!   `/proc` or `/tmp` are never added; they are either too large to
//!   crawl unasked or change all the time. The user can still add any of
//!   them by hand.
//! - Hidden folders (a name starting with a dot, such as `~/.cache` or
//!   the Trash in `~/.local/share`) are neither counted nor added: what
//!   the user opens there is no one's business but theirs.
//! - The user's choice wins: a folder that already has a root, even one
//!   the user switched off, or that an enabled root already indexes, is
//!   left as it is, and a root the user later configures, or pins, stops
//!   being frequent use's. Nor is a folder added above one the user
//!   indexes, which would crawl that folder twice. Of a folder and one
//!   inside it, only the more used is indexed, and never more than
//!   [`MAX_FREQUENT_ROOTS`], even when two opens are counted at once.
//! - A folder too large to crawl unasked is skipped: a scan of a root
//!   frequent use added stops once it would hold more than
//!   [`ServiceLimits::entries_per_frequent_root`] entries (25,000), or more
//!   than is left of [`ServiceLimits::frequent_entries`] (100,000) beside
//!   the other roots frequent use added. The root and what the scan stored
//!   are then removed, and the folder is not added again for
//!   [`SKIP_SECONDS`] (30 days), however often it is opened. Nor is any
//!   folder added while those roots already hold that many entries.
//! - Turning the switch off wins over an open counted at the same moment:
//!   an open is recorded, and a root added, only in a transaction that
//!   finds the switch on.
//!
//! The switch is kept in the cache (the `index_options` table), as the
//! pinned folders' switch is (see [`super::pins`]); the skipped folders
//! are kept in the `frequent_skipped` table.
//!
//! [`ServiceLimits::entries_per_frequent_root`]: super::limits::ServiceLimits::entries_per_frequent_root
//! [`ServiceLimits::frequent_entries`]: super::limits::ServiceLimits::frequent_entries

use rusqlite::OptionalExtension;

use super::error::SearchError;
use super::index::{begin_immediate, indexable_root, root_label, SearchIndex};
use super::mounts::{read_mounts, Mount};
use super::policy::RootStorage;
use super::root::{IndexRoot, RootOrigin};
use super::service::{AutoIndex, IndexService, ScanTrigger};
use super::text::{is_at_or_below, unix_now};
use crate::location::{file_uri, normalise};

/// The row of `index_options` that holds the switch.
const FREQUENT_INDEXING_OPTION: &str = "frequent-indexing";

/// How long it takes an open to count half as much: a week.
const HALF_LIFE_SECONDS: f64 = 7.0 * 24.0 * 60.0 * 60.0;

/// The score at which a folder is indexed: about four opens in a few days,
/// or daily use over a week.
pub const INDEX_SCORE: f64 = 4.0;

/// The score below which a folder that frequent use indexed is removed
/// again.
pub const KEEP_SCORE: f64 = 1.5;

/// The most folders frequent use indexes at once.
pub const MAX_FREQUENT_ROOTS: usize = 8;

/// The score below which an open is forgotten altogether.
const FORGET_SCORE: f64 = 0.1;

/// The most folders whose opens are remembered; the least used beyond it
/// are forgotten.
const MAX_REMEMBERED_FOLDERS: usize = 256;

/// Two opens of the same folder this close together count once, so that
/// going into a subfolder and back is not two opens.
const SAME_OPEN_SECONDS: f64 = 60.0;

/// How long a folder too large to index is not added again: 30 days.
const SKIP_SECONDS: f64 = 30.0 * 24.0 * 60.0 * 60.0;

/// Folders, and everything below them, that frequent use never indexes.
const SYSTEM_FOLDERS: [&str; 6] = [
    "file:///proc",
    "file:///sys",
    "file:///dev",
    "file:///run",
    "file:///tmp",
    "file:///var/tmp",
];

/// The experimental "Index frequently opened folders" switch in the Search
/// settings; off by default.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FrequentIndexing {
    /// Opens are not recorded and frequent use adds no root.
    #[default]
    Off,
    /// Frequently opened local folders are indexed.
    Automatic,
}

impl FrequentIndexing {
    /// The word `index_options` stores.
    const fn as_str(self) -> &'static str {
        match self {
            Self::Automatic => "automatic",
            Self::Off => "off",
        }
    }

    /// The switch a stored word names; a word this version does not write
    /// reads as the default, off.
    fn from_stored(word: &str) -> Self {
        if word == Self::Automatic.as_str() {
            Self::Automatic
        } else {
            Self::Off
        }
    }
}

/// A folder's recorded opens.
#[derive(Debug, Clone, PartialEq)]
struct Visit {
    uri: String,
    /// The score at `updated`.
    score: f64,
    /// When the score was last brought up to date, in seconds since the
    /// epoch.
    updated: f64,
}

impl Visit {
    /// The score at `now`, halved for every [`HALF_LIFE_SECONDS`] since it
    /// was last brought up to date.
    fn score_at(&self, now: f64) -> f64 {
        decayed(self.score, now - self.updated)
    }
}

/// `score` after `elapsed` seconds; time going backwards changes nothing.
fn decayed(score: f64, elapsed: f64) -> f64 {
    let half_lives = elapsed.max(0.0) / HALF_LIFE_SECONDS;
    score * 0.5_f64.powf(half_lives)
}

/// What frequent use should change: the folders to index and the roots it
/// added to remove.
#[derive(Debug, Default, PartialEq)]
struct Rebalance {
    add: Vec<String>,
    remove: Vec<String>,
}

/// Decides which folders frequent use indexes. Of the folders at or above
/// [`INDEX_SCORE`], or already indexed and still at or above
/// [`KEEP_SCORE`], the best scored that `may_add` accepts are chosen,
/// skipping a folder inside or above one already chosen, until there are
/// [`MAX_FREQUENT_ROOTS`]. `indexed` are the roots frequent use added;
/// every other one of them is removed, the least used first.
fn rebalance(visits: &[Visit], indexed: &[String], now: f64, may_add: impl Fn(&str) -> bool) -> Rebalance {
    let score_of = |uri: &str| {
        visits
            .iter()
            .find(|visit| visit.uri == uri)
            .map_or(0.0, |visit| visit.score_at(now))
    };
    let mut candidates: Vec<(&str, f64)> = visits
        .iter()
        .map(|visit| (visit.uri.as_str(), visit.score_at(now)))
        .filter(|(uri, score)| {
            let threshold = if indexed.iter().any(|root| root == uri) {
                KEEP_SCORE
            } else {
                INDEX_SCORE
            };
            *score >= threshold
        })
        .collect();
    candidates.sort_by(|a, b| b.1.total_cmp(&a.1).then_with(|| a.0.cmp(b.0)));
    let mut wanted: Vec<&str> = Vec::new();
    for (uri, _) in candidates {
        if wanted.len() == MAX_FREQUENT_ROOTS {
            break;
        }
        // A folder is indexed once (SRCH-032): of a folder and one inside
        // it, the more used one is indexed.
        let overlaps = wanted
            .iter()
            .any(|chosen| is_at_or_below(uri, chosen) || is_at_or_below(chosen, uri));
        if !overlaps && may_add(uri) {
            wanted.push(uri);
        }
    }
    let mut remove: Vec<String> = indexed
        .iter()
        .filter(|uri| !wanted.contains(&uri.as_str()))
        .cloned()
        .collect();
    remove.sort_by(|a, b| score_of(a).total_cmp(&score_of(b)));
    let add = wanted
        .iter()
        .filter(|uri| !indexed.iter().any(|root| root == *uri))
        .map(|uri| (*uri).to_owned())
        .collect();
    Rebalance { add, remove }
}

/// Whether frequent use may index the folder `uri` at all: a folder on
/// this computer's own disks, as `mounts` tell (`None` when the mount
/// table could not be read, when no folder may), that is not the home
/// folder, above it, a system folder or a hidden one.
fn may_index(uri: &str, home: &str, mounts: Option<&[Mount]>) -> bool {
    uri.starts_with("file:///")
        && !is_at_or_below(home, uri)
        && !SYSTEM_FOLDERS.iter().any(|system| is_at_or_below(uri, system))
        && !is_hidden(uri)
        && RootStorage::of(uri, mounts) == RootStorage::Local
}

/// Whether the local folder `uri` is hidden or inside a hidden folder.
fn is_hidden(uri: &str) -> bool {
    uri.trim_start_matches("file://")
        .split('/')
        .any(|name| name.starts_with('.'))
}

/// Whether a root other than frequent use's keeps the folder `uri` among
/// `roots`: one for the folder itself, even switched off, an enabled one
/// that `reaches` it from above, or an enabled one inside it, which a root
/// for `uri` would crawl a second time.
fn is_kept_by_other_root(roots: &[IndexRoot], uri: &str, reaches: impl Fn(&IndexRoot) -> bool) -> bool {
    roots
        .iter()
        .filter(|root| root.origin != RootOrigin::Frequent)
        .any(|root| {
            root.uri == uri
                || (root.is_enabled()
                    && (is_at_or_below(&root.uri, uri) || (is_at_or_below(uri, &root.uri) && reaches(root))))
        })
}

/// The home folder as a location.
fn home_uri() -> String {
    file_uri(&glib::home_dir())
}

impl IndexService {
    /// Records that the user opened the folder `uri` and, when it is now
    /// opened often enough, indexes it; frequently opened folders that are
    /// no longer opened often are removed. Nothing is recorded while the
    /// switch is off, or for a folder frequent use never indexes. Returns
    /// whether a root was added or removed.
    ///
    /// # Errors
    ///
    /// Database errors.
    pub fn folder_opened(&self, uri: &str) -> Result<bool, SearchError> {
        let mounts = read_mounts().ok();
        self.folder_opened_at(uri, unix_now(), &home_uri(), mounts.as_deref())
    }

    /// [`Self::folder_opened`] at the time `now`, with `home` as the home
    /// folder and `mounts` as the mount table.
    fn folder_opened_at(
        &self,
        uri: &str,
        now: f64,
        home: &str,
        mounts: Option<&[Mount]>,
    ) -> Result<bool, SearchError> {
        if self.is_auto_index_paused() {
            return Ok(false);
        }
        // Virtual places such as the Recycle Bin, and anything that is no
        // folder address, are not folders the user works in.
        let Ok(uri) = normalise(uri) else {
            return Ok(false);
        };
        if !may_index(&uri, home, mounts) {
            return Ok(false);
        }
        if !self.index().record_visit(&uri, now)? {
            return Ok(false);
        }
        self.rebalance_frequent_roots(now)
    }

    /// Brings the roots frequent use added up to date when the service
    /// starts, so a folder no longer opened often is removed even if no
    /// folder is opened, unless the switch is off or `auto_index` is
    /// paused. Returns whether a root was added or removed.
    ///
    /// # Errors
    ///
    /// Database errors.
    pub fn update_frequent_roots(&self, auto_index: AutoIndex) -> Result<bool, SearchError> {
        if auto_index == AutoIndex::Paused || self.index().frequent_indexing()? == FrequentIndexing::Off {
            return Ok(false);
        }
        self.rebalance_frequent_roots(unix_now())
    }

    /// Follows a change of the "Index frequently opened folders" switch and
    /// remembers it for [`SearchIndex::frequent_indexing`]. Turning it off
    /// forgets every recorded open and removes the roots frequent use
    /// added, while roots the user chose stay.
    ///
    /// # Errors
    ///
    /// Database errors.
    pub fn set_frequent_indexing(&self, indexing: FrequentIndexing) -> Result<(), SearchError> {
        self.index().store_frequent_indexing(indexing)?;
        if indexing == FrequentIndexing::Off {
            // No root is added once the switch is stored off (see
            // [`SearchIndex::add_frequent_root`]), so none outlives this.
            for uri in self.index().frequent_roots()? {
                self.stop(&uri)?;
                self.index().remove_frequent_root(&uri)?;
            }
        }
        Ok(())
    }

    /// Whether Auto-index is paused, when frequent use changes nothing.
    fn is_auto_index_paused(&self) -> bool {
        self.shared.state().settings.auto_index == AutoIndex::Paused
    }

    /// Adds the roots frequent use now wants and removes those it no
    /// longer does; returns whether it changed any. A folder skipped as
    /// too large in the last [`SKIP_SECONDS`] is not added, nor is any
    /// while the roots frequent use added hold
    /// [`ServiceLimits::frequent_entries`](super::limits::ServiceLimits::frequent_entries).
    fn rebalance_frequent_roots(&self, now: f64) -> Result<bool, SearchError> {
        let visits = self.index().visits()?;
        let indexed = self.index().frequent_roots()?;
        let roots = self.index().roots()?;
        let skipped = self.index().skipped_frequent_folders(now)?;
        let frequent_entries: u64 = roots
            .iter()
            .filter(|root| root.origin == RootOrigin::Frequent)
            .map(|root| root.entry_count)
            .sum();
        let is_full =
            usize::try_from(frequent_entries).unwrap_or(usize::MAX) >= self.shared.limits.frequent_entries;
        let changes = rebalance(&visits, &indexed, now, |uri| {
            let is_new = !indexed.iter().any(|root| root == uri);
            let is_refused = is_new && (is_full || skipped.iter().any(|folder| folder == uri));
            !is_refused
                && !is_kept_by_other_root(&roots, uri, |root| {
                    self.index().root_reaches(&root.uri, root.hidden_items, uri)
                })
        });
        let mut is_changed = !changes.remove.is_empty();
        for uri in &changes.remove {
            self.stop(uri)?;
            self.index().remove_frequent_root(uri)?;
        }
        for uri in &changes.add {
            if self.index().add_frequent_root(uri)?.is_some() {
                // As a pin's: an automatic scan, which never resumes a
                // server paused for sign-out.
                self.start_scan(uri, ScanTrigger::Automatic)?;
                is_changed = true;
            }
        }
        Ok(is_changed)
    }
}

impl SearchIndex {
    /// The "Index frequently opened folders" switch as last set in any
    /// process; off until the user turns it on.
    ///
    /// # Errors
    ///
    /// Database errors.
    pub fn frequent_indexing(&self) -> Result<FrequentIndexing, SearchError> {
        let connection = self.connect()?;
        Ok(read_frequent_indexing(&connection)?)
    }

    /// Remembers the switch for [`SearchIndex::frequent_indexing`]; off
    /// also forgets every recorded open and skipped folder, in the same
    /// transaction.
    fn store_frequent_indexing(&self, indexing: FrequentIndexing) -> Result<(), SearchError> {
        let mut connection = self.connect()?;
        let transaction = begin_immediate(&mut connection)?;
        transaction.execute(
            "INSERT INTO index_options(name, value) VALUES(?1, ?2)
             ON CONFLICT(name) DO UPDATE SET value=excluded.value",
            (FREQUENT_INDEXING_OPTION, indexing.as_str()),
        )?;
        if indexing == FrequentIndexing::Off {
            transaction.execute("DELETE FROM folder_visits", ())?;
            transaction.execute("DELETE FROM frequent_skipped", ())?;
        }
        transaction.commit()?;
        Ok(())
    }

    /// Counts an open of the folder `uri` at `now`, and forgets the opens
    /// that no longer count or are beyond [`MAX_REMEMBERED_FOLDERS`].
    /// Returns `false`, recording nothing, when the switch is off.
    fn record_visit(&self, uri: &str, now: f64) -> Result<bool, SearchError> {
        let mut connection = self.connect()?;
        let transaction = begin_immediate(&mut connection)?;
        if read_frequent_indexing(&transaction)? == FrequentIndexing::Off {
            return Ok(false);
        }
        let earlier: Option<(f64, f64)> = transaction
            .query_row(
                "SELECT score, updated FROM folder_visits WHERE uri=?1",
                [uri],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        let visit = match earlier {
            Some((score, updated)) if now - updated < SAME_OPEN_SECONDS => Visit {
                uri: uri.to_owned(),
                score,
                updated,
            },
            Some((score, updated)) => Visit {
                uri: uri.to_owned(),
                score: decayed(score, now - updated) + 1.0,
                updated: now,
            },
            None => Visit {
                uri: uri.to_owned(),
                score: 1.0,
                updated: now,
            },
        };
        transaction.execute(
            "INSERT INTO folder_visits(uri, score, updated) VALUES(?1, ?2, ?3)
             ON CONFLICT(uri) DO UPDATE SET score=excluded.score, updated=excluded.updated",
            (&visit.uri, visit.score, visit.updated),
        )?;
        let forgotten = forgotten_visits(&read_visits(&transaction)?, now);
        for uri in &forgotten {
            transaction.execute("DELETE FROM folder_visits WHERE uri=?1", [uri])?;
        }
        transaction.commit()?;
        Ok(true)
    }

    /// Every folder whose opens are remembered.
    fn visits(&self) -> Result<Vec<Visit>, SearchError> {
        let connection = self.connect()?;
        Ok(read_visits(&connection)?)
    }

    /// The roots frequent use added.
    fn frequent_roots(&self) -> Result<Vec<String>, SearchError> {
        let connection = self.connect()?;
        let mut statement = connection.prepare("SELECT uri FROM roots WHERE frequent_added=1")?;
        let uris = statement.query_map((), |row| row.get(0))?;
        Ok(uris.collect::<rusqlite::Result<_>>()?)
    }

    /// Adds `uri` as an enabled root marked as added by frequent use, and
    /// returns it; `None` when the switch is off, the folder cannot be
    /// indexed, already has a root, or an enabled root already indexes it.
    ///
    /// Safety rule "the user's choice wins": an existing root, including
    /// one the user switched off, is left exactly as it is. The switch is
    /// read in the transaction that adds the root, so a root is never
    /// added after it is turned off.
    fn add_frequent_root(&self, uri: &str) -> Result<Option<String>, SearchError> {
        let uri = match indexable_root(uri) {
            Ok(uri) => uri,
            Err(SearchError::DeviceLocation | SearchError::ServerList) => return Ok(None),
            Err(error) => return Err(error),
        };
        if self.is_indexed_by_enabled_root(&uri)? {
            return Ok(None);
        }
        let label = root_label("", &uri);
        let mut connection = self.connect()?;
        let transaction = begin_immediate(&mut connection)?;
        if read_frequent_indexing(&transaction)? == FrequentIndexing::Off {
            return Ok(None);
        }
        // Two opens counted at once each make room for one folder; only
        // the first takes it.
        let frequent: i64 =
            transaction.query_row("SELECT COUNT(*) FROM roots WHERE frequent_added=1", [], |row| {
                row.get(0)
            })?;
        if usize::try_from(frequent).unwrap_or(usize::MAX) >= MAX_FREQUENT_ROOTS {
            return Ok(None);
        }
        let added = transaction.execute(
            "INSERT INTO roots(uri, label, enabled, include_hidden, frequent_added) VALUES(?1, ?2, 1, 0, 1)
             ON CONFLICT(uri) DO NOTHING",
            (&uri, &label),
        )?;
        transaction.commit()?;
        Ok((added == 1).then_some(uri))
    }

    /// Removes the root `uri`, which a scan found too large, and its
    /// entries if frequent use added it, and remembers to skip the folder
    /// until [`SKIP_SECONDS`] after `now`; skips older than that are
    /// forgotten. Returns whether the root was removed: not when the user
    /// chose or pinned it in the meantime, which keeps it. Nothing is
    /// remembered once the switch is off.
    pub(super) fn skip_frequent_root(&self, uri: &str, now: f64) -> Result<bool, SearchError> {
        let mut connection = self.connect()?;
        let transaction = begin_immediate(&mut connection)?;
        let removed = transaction.execute(
            "DELETE FROM roots WHERE uri=?1 AND frequent_added=1 AND pin_added=0",
            [uri],
        )?;
        if removed != 1 {
            return Ok(false);
        }
        transaction.execute("DELETE FROM entries WHERE root=?1", [uri])?;
        transaction.execute(
            "DELETE FROM frequent_skipped WHERE skipped<=?1",
            [now - SKIP_SECONDS],
        )?;
        if read_frequent_indexing(&transaction)? == FrequentIndexing::Automatic {
            transaction.execute(
                "INSERT INTO frequent_skipped(uri, skipped) VALUES(?1, ?2)
                 ON CONFLICT(uri) DO UPDATE SET skipped=excluded.skipped",
                (uri, now),
            )?;
        }
        transaction.commit()?;
        Ok(true)
    }

    /// The folders skipped as too large in the [`SKIP_SECONDS`] before
    /// `now`.
    fn skipped_frequent_folders(&self, now: f64) -> Result<Vec<String>, SearchError> {
        let connection = self.connect()?;
        let mut statement = connection.prepare("SELECT uri FROM frequent_skipped WHERE skipped>?1")?;
        let uris = statement.query_map([now - SKIP_SECONDS], |row| row.get(0))?;
        Ok(uris.collect::<rusqlite::Result<_>>()?)
    }

    /// Deletes the root `uri` and its entries if frequent use added it.
    ///
    /// Safety rule "the user's choice wins": the check and the deletion
    /// are one transaction, so a root the user chose or pinned in the
    /// meantime stays.
    fn remove_frequent_root(&self, uri: &str) -> Result<(), SearchError> {
        let mut connection = self.connect()?;
        let transaction = begin_immediate(&mut connection)?;
        let removed = transaction.execute(
            "DELETE FROM roots WHERE uri=?1 AND frequent_added=1 AND pin_added=0",
            [uri],
        )?;
        if removed == 1 {
            transaction.execute("DELETE FROM entries WHERE root=?1", [uri])?;
        }
        transaction.commit()?;
        Ok(())
    }
}

/// The switch as stored, off when it never was.
fn read_frequent_indexing(connection: &rusqlite::Connection) -> rusqlite::Result<FrequentIndexing> {
    let stored: Option<String> = connection
        .query_row(
            "SELECT value FROM index_options WHERE name=?1",
            [FREQUENT_INDEXING_OPTION],
            |row| row.get(0),
        )
        .optional()?;
    let indexing = stored.as_deref().map(FrequentIndexing::from_stored);
    Ok(indexing.unwrap_or_default())
}

/// Every row of `folder_visits`.
fn read_visits(connection: &rusqlite::Connection) -> rusqlite::Result<Vec<Visit>> {
    let mut statement = connection.prepare("SELECT uri, score, updated FROM folder_visits")?;
    let rows = statement.query_map((), |row| {
        Ok(Visit {
            uri: row.get(0)?,
            score: row.get(1)?,
            updated: row.get(2)?,
        })
    })?;
    rows.collect()
}

/// The folders among `visits` whose opens no longer count at `now`, below
/// [`FORGET_SCORE`], or that are beyond the [`MAX_REMEMBERED_FOLDERS`]
/// best scored.
fn forgotten_visits(visits: &[Visit], now: f64) -> Vec<String> {
    let mut scored: Vec<(&str, f64)> = visits
        .iter()
        .map(|visit| (visit.uri.as_str(), visit.score_at(now)))
        .collect();
    scored.sort_by(|a, b| b.1.total_cmp(&a.1).then_with(|| a.0.cmp(b.0)));
    scored
        .iter()
        .enumerate()
        .filter(|(rank, (_, score))| *rank >= MAX_REMEMBERED_FOLDERS || *score < FORGET_SCORE)
        .map(|(_, (uri, _))| (*uri).to_owned())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::search::fixtures::{open_index, wait_until};
    use crate::search::limits::ServiceLimits;
    use crate::search::reader::GioFolderReader;
    use crate::search::root::{Caching, HiddenItems, RootStatus};
    use crate::search::service::IndexSettings;
    use crate::search::PinIndexing;
    use crate::settings::Bookmark;

    const HOME: &str = "file:///home/alex";
    const PROJECTS: &str = "file:///home/alex/Projects";
    const DAY: f64 = 24.0 * 60.0 * 60.0;

    /// A computer whose folders are on one local disk, with a team share
    /// mounted over CIFS at `/mnt/team`.
    fn mounts() -> Vec<Mount> {
        vec![Mount::new("/", "ext4"), Mount::new("/mnt/team", "cifs")]
    }

    fn may_index_here(uri: &str) -> bool {
        may_index(uri, HOME, Some(&mounts()))
    }

    fn opened(service: &IndexService, uri: &str, now: f64) -> bool {
        service.folder_opened_at(uri, now, HOME, Some(&mounts())).unwrap()
    }

    fn every_folder(_: &str) -> bool {
        true
    }

    fn visit(uri: &str, score: f64, updated: f64) -> Visit {
        Visit {
            uri: uri.to_owned(),
            score,
            updated,
        }
    }

    #[test]
    fn a_score_halves_every_week() {
        let week = visit(PROJECTS, 8.0, 0.0);

        assert!((week.score_at(7.0 * DAY) - 4.0).abs() < 1e-9);
        assert!((week.score_at(14.0 * DAY) - 2.0).abs() < 1e-9);
        assert!((week.score_at(-DAY) - 8.0).abs() < 1e-9);
    }

    #[test]
    fn only_local_folders_below_home_or_elsewhere_may_be_indexed() {
        assert!(may_index_here(PROJECTS));
        assert!(may_index_here("file:///srv/media"));
        assert!(!may_index_here(HOME));
        assert!(!may_index_here("file:///home"));
        assert!(!may_index_here("file:///"));
        assert!(!may_index_here("file:///tmp/build"));
        assert!(!may_index_here("file:///proc"));
        assert!(!may_index_here("smb://nas/share/Team"));
        assert!(!may_index_here("file:///mnt/team/Reports"));
        assert!(!may_index_here("file:///home/alex/.cache/thumbnails"));
        assert!(!may_index_here("file:///home/alex/.local/share/Trash/files"));
        assert!(!may_index_here("file:///srv/.private"));
        // Without the mount table, a network mount cannot be told apart.
        assert!(!may_index(PROJECTS, HOME, None));
    }

    /// parity: SRCH-041
    #[test]
    fn a_folder_is_indexed_at_the_index_score_and_kept_until_the_keep_score() {
        let now = 100.0 * DAY;
        let fresh = [visit(PROJECTS, INDEX_SCORE - 0.5, now)];
        let busy = [visit(PROJECTS, INDEX_SCORE, now)];
        let fading = [visit(PROJECTS, KEEP_SCORE + 0.1, now)];
        let gone = [visit(PROJECTS, KEEP_SCORE - 0.1, now)];
        let indexed = [PROJECTS.to_owned()];

        assert_eq!(rebalance(&fresh, &[], now, every_folder), Rebalance::default());
        assert_eq!(rebalance(&busy, &[], now, every_folder).add, [PROJECTS]);
        assert_eq!(
            rebalance(&fading, &indexed, now, every_folder),
            Rebalance::default()
        );
        assert_eq!(rebalance(&gone, &indexed, now, every_folder).remove, [PROJECTS]);
    }

    #[test]
    fn at_most_the_best_scored_folders_are_indexed() {
        let now = 0.0;
        let count = u32::try_from(MAX_FREQUENT_ROOTS + 2).expect("a few folders");
        let visits: Vec<Visit> = (0..count)
            .map(|n| visit(&format!("file:///data/{n:02}"), INDEX_SCORE + f64::from(n), now))
            .collect();
        let least = ["file:///data/00".to_owned()];

        let changes = rebalance(&visits, &least, now, every_folder);

        assert_eq!(changes.add.len(), MAX_FREQUENT_ROOTS);
        assert_eq!(changes.add[0], "file:///data/09");
        assert!(!changes.add.contains(&"file:///data/01".to_owned()));
        assert_eq!(changes.remove, least);
        // A folder that may not be added leaves its place to the next.
        let changes = rebalance(&visits, &least, now, |uri| uri != "file:///data/09");
        assert_eq!(changes.add.len(), MAX_FREQUENT_ROOTS);
        assert!(changes.add.contains(&"file:///data/01".to_owned()));
    }

    /// parity: SRCH-041
    #[test]
    fn of_a_folder_and_one_inside_it_the_more_used_is_indexed() {
        let now = 0.0;
        let report = "file:///home/alex/Projects/Report";
        let visits = [
            visit(PROJECTS, INDEX_SCORE, now),
            visit(report, INDEX_SCORE + 1.0, now),
        ];
        let indexed = [PROJECTS.to_owned()];

        let changes = rebalance(&visits, &indexed, now, every_folder);

        assert_eq!(changes.add, [report]);
        assert_eq!(changes.remove, indexed);
    }

    #[test]
    fn faded_and_surplus_opens_are_forgotten() {
        let now = 60.0 * DAY;
        let visits = [visit(PROJECTS, 1.0, 0.0), visit("file:///srv/media", 1.0, now)];

        assert_eq!(forgotten_visits(&visits, now), [PROJECTS]);
    }

    #[test]
    fn opens_are_counted_once_a_minute_and_the_switch_is_shared() {
        let directory = tempfile::tempdir().unwrap();
        let index = open_index(&directory);
        assert_eq!(index.frequent_indexing().unwrap(), FrequentIndexing::Off);

        assert!(!index.record_visit(PROJECTS, 0.0).unwrap());
        index
            .store_frequent_indexing(FrequentIndexing::Automatic)
            .unwrap();
        assert!(index.record_visit(PROJECTS, 0.0).unwrap());
        index.record_visit(PROJECTS, 10.0).unwrap();
        index.record_visit(PROJECTS, 120.0).unwrap();

        let visits = index.visits().unwrap();
        assert_eq!(visits.len(), 1);
        assert!((visits[0].score - 2.0).abs() < 1e-3);
        let other_process = SearchIndex::open(directory.path()).unwrap();
        assert_eq!(
            other_process.frequent_indexing().unwrap(),
            FrequentIndexing::Automatic
        );
    }

    /// parity: SRCH-041
    #[test]
    fn a_frequent_root_yields_to_the_users_choice_and_to_a_pin() {
        let directory = tempfile::tempdir().unwrap();
        let index = open_index(&directory);
        index
            .configure("file:///srv/off", Caching::Disabled, "", HiddenItems::Skip)
            .unwrap();

        assert_eq!(index.add_frequent_root(PROJECTS).unwrap(), None);
        index
            .store_frequent_indexing(FrequentIndexing::Automatic)
            .unwrap();
        assert_eq!(index.add_frequent_root("file:///srv/off").unwrap(), None);
        assert!(index.add_frequent_root(PROJECTS).unwrap().is_some());
        assert_eq!(index.root_origin(PROJECTS).unwrap(), Some(RootOrigin::Frequent));
        index
            .configure(PROJECTS, Caching::Enabled, "", HiddenItems::Skip)
            .unwrap();
        assert_eq!(index.root_origin(PROJECTS).unwrap(), Some(RootOrigin::User));
        index.remove_frequent_root(PROJECTS).unwrap();
        assert_eq!(index.root_origin(PROJECTS).unwrap(), Some(RootOrigin::User));
    }

    /// parity: SRCH-041
    #[test]
    fn no_more_frequent_roots_than_the_cap_are_added_even_at_once() {
        let directory = tempfile::tempdir().unwrap();
        let index = open_index(&directory);
        index
            .store_frequent_indexing(FrequentIndexing::Automatic)
            .unwrap();

        for number in 0..MAX_FREQUENT_ROOTS {
            let uri = format!("file:///srv/folder{number}");
            assert!(index.add_frequent_root(&uri).unwrap().is_some());
        }
        assert_eq!(index.add_frequent_root("file:///srv/one-more").unwrap(), None);
        assert_eq!(index.frequent_roots().unwrap().len(), MAX_FREQUENT_ROOTS);
    }

    /// A new folder `name` in `base` with `files` files, as a location.
    fn folder_with_files(base: &std::path::Path, name: &str, files: usize) -> String {
        let folder = base.join(name);
        std::fs::create_dir(&folder).unwrap();
        for number in 0..files {
            std::fs::write(folder.join(format!("{number}.txt")), "x").unwrap();
        }
        file_uri(&folder)
    }

    /// A service over a new cache in `base` with `limits`.
    fn start_with(base: &tempfile::TempDir, limits: ServiceLimits) -> IndexService {
        let index = SearchIndex::open(&base.path().join("index")).unwrap();
        let reader = Box::new(GioFolderReader);
        let service = IndexService::start_with_limits(index, reader, Box::new(|| {}), limits).unwrap();
        service
            .set_frequent_indexing(FrequentIndexing::Automatic)
            .unwrap();
        service
    }

    /// Counts five opens, two minutes apart, of each of `uris` from `now`.
    fn open_often(service: &IndexService, uris: &[&str], now: f64) {
        for uri in uris {
            for open in 0..5 {
                let at = now + f64::from(open) * 120.0;
                assert!(service.index().record_visit(uri, at).unwrap());
            }
        }
    }

    /// Ticks `service` until no root that frequent use added waits for or
    /// runs a scan.
    fn wait_for_frequent_scans(service: &IndexService, what: &str) {
        let tick = || service.tick(&IndexSettings::default()).unwrap();
        wait_until(what, tick, || {
            service.index().roots().unwrap().iter().all(|root| {
                root.origin != RootOrigin::Frequent
                    || !matches!(
                        root.status,
                        RootStatus::NotIndexed | RootStatus::Queued | RootStatus::Indexing
                    )
            })
        });
    }

    fn entry_count(service: &IndexService, uri: &str) -> u64 {
        let roots = service.index().roots().unwrap();
        roots.iter().find(|root| root.uri == uri).unwrap().entry_count
    }

    #[test]
    fn a_skipped_folder_is_remembered_for_thirty_days_unless_the_user_keeps_it() {
        let directory = tempfile::tempdir().unwrap();
        let index = open_index(&directory);
        index
            .store_frequent_indexing(FrequentIndexing::Automatic)
            .unwrap();
        let now = 100.0 * DAY;
        index.add_frequent_root(PROJECTS).unwrap();
        index.add_frequent_root("file:///srv/media").unwrap();
        index
            .configure("file:///srv/media", Caching::Enabled, "", HiddenItems::Skip)
            .unwrap();

        assert!(index.skip_frequent_root(PROJECTS, now).unwrap());
        assert!(!index.skip_frequent_root("file:///srv/media", now).unwrap());

        assert!(index.frequent_roots().unwrap().is_empty());
        assert_eq!(
            index.root_origin("file:///srv/media").unwrap(),
            Some(RootOrigin::User)
        );
        assert_eq!(
            index.skipped_frequent_folders(now + 29.0 * DAY).unwrap(),
            [PROJECTS]
        );
        assert!(index
            .skipped_frequent_folders(now + 31.0 * DAY)
            .unwrap()
            .is_empty());
        index.store_frequent_indexing(FrequentIndexing::Off).unwrap();
        assert!(index.skipped_frequent_folders(now).unwrap().is_empty());
    }

    /// parity: SRCH-041
    #[test]
    fn a_folder_beyond_the_frequent_entry_limits_is_skipped_for_thirty_days() {
        let base = tempfile::tempdir().unwrap();
        let large = folder_with_files(base.path(), "large", 3);
        let small = folder_with_files(base.path(), "small", 2);
        let more = folder_with_files(base.path(), "more", 2);
        let limits = ServiceLimits {
            entries_per_frequent_root: 2,
            frequent_entries: 3,
            ..ServiceLimits::default()
        };
        let service = start_with(&base, limits);
        let now = unix_now();

        // Larger than one root may be.
        open_often(&service, &[&large, &small], now);
        assert!(service.rebalance_frequent_roots(now).unwrap());
        wait_for_frequent_scans(&service, "the first scans end");
        assert_eq!(
            service.index().frequent_roots().unwrap(),
            std::slice::from_ref(&small)
        );
        assert_eq!(entry_count(&service, &small), 2);
        assert_eq!(service.index().root_origin(&large).unwrap(), None);
        // Larger than is left beside the other roots.
        open_often(&service, &[&more], now);
        assert!(service.rebalance_frequent_roots(now).unwrap());
        wait_for_frequent_scans(&service, "the second scan ends");
        assert_eq!(
            service.index().frequent_roots().unwrap(),
            std::slice::from_ref(&small)
        );
        assert_eq!(service.index().root_origin(&more).unwrap(), None);

        let mut skipped = service.index().skipped_frequent_folders(now).unwrap();
        skipped.sort();
        assert_eq!(skipped, [large.clone(), more]);
        open_often(&service, &[&large], now + DAY);
        assert!(!service.rebalance_frequent_roots(now + DAY).unwrap());
        // After 30 days, the folder is tried again.
        let later = now + 31.0 * DAY;
        open_often(&service, &[&large, &small], later);
        assert!(service.rebalance_frequent_roots(later).unwrap());
        service.close();
    }

    /// parity: SRCH-041
    #[test]
    fn no_folder_is_added_while_the_frequent_roots_hold_the_most_entries() {
        let base = tempfile::tempdir().unwrap();
        let small = folder_with_files(base.path(), "small", 2);
        let other = folder_with_files(base.path(), "other", 1);
        let limits = ServiceLimits {
            frequent_entries: 2,
            ..ServiceLimits::default()
        };
        let service = start_with(&base, limits);
        let now = unix_now();
        open_often(&service, &[&small], now);
        assert!(service.rebalance_frequent_roots(now).unwrap());
        wait_for_frequent_scans(&service, "the scan ends");
        assert_eq!(entry_count(&service, &small), 2);

        open_often(&service, &[&other], now + 600.0);

        assert!(!service.rebalance_frequent_roots(now + 600.0).unwrap());
        assert_eq!(service.index().frequent_roots().unwrap(), [small]);
        assert!(service.index().skipped_frequent_folders(now).unwrap().is_empty());
        service.close();
    }

    /// parity: SRCH-041
    #[test]
    fn a_folder_above_one_the_user_indexes_is_not_added() {
        let directory = tempfile::tempdir().unwrap();
        let service = IndexService::start(open_index(&directory), GioFolderReader, || {}).unwrap();
        service
            .index()
            .configure(
                &format!("{PROJECTS}/App"),
                Caching::Enabled,
                "",
                HiddenItems::Skip,
            )
            .unwrap();
        service
            .set_frequent_indexing(FrequentIndexing::Automatic)
            .unwrap();

        let added: Vec<bool> = [0.0, 1.0, 2.0, 3.0, 3.5]
            .iter()
            .map(|day| opened(&service, PROJECTS, day * DAY))
            .collect();
        assert_eq!(added, [false; 5]);
        assert!(service.index().frequent_roots().unwrap().is_empty());
    }

    /// parity: SRCH-041
    #[test]
    fn a_folder_opened_often_is_indexed_and_turning_off_forgets_it() {
        let directory = tempfile::tempdir().unwrap();
        // The folder does not exist, so its scan ends at once with an error.
        let service = IndexService::start(open_index(&directory), GioFolderReader, || {}).unwrap();
        let opens = [0.0, 1.0, 2.0, 3.0].map(|day| day * DAY);

        for now in opens {
            assert!(!opened(&service, PROJECTS, now));
        }
        assert!(service.index().visits().unwrap().is_empty());
        service
            .set_frequent_indexing(FrequentIndexing::Automatic)
            .unwrap();
        let added: Vec<bool> = opens.iter().map(|now| opened(&service, PROJECTS, *now)).collect();
        assert_eq!(added, [false, false, false, false]);
        assert!(opened(&service, PROJECTS, 3.5 * DAY));
        assert_eq!(service.index().frequent_roots().unwrap(), [PROJECTS]);
        assert!(!opened(&service, HOME, 3.5 * DAY));
        assert!(!opened(&service, "file:///mnt/team/Reports", 3.5 * DAY));

        service.set_frequent_indexing(FrequentIndexing::Off).unwrap();
        assert!(service.index().frequent_roots().unwrap().is_empty());
        assert!(service.index().visits().unwrap().is_empty());
        assert_eq!(service.index().root_origin(PROJECTS).unwrap(), None);
        service.close();
    }

    /// parity: SRCH-041
    #[test]
    fn nothing_is_counted_while_auto_index_is_paused() {
        let directory = tempfile::tempdir().unwrap();
        let service = IndexService::start(open_index(&directory), GioFolderReader, || {}).unwrap();
        service
            .set_frequent_indexing(FrequentIndexing::Automatic)
            .unwrap();
        let paused = IndexSettings {
            auto_index: AutoIndex::Paused,
            ..IndexSettings::default()
        };
        service.tick(&paused).unwrap();

        for day in 0..6 {
            assert!(!opened(&service, PROJECTS, f64::from(day) * DAY));
        }
        assert!(service.index().visits().unwrap().is_empty());
        assert!(!service.update_frequent_roots(AutoIndex::Paused).unwrap());
        service.close();
    }

    /// parity: SRCH-041
    #[test]
    fn a_folder_no_longer_opened_is_removed_when_the_service_starts() {
        let directory = tempfile::tempdir().unwrap();
        let service = IndexService::start(open_index(&directory), GioFolderReader, || {}).unwrap();
        service
            .set_frequent_indexing(FrequentIndexing::Automatic)
            .unwrap();
        // Opened often, two months ago.
        let long_ago = unix_now() - 60.0 * DAY;
        for day in 0..5 {
            opened(&service, PROJECTS, long_ago + f64::from(day) * DAY / 2.0);
        }
        assert_eq!(service.index().frequent_roots().unwrap(), [PROJECTS]);

        assert!(service.update_frequent_roots(AutoIndex::On).unwrap());
        assert!(service.index().frequent_roots().unwrap().is_empty());
        service.close();
    }

    /// parity: SRCH-041
    #[test]
    fn pinning_a_frequent_folder_makes_its_root_the_pins() {
        let directory = tempfile::tempdir().unwrap();
        let service = IndexService::start(open_index(&directory), GioFolderReader, || {}).unwrap();
        service
            .set_frequent_indexing(FrequentIndexing::Automatic)
            .unwrap();
        service.index().add_frequent_root(PROJECTS).unwrap();
        let pin = Bookmark {
            uri: PROJECTS.to_owned(),
            label: "Projects".to_owned(),
        };

        service.pin_added(&pin, PinIndexing::Automatic).unwrap();

        assert_eq!(
            service.index().root_origin(PROJECTS).unwrap(),
            Some(RootOrigin::Pin)
        );
        service.set_frequent_indexing(FrequentIndexing::Off).unwrap();
        assert_eq!(
            service.index().root_origin(PROJECTS).unwrap(),
            Some(RootOrigin::Pin)
        );
        service.close();
    }
}
