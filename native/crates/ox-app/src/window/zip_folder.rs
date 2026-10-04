// SPDX-License-Identifier: AGPL-3.0-only
//! A ZIP opened like a folder, as Windows Explorer opens a "Compressed
//! (zipped) Folder" (ARC-026).
//!
//! With Settings › Windows & tabs › "Open ZIP files" set to "Like a
//! folder", opening a ZIP shows it in the tab at an `ox-zip:` location
//! (see [`ox_core::location::ArchiveLocation`]): the address bar, Back,
//! Forward and Up, tabs and views work as in any folder; the folder model
//! lists the members the archive reader lists, read-only. A folder inside
//! opens in the tab; a file opens as the read-only private copy the ZIP
//! window already makes. Extract all in the command bar extracts the ZIP
//! whose contents are shown, and Copy and dragging hand out real copies of
//! the selected items (see `zip_copies.rs`). The other choice, "In a
//! pop-up window", keeps the "Compressed folder" window.

use ox_core::entry::Entry;
use ox_core::location::ArchiveLocation;
use ox_core::settings::ZipOpening;
use ox_core::transfer::Cancellation;

use gtk::glib;
use gtk::subclass::prelude::*;

use crate::archive_view::{archive_browser, ArchiveTarget};
use crate::icons;

use super::BrowserWindow;

/// Shown for a file inside a ZIP that is not opened from it.
const NOT_OPENABLE: &str =
    crate::i18n::message_id("This file cannot be opened from the ZIP. Copy it out or use Extract all.");

/// Shown when a file dialog meets an item inside a ZIP.
const NOT_IN_DIALOGS: &str =
    crate::i18n::message_id("Files inside a ZIP cannot be chosen here. Extract the ZIP first.");

/// Whether `entry` is a ZIP, by name or type.
fn is_zip(entry: &Entry) -> bool {
    icons::is_zip(&entry.name, entry.content_type.as_deref())
}

impl BrowserWindow {
    /// Whether opening a ZIP shows it in the tab like a folder: "Open
    /// archives as folders" is on and "Open ZIP files" is "Like a folder".
    pub(super) fn opens_zips_as_folders(&self) -> bool {
        self.context().settings_data().preferences.browse_archives && self.browses_zips_as_folders()
    }

    /// Whether opening `entry` shows it in the tab like a folder: a ZIP
    /// (by name or type, the one archive type that opens like a folder;
    /// TAR archives keep the window), with [`Self::opens_zips_as_folders`].
    pub(super) fn opens_zip_as_folder(&self, entry: &Entry) -> bool {
        self.opens_zips_as_folders() && is_zip(entry)
    }

    /// Whether the ZIP `entry`, browsed whatever "Open archives as
    /// folders" says because another app handed it to this one, shows in
    /// the tab like a folder: "Open ZIP files" is "Like a folder".
    pub(super) fn browses_zip_as_folder(&self, entry: &Entry) -> bool {
        self.browses_zips_as_folders() && is_zip(entry)
    }

    /// Whether a browsed ZIP shows in the tab: "Open ZIP files" is "Like a
    /// folder".
    fn browses_zips_as_folders(&self) -> bool {
        self.context().settings_data().preferences.zip_opening == ZipOpening::Folder
    }

    /// The location of the root of the ZIP `entry`, to navigate to.
    pub(super) fn zip_root_of(entry: &Entry) -> String {
        ArchiveLocation::root(entry.navigation_uri()).uri()
    }

    /// The ZIP whose contents the active tab shows, if it shows one.
    pub(super) fn shown_zip(&self) -> Option<ArchiveTarget> {
        let inside = ArchiveLocation::from_uri(&self.current_uri()?)?;
        let name = self.imp().locations.borrow().base_name(&inside.archive_uri);
        Some(ArchiveTarget {
            uri: inside.archive_uri,
            name,
        })
    }

    /// Opens `entry`, an item listed inside a ZIP: a folder in the tab, a
    /// file as a read-only private copy in its application. True when
    /// `entry` is such an item and was handled here.
    pub(super) fn activate_zip_member(&self, entry: &Entry) -> bool {
        let Some(inside) = ArchiveLocation::from_uri(&entry.uri) else {
            return false;
        };
        if inside.is_folder() {
            self.navigate_or_report(&entry.uri);
            return true;
        }
        if self.is_picking() {
            self.show_message(ox_core::i18n::gettext_static(NOT_IN_DIALOGS));
            return true;
        }
        let browser = archive_browser();
        glib::spawn_future_local(glib::clone!(
            #[weak(rename_to = window)]
            self,
            async move {
                let copy = browser
                    .preview_member_in_background(inside.archive_uri, inside.member, Cancellation::new())
                    .await;
                match copy {
                    Ok(copy) => {
                        window.open_externally(&copy.uri());
                        remove_copy_later(copy.path);
                    }
                    Err(ox_core::archive::ArchiveError::UnsafePreviewType) => {
                        window.show_message(ox_core::i18n::gettext_static(NOT_OPENABLE));
                    }
                    Err(error) => window.show_message(&error.to_string()),
                }
            }
        ));
        true
    }
}

/// Removes the copy at `path`, opened from an archive in another
/// application, once that application has had time to read it
/// ([`ox_core::archive::PREVIEW_LIFETIME`]). The copies are in the runtime
/// folder, which lives in memory until logout; on Linux removing a file
/// an application has open does not disturb it (ARC-026).
pub(super) fn remove_copy_later(path: std::path::PathBuf) {
    glib::timeout_add_local_once(copy_lifetime(), move || {
        ox_core::archive::remove_preview_copy(&path);
    });
}

#[cfg(test)]
thread_local! {
    /// How long tests keep copies, `None` for the real lifetime.
    static TEST_LIFETIME: std::cell::Cell<Option<std::time::Duration>> = const { std::cell::Cell::new(None) };
}

/// Keeps copies opened from archives for `lifetime` instead of the real
/// lifetime, or the real one again with `None`, for tests.
#[cfg(test)]
pub(crate) fn keep_copies_for_tests(lifetime: Option<std::time::Duration>) {
    TEST_LIFETIME.with(|shown| shown.set(lifetime));
}

/// How long a copy opened from an archive is kept.
fn copy_lifetime() -> std::time::Duration {
    #[cfg(test)]
    if let Some(lifetime) = TEST_LIFETIME.with(std::cell::Cell::get) {
        return lifetime;
    }
    ox_core::archive::PREVIEW_LIFETIME
}
