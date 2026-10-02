// SPDX-License-Identifier: AGPL-3.0-only
//! Recent locations: the folders visited lately, newest first, listed
//! like a folder (SIDE-026).
//!
//! Dolphin lists them from the desktop's activity history
//! (`recentlyused:/locations`). The native app reads the desktop's
//! recently used list, `recently-used.xbel`, where every folder it lists
//! after a navigation is recorded (OPEN-025), through `GtkRecentManager`.
//! That honours the desktop's privacy settings: with "remember recent
//! files" off the list is empty, and entries older than the desktop's
//! maximum age are left out. "Clear recent locations" removes the folders
//! from that list and leaves the recent files alone.

use gtk::prelude::*;
use gtk::{gio, glib};
use ox_core::entry::{self, Entry, EntryError};
use ox_core::integration::FOLDER_CONTENT_TYPE;

/// The most folders the list shows.
const MAX_RECENT_LOCATIONS: usize = 50;

/// Whether the desktop lets applications remember recently used items,
/// and for how many days; a negative age keeps them for ever.
fn recent_history_policy() -> (bool, i32) {
    gtk::Settings::default().map_or((true, -1), |settings| {
        (
            settings.is_gtk_recent_files_enabled(),
            settings.gtk_recent_files_max_age(),
        )
    })
}

/// The folders of the desktop's recently used list, newest visit first,
/// within the desktop's privacy settings.
pub(crate) fn recent_folder_uris() -> Vec<String> {
    let (enabled, max_age) = recent_history_policy();
    if !enabled {
        return Vec::new();
    }
    let mut folders: Vec<gtk::RecentInfo> = gtk::RecentManager::default()
        .items()
        .into_iter()
        .filter(|info| info.mime_type() == FOLDER_CONTENT_TYPE)
        .filter(|info| max_age < 0 || info.age() <= max_age)
        .collect();
    folders.sort_by_key(|info| std::cmp::Reverse(info.visited().to_unix()));
    folders
        .iter()
        .map(|info| info.uri().to_string())
        .take(MAX_RECENT_LOCATIONS)
        .collect()
}

/// The entry of the folder at `uri`, if it can be read now.
async fn folder_entry(uri: &str) -> Option<Entry> {
    let file = gio::File::for_uri(uri);
    let info = file
        .query_info_future(
            entry::ATTRIBUTES,
            gio::FileQueryInfoFlags::NONE,
            glib::Priority::DEFAULT,
        )
        .await
        .ok()?;
    let entry = entry::entry_from_info(&file, &info);
    entry.is_dir.then_some(entry)
}

/// Lists the recent folders that still exist, in one batch.
pub(crate) async fn list_recent_locations(on_batch: impl Fn(Vec<Entry>)) -> Result<(), EntryError> {
    let mut entries = Vec::new();
    for uri in recent_folder_uris() {
        // A folder that was removed, or a share that is not connected now,
        // is left out rather than listed broken.
        entries.extend(folder_entry(&uri).await);
    }
    on_batch(entries);
    Ok(())
}

/// Removes every folder from the desktop's recently used list; its files
/// stay. Tests clear only a private data folder (native/tools/check.py).
pub(crate) fn clear_recent_locations() {
    #[cfg(test)]
    if !gtk::glib::user_data_dir().starts_with(std::env::temp_dir()) {
        return;
    }
    let manager = gtk::RecentManager::default();
    for info in manager.items() {
        if info.mime_type() == FOLDER_CONTENT_TYPE {
            // An item another program removed meanwhile is already gone.
            let _ = manager.remove_item(&info.uri());
        }
    }
}
