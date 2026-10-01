// SPDX-License-Identifier: AGPL-3.0-only
//! Mirroring the Quick access pins into the desktop's places list
//! (SIDE-013), so a folder pinned here also appears in GTK's open and save
//! dialogs and in GNOME Files.
//!
//! The application turns it on for the user's own list
//! ([`AppContext::export_pins_to`]); tests turn it on for a temporary file
//! only. Each time the pins change, one worker thread brings the list up
//! to date with them: it appends the pins the list lacks and drops only
//! the lines the app added for pins since removed, which it records in
//! `desktop-bookmarks` in the settings folder. Lines the user or other
//! apps wrote are never removed. Changes reach the worker in order and it
//! skips to the newest, so a quick series of changes never loses one.

use std::path::PathBuf;
use std::sync::mpsc;

use gtk::subclass::prelude::*;
use ox_core::places::sync_bookmarks;
use ox_core::settings::Bookmark;

use super::AppContext;

/// The file in the settings folder listing the lines the app added.
const OWNED_LINES_FILE: &str = "desktop-bookmarks";

/// Starts the worker that keeps the list at `list` up to date with the
/// pins it is sent, recording its own lines in `owned`. It ends when the
/// sender is dropped.
fn start_mirror(list: PathBuf, owned: PathBuf) -> mpsc::Sender<Vec<Bookmark>> {
    let (sender, receiver) = mpsc::channel::<Vec<Bookmark>>();
    // Without a thread the sends fail quietly and the mirror is off.
    let _ = std::thread::Builder::new()
        .name("ox-places-mirror".into())
        .spawn(move || {
            while let Ok(mut pins) = receiver.recv() {
                while let Ok(newer) = receiver.try_recv() {
                    pins = newer;
                }
                // A list that cannot be written only loses the mirror,
                // never a pin, which stays in OpenXplorer's settings.
                let _ = sync_bookmarks(&list, &owned, &pins);
            }
        });
    sender
}

impl AppContext {
    /// Mirrors the pins into the places list at `bookmarks` from now on,
    /// starting with every pin it lacks.
    pub(crate) fn export_pins_to(&self, bookmarks: PathBuf) {
        let owned = self.settings_directory().join(OWNED_LINES_FILE);
        self.imp()
            .bookmarks_mirror
            .replace(Some(start_mirror(bookmarks, owned)));
        self.imp().exported_pins.replace(None);
        self.export_pins();
    }

    /// Brings the places list up to date with the pins, if it is mirrored.
    pub(super) fn export_pins(&self) {
        let imp = self.imp();
        let Some(mirror) = imp.bookmarks_mirror.borrow().clone() else {
            return;
        };
        let pins = self.settings_data().pins;
        if imp.exported_pins.borrow().as_ref() == Some(&pins) {
            return;
        }
        imp.exported_pins.replace(Some(pins.clone()));
        let _ = mirror.send(pins);
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
        let users_line = format!("{} My projects\n", fixture.uri());
        fs::write(&list, format!("sftp://build/srv Build server\n{users_line}")).expect("other bookmarks");
        test.context.export_pins_to(list.clone());

        test.activate("pin-folder", None);
        let pinned = format!("{} Example projects", fixture.uri());
        let mut settings = Settings::open(test.settings_directory());
        let request = BookmarkRequest::new(fixture.uri(), String::new());
        settings
            .bookmark(BookmarkAction::Remove, BookmarkKind::Pin, &request)
            .expect("unpinned");
        test.activate("refresh", None);
        let folder = fixture.uri_of("Documents");
        let request = BookmarkRequest::new(folder.clone(), String::new());
        settings
            .bookmark(BookmarkAction::Add, BookmarkKind::Pin, &request)
            .expect("pinned");
        test.activate("refresh", None);

        wait_until("the new pin in the list", || {
            fs::read_to_string(&list).is_ok_and(|text| text.contains(&folder))
        });
        let text = fs::read_to_string(&list).expect("the list");
        assert!(text.starts_with("sftp://build/srv Build server\n"), "{text}");
        assert!(
            text.contains(&users_line),
            "the user's line for the unpinned folder stays: {text}"
        );
        assert!(!text.contains(&pinned), "{text}");
    }
}
