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

/// The folders of the desktop's recently used list, newest visit first,
/// within the desktop's privacy settings.
pub(crate) fn recent_folder_uris() -> Vec<String> {
    let policy = crate::app_context::desktop_recent_policy();
    if !policy.remember {
        return Vec::new();
    }
    let mut folders: Vec<gtk::RecentInfo> = gtk::RecentManager::default()
        .items()
        .into_iter()
        .filter(|info| info.mime_type() == FOLDER_CONTENT_TYPE)
        .filter(|info| {
            policy
                .max_age_days
                .is_none_or(|days| u64::try_from(info.age()).is_ok_and(|age| age <= days))
        })
        .collect();
    folders.sort_by_key(|info| std::cmp::Reverse(info.visited().to_unix()));
    newest_of_each(folders.iter().map(|info| info.uri().to_string()))
        .take(MAX_RECENT_LOCATIONS)
        .collect()
}

/// `uris`, newest first, each folder once: two spellings of one SMB folder
/// that differ only in case (`smb://nas/Private` and `smb://nas/private`,
/// as a share typed and the share mounted spell it) are one folder, as SMB
/// names ignore case, and its newest visit stands for it.
fn newest_of_each(uris: impl Iterator<Item = String>) -> impl Iterator<Item = String> {
    let mut seen = Vec::new();
    uris.filter(move |uri| {
        let key = match ox_core::places::network_key(uri) {
            Ok(key) if ox_core::location::is_smb_location(uri) => format!("{key:?}"),
            _ => uri.clone(),
        };
        let is_new = !seen.contains(&key);
        seen.push(key);
        is_new
    })
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
        if !crate::app_context::desktop_recent_policy().remember {
            entries.clear();
            break;
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Recent locations lists a share once however its case was spelled,
    /// at its newest visit; local folders keep their case.
    ///
    /// parity: SIDE-026
    #[test]
    fn a_share_spelled_in_two_cases_is_listed_once() {
        let uris = [
            "smb://harbornas.local/private",
            "file:///home/ana/Plans",
            "smb://harbornas.local/Private",
            "file:///home/ana/plans",
        ];

        let listed: Vec<String> = newest_of_each(uris.iter().map(|uri| (*uri).to_owned())).collect();

        assert_eq!(
            listed,
            [
                "smb://harbornas.local/private",
                "file:///home/ana/Plans",
                "file:///home/ana/plans"
            ]
        );
    }
}
