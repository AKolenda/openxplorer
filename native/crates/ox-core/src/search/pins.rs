// SPDX-License-Identifier: AGPL-3.0-only
//! Pinned folders are indexed automatically (SRCH-040).
//!
//! A new behaviour the product owner decided on 2026-09-28; the Python app
//! does not have it. Pinning a folder to Quick access adds it to the
//! search cache as a root marked as added by the pin (the `pin_added`
//! column, shown as [`RootOrigin::Pin`]). Unpinning removes only such
//! roots: a folder the user chose to index, or switched off, stays as the
//! user left it.
//!
//! The app calls [`IndexService::pin_added`] and
//! [`IndexService::pin_removed`] when a pin changes,
//! [`IndexService::index_existing_pins`] once at start-up, and
//! [`IndexService::set_pin_indexing`] when the "Index pinned folders
//! automatically" switch changes; [`SearchIndex::pin_indexing`] reads the
//! switch back.
//!
//! The switch is kept in the cache (the `index_options` table), not in
//! `settings.json`: both apps rewrite the settings file with only the keys
//! they know, so the Python app would drop it, and the roots it governs
//! are stored here too.
//!
//! Phones, cameras and server share lists cannot be indexed, so pinning
//! them adds nothing, and neither does pinning a folder an enabled root
//! already indexes. A pinned share that needs sign-in fails its first
//! scan without mounting anything, because the crawler never mounts, and
//! is scanned again when the user signs in
//! ([`IndexService::resume_server`]).

use rusqlite::OptionalExtension;

use super::error::SearchError;
use super::index::{begin_immediate, indexable_root, root_label, SearchIndex};
use super::policy::IndexScope;
use super::root::{HiddenItems, IndexRoot, RootOrigin};
use super::service::{IndexService, ScanTrigger};
use super::text::{folder_prefix, is_at_or_below, unix_now};
use crate::location::normalise;
use crate::settings::Bookmark;

/// The one-time indexing of the folders that were pinned before pinned
/// folders were indexed automatically.
const EXISTING_PINS_MIGRATION: &str = "index-existing-pins";

/// The row of `index_options` that holds the switch.
const PIN_INDEXING_OPTION: &str = "pin-indexing";

/// The "Index pinned folders automatically" switch in the Search &
/// indexing settings; on by default.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PinIndexing {
    /// Pinned folders are indexed.
    #[default]
    Automatic,
    /// Pinning does not change the search cache.
    Off,
}

impl PinIndexing {
    /// The word `index_options` stores.
    const fn as_str(self) -> &'static str {
        match self {
            Self::Automatic => "automatic",
            Self::Off => "off",
        }
    }

    /// The switch a stored word names; a word this version does not write
    /// reads as the default, on.
    fn from_stored(word: &str) -> Self {
        if word == Self::Off.as_str() {
            Self::Off
        } else {
            Self::Automatic
        }
    }
}

impl IndexService {
    /// Indexes a folder that was just pinned to Quick access and starts
    /// its scan. Nothing changes when the switch is off, when the folder
    /// already has a root (enabled or switched off by the user) or lies
    /// inside an enabled root that indexes it, or when it cannot be
    /// indexed.
    ///
    /// # Errors
    ///
    /// [`SearchError::Location`] for an invalid address, and database
    /// errors.
    pub fn pin_added(&self, pin: &Bookmark, indexing: PinIndexing) -> Result<(), SearchError> {
        if indexing == PinIndexing::Off {
            return Ok(());
        }
        let Some(root) = self.index().add_pinned_root(pin)? else {
            return Ok(());
        };
        self.scan_pinned_root(&root)
    }

    /// Stops indexing a folder that was just unpinned, if pinning added
    /// it, and deletes its cached entries.
    ///
    /// # Errors
    ///
    /// [`SearchError::Location`] for an invalid address, and database
    /// errors.
    pub fn pin_removed(&self, uri: &str) -> Result<(), SearchError> {
        let uri = normalise(uri)?;
        if self.index().root_origin(&uri)? != Some(RootOrigin::Pin) {
            return Ok(());
        }
        self.stop(&uri)?;
        self.index().remove_pinned_root(&uri)
    }

    /// Indexes, in the background, the folders that were pinned before
    /// pinned folders were indexed automatically: pins made by an earlier
    /// version or by the Python app. Runs once per cache; later start-ups
    /// change nothing, so a root the user removed stays removed.
    ///
    /// # Errors
    ///
    /// Database errors. Pins that cannot be indexed are skipped.
    pub fn index_existing_pins(&self, pins: &[Bookmark], indexing: PinIndexing) -> Result<(), SearchError> {
        if self.index().is_migration_applied(EXISTING_PINS_MIGRATION)? {
            return Ok(());
        }
        if indexing == PinIndexing::Automatic {
            self.index_every_pin(pins)?;
        }
        // Safety rule "a root the user removed stays removed": with the
        // switch off the change counts as applied too, because turning the
        // switch on indexes every pin itself. Otherwise a later start-up
        // would add back a pinned root the user removed in between.
        self.index().mark_migration_applied(EXISTING_PINS_MIGRATION)
    }

    /// Follows a change of the "Index pinned folders automatically"
    /// switch and remembers it for [`SearchIndex::pin_indexing`]. Turning
    /// it on indexes every pinned folder that has no root; turning it off
    /// removes the roots pinning added, while roots the user chose stay.
    ///
    /// # Errors
    ///
    /// Database errors. Pins that cannot be indexed are skipped.
    pub fn set_pin_indexing(&self, pins: &[Bookmark], indexing: PinIndexing) -> Result<(), SearchError> {
        self.index().store_pin_indexing(indexing)?;
        match indexing {
            PinIndexing::Automatic => {
                self.index_every_pin(pins)?;
                // Safety rule "a root the user removed stays removed": every
                // pin is indexed now, so the start-up change of
                // `index_existing_pins` must never run after this.
                self.index().mark_migration_applied(EXISTING_PINS_MIGRATION)
            }
            PinIndexing::Off => self.remove_pinned_roots(),
        }
    }

    /// Adds a root for every pin that has none and starts its scan.
    fn index_every_pin(&self, pins: &[Bookmark]) -> Result<(), SearchError> {
        for pin in pins {
            match self.index().add_pinned_root(pin) {
                Ok(Some(root)) => self.scan_pinned_root(&root)?,
                // A pin saved by an older version may no longer be a valid
                // location; it cannot be indexed, and the other pins can.
                Ok(None) | Err(SearchError::Location(_)) => {}
                Err(error) => return Err(error),
            }
        }
        Ok(())
    }

    /// Starts the first scan of `root`, which pinning just added.
    ///
    /// Safety rule "no scan while signing out" (`refresh(explicit=False)`
    /// in `index_service.py`): pinning does not ask for a scan, so it never
    /// resumes a server paused for sign-out; the root is scanned once the
    /// user signs in. Another process queues nothing, because the owner
    /// scans every root it has not scanned yet on its next tick.
    fn scan_pinned_root(&self, root: &str) -> Result<(), SearchError> {
        self.start_scan(root, ScanTrigger::Automatic)?;
        Ok(())
    }

    /// Stops indexing and removes every root pinning added.
    fn remove_pinned_roots(&self) -> Result<(), SearchError> {
        let roots = self.index().roots()?;
        let pinned = roots.iter().filter(|root| root.origin == RootOrigin::Pin);
        for root in pinned {
            self.stop(&root.uri)?;
            self.index().remove_pinned_root(&root.uri)?;
        }
        Ok(())
    }
}

impl SearchIndex {
    /// The "Index pinned folders automatically" switch as last set in any
    /// process; on until the user turns it off.
    ///
    /// # Errors
    ///
    /// Database errors.
    pub fn pin_indexing(&self) -> Result<PinIndexing, SearchError> {
        let connection = self.connect()?;
        let stored: Option<String> = connection
            .query_row(
                "SELECT value FROM index_options WHERE name=?1",
                [PIN_INDEXING_OPTION],
                |row| row.get(0),
            )
            .optional()?;
        let indexing = stored.as_deref().map(PinIndexing::from_stored);
        Ok(indexing.unwrap_or_default())
    }

    /// Remembers the switch for [`SearchIndex::pin_indexing`].
    fn store_pin_indexing(&self, indexing: PinIndexing) -> Result<(), SearchError> {
        let connection = self.connect()?;
        connection.execute(
            "INSERT INTO index_options(name, value) VALUES(?1, ?2)
             ON CONFLICT(name) DO UPDATE SET value=excluded.value",
            (PIN_INDEXING_OPTION, indexing.as_str()),
        )?;
        Ok(())
    }

    /// Adds `pin` as an enabled root marked as added by the pin, and
    /// returns its URI; `None` when the folder cannot be indexed, already
    /// has a root, or an enabled root already indexes it.
    ///
    /// Safety rule "the user's choice wins": an existing root, including
    /// one the user switched off, is left exactly as it is.
    fn add_pinned_root(&self, pin: &Bookmark) -> Result<Option<String>, SearchError> {
        let uri = match indexable_root(&pin.uri) {
            Ok(uri) => uri,
            Err(SearchError::DeviceLocation | SearchError::ServerList) => return Ok(None),
            Err(error) => return Err(error),
        };
        if self.is_indexed_by_enabled_root(&uri)? {
            return Ok(None);
        }
        let label = root_label(&pin.label, &uri);
        let connection = self.connect()?;
        let added = connection.execute(
            "INSERT INTO roots(uri, label, enabled, include_hidden, pin_added) VALUES(?1, ?2, 1, 0, 1)
             ON CONFLICT(uri) DO NOTHING",
            (&uri, &label),
        )?;
        Ok((added == 1).then_some(uri))
    }

    /// Whether an enabled root already indexes the folder `uri`.
    ///
    /// Safety rule "a folder is indexed once" (SRCH-032): a root for a
    /// folder inside another root would crawl, watch and store that
    /// subtree twice, and count it twice toward the entry limit.
    fn is_indexed_by_enabled_root(&self, uri: &str) -> Result<bool, SearchError> {
        let roots = self.roots()?;
        let is_indexed = roots
            .iter()
            .filter(|root| root.is_enabled() && is_at_or_below(uri, &root.uri))
            .any(|root| self.root_reaches(root, uri));
        Ok(is_indexed)
    }

    /// Whether `root`, at or above the folder `uri`, indexes it: `uri` is
    /// in the root's scope (not another filesystem, a system folder or
    /// snapshot history) and not hidden from a root that skips hidden
    /// items.
    fn root_reaches(&self, root: &IndexRoot, uri: &str) -> bool {
        // Without the mount table, other filesystems cannot be told apart;
        // the pin then gets a root of its own, whose scan reports why.
        let Ok(scope) = IndexScope::current(&root.uri, self.directory()) else {
            return false;
        };
        let is_hidden_from_root =
            root.hidden_items == HiddenItems::Skip && has_hidden_name_below(&root.uri, uri);
        scope.admits(uri) && !is_hidden_from_root
    }

    /// Who chose to index `uri`; `None` when it has no root.
    fn root_origin(&self, uri: &str) -> Result<Option<RootOrigin>, SearchError> {
        let connection = self.connect()?;
        let pin_added: Option<bool> = connection
            .query_row("SELECT pin_added FROM roots WHERE uri=?1", [uri], |row| {
                row.get(0)
            })
            .optional()?;
        Ok(pin_added.map(RootOrigin::from_stored))
    }

    /// Deletes the root `uri` and its entries if pinning added it.
    ///
    /// Safety rule "the user's choice wins": the check and the deletion
    /// are one transaction, so a root the user chose in the meantime
    /// stays.
    fn remove_pinned_root(&self, uri: &str) -> Result<(), SearchError> {
        let mut connection = self.connect()?;
        let transaction = begin_immediate(&mut connection)?;
        let removed = transaction.execute("DELETE FROM roots WHERE uri=?1 AND pin_added=1", [uri])?;
        if removed == 1 {
            transaction.execute("DELETE FROM entries WHERE root=?1", [uri])?;
        }
        transaction.commit()?;
        Ok(())
    }

    /// Whether the one-time change `name` was applied to this cache.
    fn is_migration_applied(&self, name: &str) -> Result<bool, SearchError> {
        let connection = self.connect()?;
        let applied: Option<f64> = connection
            .query_row(
                "SELECT applied FROM index_migrations WHERE name=?1",
                [name],
                |row| row.get(0),
            )
            .optional()?;
        Ok(applied.is_some())
    }

    /// Records that the one-time change `name` was applied.
    fn mark_migration_applied(&self, name: &str) -> Result<(), SearchError> {
        let connection = self.connect()?;
        connection.execute(
            "INSERT OR IGNORE INTO index_migrations(name, applied) VALUES(?1, ?2)",
            (name, unix_now()),
        )?;
        Ok(())
    }
}

/// Whether a folder on the way from `root` down to `uri`, `uri` included,
/// has a hidden name, one that starts with a dot. A name that a `.hidden`
/// file hides is not known without reading that file, so such a folder
/// counts as reached.
fn has_hidden_name_below(root: &str, uri: &str) -> bool {
    let Some(relative) = uri.strip_prefix(&folder_prefix(root)) else {
        return false;
    };
    relative.split('/').any(|name| name.starts_with('.'))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::search::fixtures::open_index;
    use crate::search::root::Caching;

    /// A pin below an existing root, and whether the pin gets a root of
    /// its own.
    struct NestedPinCase {
        root: &'static str,
        caching: Caching,
        hidden_items: HiddenItems,
        pin: &'static str,
        gets_own_root: bool,
    }

    /// The cases of [`a_pin_gets_a_root_only_when_no_enabled_root_indexes_it`].
    const NESTED_PINS: [NestedPinCase; 5] = [
        NestedPinCase {
            root: "smb://nas/share",
            caching: Caching::Enabled,
            hidden_items: HiddenItems::Skip,
            pin: "smb://nas/share/Team",
            gets_own_root: false,
        },
        NestedPinCase {
            root: "smb://nas/share",
            caching: Caching::Disabled,
            hidden_items: HiddenItems::Skip,
            pin: "smb://nas/share/Team",
            gets_own_root: true,
        },
        NestedPinCase {
            root: "smb://nas/share",
            caching: Caching::Enabled,
            hidden_items: HiddenItems::Skip,
            pin: "smb://nas/share/.private/Team",
            gets_own_root: true,
        },
        NestedPinCase {
            root: "smb://nas/share",
            caching: Caching::Enabled,
            hidden_items: HiddenItems::Include,
            pin: "smb://nas/share/.private/Team",
            gets_own_root: false,
        },
        // Whole-disk exclusions leave /var/tmp out of a /var root.
        NestedPinCase {
            root: "file:///var",
            caching: Caching::Enabled,
            hidden_items: HiddenItems::Skip,
            pin: "file:///var/tmp/project",
            gets_own_root: true,
        },
    ];

    /// Safety rule "a folder is indexed once", and the folders an enabled
    /// root does not reach: below a switched-off root, hidden from a root
    /// that skips hidden items, or excluded as a system folder.
    ///
    /// parity: SRCH-040
    #[test]
    fn a_pin_gets_a_root_only_when_no_enabled_root_indexes_it() {
        for case in NESTED_PINS {
            let directory = tempfile::tempdir().unwrap();
            let index = open_index(&directory);
            index
                .configure(case.root, case.caching, "", case.hidden_items)
                .unwrap();
            let pin = Bookmark {
                uri: case.pin.to_owned(),
                label: String::new(),
            };

            let added = index.add_pinned_root(&pin).unwrap();

            let what = format!("{} in {}", case.pin, case.root);
            assert_eq!(added.is_some(), case.gets_own_root, "{what}");
        }
    }

    /// The switch is on in a new cache, and every handle on the cache,
    /// such as another process's, reads the choice last stored.
    ///
    /// parity: SRCH-040
    #[test]
    fn the_switch_is_on_until_turned_off_and_is_remembered() {
        let directory = tempfile::tempdir().unwrap();
        let index = open_index(&directory);
        assert_eq!(index.pin_indexing().unwrap(), PinIndexing::Automatic);

        index.store_pin_indexing(PinIndexing::Off).unwrap();

        let other_process = SearchIndex::open(directory.path()).unwrap();
        assert_eq!(other_process.pin_indexing().unwrap(), PinIndexing::Off);
        index.store_pin_indexing(PinIndexing::Automatic).unwrap();
        assert_eq!(other_process.pin_indexing().unwrap(), PinIndexing::Automatic);
    }

    #[test]
    fn hidden_names_count_only_below_the_root() {
        assert!(has_hidden_name_below("smb://nas/share", "smb://nas/share/a/.b/c"));
        assert!(!has_hidden_name_below("smb://nas/.share", "smb://nas/.share/a"));
        assert!(!has_hidden_name_below("smb://nas/share", "smb://nas/share"));
    }
}
