// SPDX-License-Identifier: AGPL-3.0-only
//! Mirroring the Quick access pins into the desktop's places list
//! (SIDE-013), so a folder pinned here also appears in GTK's open and save
//! dialogs and in GNOME Files.
//!
//! The application turns it on for the user's own list
//! ([`AppContext::export_pins_to`]); tests turn it on for a temporary file
//! only. Each time the places change, the pins added since the last time
//! are appended and the pins removed are dropped; the first time, every
//! pin the list lacks is added. Other lines of the list are never touched.
//! The file is written on a worker thread.

use std::path::PathBuf;

use gtk::gio;
use gtk::subclass::prelude::*;
use ox_core::location::same_location;
use ox_core::places::sync_bookmarks;
use ox_core::settings::Bookmark;

use super::AppContext;

/// The pins of `now` that `before` lacks, and the locations of `before`
/// that `now` lacks; every pin is new when there was no `before`.
fn pin_changes(before: Option<&[Bookmark]>, now: &[Bookmark]) -> (Vec<Bookmark>, Vec<String>) {
    let Some(before) = before else {
        return (now.to_vec(), Vec::new());
    };
    let is_in = |list: &[Bookmark], uri: &str| list.iter().any(|pin| same_location(&pin.uri, uri));
    let added = now
        .iter()
        .filter(|pin| !is_in(before, &pin.uri))
        .cloned()
        .collect();
    let removed = before
        .iter()
        .filter(|pin| !is_in(now, &pin.uri))
        .map(|pin| pin.uri.clone())
        .collect();
    (added, removed)
}

impl AppContext {
    /// Mirrors the pins into the places list at `bookmarks` from now on,
    /// starting with every pin it lacks.
    pub(crate) fn export_pins_to(&self, bookmarks: PathBuf) {
        self.imp().desktop_bookmarks.replace(Some(bookmarks));
        self.imp().exported_pins.replace(None);
        self.export_pins();
    }

    /// Brings the places list up to date with the pins, if it is mirrored.
    pub(super) fn export_pins(&self) {
        let Some(path) = self.imp().desktop_bookmarks.borrow().clone() else {
            return;
        };
        let pins = self.settings_data().pins;
        let before = self.imp().exported_pins.replace(Some(pins.clone()));
        let (added, removed) = pin_changes(before.as_deref(), &pins);
        if added.is_empty() && removed.is_empty() {
            return;
        }
        // A list that cannot be written only loses the mirror, never a
        // pin, which stays in OpenXplorer's settings.
        gio::spawn_blocking(move || {
            let _ = sync_bookmarks(&path, &added, &removed);
        });
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use ox_core::settings::{BookmarkAction, BookmarkKind, BookmarkRequest, Settings};

    use crate::test_support::harness::{wait_until, Fixture, TestWindow};

    /// parity: SIDE-013
    #[gtk::test]
    fn pins_are_mirrored_into_the_desktop_places_list() {
        let fixture = Fixture::standard();
        let test = TestWindow::open(&fixture.uri());
        let list = fixture.path("gtk-3.0").join("bookmarks");
        fs::create_dir(fixture.path("gtk-3.0")).expect("the list's folder");
        fs::write(&list, "sftp://build/srv Build server\n").expect("another app's bookmark");
        test.context.export_pins_to(list.clone());

        test.activate("pin-folder", None);
        let pinned = format!("{} Example projects", fixture.uri());
        wait_until("the pin in the list", || {
            fs::read_to_string(&list).is_ok_and(|text| text.contains(&pinned))
        });
        let mut settings = Settings::open(test.settings_directory());
        let request = BookmarkRequest::new(fixture.uri(), String::new());
        settings
            .bookmark(BookmarkAction::Remove, BookmarkKind::Pin, &request)
            .expect("unpinned");
        test.activate("refresh", None);

        wait_until("the pin to leave the list", || {
            fs::read_to_string(&list).is_ok_and(|text| text == "sftp://build/srv Build server\n")
        });
    }
}
