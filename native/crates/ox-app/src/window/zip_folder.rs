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

use std::sync::Arc;

use ox_core::archive::{default_preview_root, ArchiveBrowser, GioArchiveOpener};
use ox_core::entry::Entry;
use ox_core::location::ArchiveLocation;
use ox_core::settings::ZipOpening;
use ox_core::transfer::Cancellation;

use gtk::glib;
use gtk::subclass::prelude::*;

use crate::archive_view::ArchiveTarget;

use super::BrowserWindow;

/// Shown for a file inside a ZIP that is not opened from it.
const NOT_OPENABLE: &str = "This file cannot be opened from the ZIP. Copy it out or use Extract all.";

/// Shown when a file dialog meets an item inside a ZIP.
const NOT_IN_DIALOGS: &str = "Files inside a ZIP cannot be chosen here. Extract the ZIP first.";

/// Whether `entry` is a ZIP (by name or type), the one archive type that
/// opens like a folder; TAR archives keep the window.
fn is_zip(entry: &Entry) -> bool {
    entry.name.to_lowercase().ends_with(".zip")
        || entry.content_type.as_deref().is_some_and(|content_type| {
            matches!(content_type, "application/zip" | "application/x-zip-compressed")
        })
}

/// The location inside a ZIP that `uri` names, if it is one.
pub(super) fn archive_location(uri: &str) -> Option<ArchiveLocation> {
    ArchiveLocation::parse(uri, &glib::home_dir()).ok().flatten()
}

impl BrowserWindow {
    /// Whether opening `entry` shows it in the tab like a folder: a ZIP,
    /// with "Open ZIP files" set to "Like a folder".
    pub(super) fn opens_zip_as_folder(&self, entry: &Entry) -> bool {
        let preferences = self.context().settings_data().preferences;
        preferences.browse_archives && preferences.zip_opening == ZipOpening::Folder && is_zip(entry)
    }

    /// The location of the root of the ZIP `entry`, to navigate to.
    pub(super) fn zip_root_of(entry: &Entry) -> String {
        ArchiveLocation::root(entry.navigation_uri()).uri()
    }

    /// The ZIP whose contents the active tab shows, if it shows one.
    pub(super) fn shown_zip(&self) -> Option<ArchiveTarget> {
        let inside = archive_location(&self.current_uri()?)?;
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
        let Some(inside) = archive_location(&entry.uri) else {
            return false;
        };
        if inside.is_folder() {
            self.navigate_or_report(&entry.uri);
            return true;
        }
        if self.is_picking() {
            self.show_message(NOT_IN_DIALOGS);
            return true;
        }
        let browser = ArchiveBrowser::new(Arc::new(GioArchiveOpener), default_preview_root());
        glib::spawn_future_local(glib::clone!(
            #[weak(rename_to = window)]
            self,
            async move {
                let copy = browser
                    .preview_member_in_background(inside.archive_uri, inside.member, Cancellation::new())
                    .await;
                match copy {
                    Ok(copy) => window.open_externally(&copy.uri()),
                    Err(ox_core::archive::ArchiveError::UnsafePreviewType) => {
                        window.show_message(NOT_OPENABLE);
                    }
                    Err(error) => window.show_message(&error.to_string()),
                }
            }
        ));
        true
    }
}
