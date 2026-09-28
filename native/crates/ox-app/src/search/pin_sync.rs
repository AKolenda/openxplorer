// SPDX-License-Identifier: AGPL-3.0-only
//! Which Quick access pins appeared and disappeared, so the index service
//! hears about each one (SRCH-040).
//!
//! New behaviour decided by the owner on 2026-09-28; the Python app does
//! not have it. Pins change through the shared settings file, from this
//! window, another window, another process or the Python app, and every
//! change reaches the app as `places-changed`. [`PinnedFolders`] compares
//! the pins of each reading with the previous one, so a pin is reported
//! once however it was made; the service's own rules decide what indexing
//! it causes.

use ox_core::location::same_location;
use ox_core::settings::Bookmark;

/// Whether the settings file was read as it is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SettingsReading {
    /// The file was read, or is missing: its pins are the user's.
    Sound,
    /// The file could not be read and the app fell back to the defaults,
    /// which have no pins.
    FellBackToDefaults,
}

/// What changed in Quick access since the previous reading.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct PinChanges {
    /// Folders pinned since then.
    pub added: Vec<Bookmark>,
    /// The URIs of folders unpinned since then.
    pub removed: Vec<String>,
}

impl PinChanges {
    /// Whether nothing changed.
    pub(crate) fn is_empty(&self) -> bool {
        self.added.is_empty() && self.removed.is_empty()
    }
}

/// The pins as last read.
#[derive(Debug, Default)]
pub(crate) struct PinnedFolders {
    /// `None` until the first reading, which only records them: the pins
    /// made before the app started are indexed by the service's one-time
    /// start-up step instead.
    known: Option<Vec<Bookmark>>,
}

impl PinnedFolders {
    /// Records `pins`, read with `reading`, and returns what changed since
    /// the previous reading.
    pub(crate) fn update(&mut self, pins: &[Bookmark], reading: SettingsReading) -> PinChanges {
        // Safety rule "an unreadable settings file unpins nothing": the
        // defaults the app falls back to have no pins, and reading them as
        // unpinning would delete the index of every pinned folder.
        if reading == SettingsReading::FellBackToDefaults {
            return PinChanges::default();
        }
        let Some(known) = self.known.replace(pins.to_vec()) else {
            return PinChanges::default();
        };
        let added = pins
            .iter()
            .filter(|pin| !is_listed(&known, &pin.uri))
            .cloned()
            .collect();
        let removed = known
            .iter()
            .filter(|pin| !is_listed(pins, &pin.uri))
            .map(|pin| pin.uri.clone())
            .collect();
        PinChanges { added, removed }
    }
}

/// Whether `pins` holds a pin of `uri`.
fn is_listed(pins: &[Bookmark], uri: &str) -> bool {
    pins.iter().any(|pin| same_location(&pin.uri, uri))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pin(uri: &str) -> Bookmark {
        Bookmark {
            uri: uri.to_owned(),
            label: String::new(),
        }
    }

    /// parity: SRCH-040
    #[test]
    fn a_pin_is_reported_once_when_it_appears_and_once_when_it_goes() {
        let mut pinned = PinnedFolders::default();
        let documents = pin("file:///home/demo/Documents");
        let team = pin("smb://nas/team");
        pinned.update(std::slice::from_ref(&documents), SettingsReading::Sound);

        let pinning = pinned.update(&[documents.clone(), team.clone()], SettingsReading::Sound);
        let unchanged = pinned.update(&[documents.clone(), team.clone()], SettingsReading::Sound);
        let unpinning = pinned.update(std::slice::from_ref(&team), SettingsReading::Sound);

        assert_eq!(pinning.added, [team]);
        assert!(pinning.removed.is_empty());
        assert!(unchanged.is_empty());
        assert_eq!(unpinning.removed, [documents.uri]);
        assert!(unpinning.added.is_empty());
    }

    #[test]
    fn the_pins_at_start_up_are_only_recorded() {
        let mut pinned = PinnedFolders::default();

        let first = pinned.update(&[pin("file:///home/demo/Work")], SettingsReading::Sound);

        assert!(first.is_empty());
    }

    /// Safety rule "an unreadable settings file unpins nothing".
    ///
    /// parity: SRCH-040
    #[test]
    fn settings_that_fell_back_to_defaults_unpin_nothing() {
        let mut pinned = PinnedFolders::default();
        let work = pin("file:///home/demo/Work");
        pinned.update(std::slice::from_ref(&work), SettingsReading::Sound);

        let unreadable = pinned.update(&[], SettingsReading::FellBackToDefaults);
        let readable_again = pinned.update(std::slice::from_ref(&work), SettingsReading::Sound);

        assert!(unreadable.is_empty());
        assert!(readable_again.is_empty(), "the pin was never gone");
    }
}
