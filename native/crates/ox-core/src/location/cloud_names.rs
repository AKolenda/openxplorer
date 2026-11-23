// SPDX-License-Identifier: AGPL-3.0-only
//! The names of Google Drive folders, whose addresses hold IDs (DEV-013).
//!
//! `GVfs`'s Google Drive backend names every file and folder in an
//! address by its Drive ID (`google-drive://alice@example.com/0AIb…/1x9…`)
//! and reports the name people see as `standard::display-name`. Titles and
//! breadcrumbs are made from addresses, so the names of the folders a
//! listing showed are remembered here, for every window: a name belongs to
//! the drive, not to a tab. `OneDrive` addresses hold the names themselves.

use std::collections::HashMap;
use std::sync::{LazyLock, Mutex, PoisonError};

use super::text::strip_one_trailing_slash;

/// The scheme whose addresses name items by ID.
const ID_SCHEME_PREFIX: &str = "google-drive://";

/// How many names are kept before the oldest listings are forgotten; a
/// name forgotten is shown as its ID until its folder is listed again.
const MAX_NAMES: usize = 100_000;

/// Display names by address, without a trailing slash.
static NAMES: LazyLock<Mutex<HashMap<String, String>>> = LazyLock::new(Mutex::default);

/// Whether the items of `uri` are named by ID in their addresses.
pub fn names_items_by_id(uri: &str) -> bool {
    uri.get(..ID_SCHEME_PREFIX.len())
        .is_some_and(|scheme| scheme.eq_ignore_ascii_case(ID_SCHEME_PREFIX))
}

/// Remembers that the item at `uri` is called `name`, when its address
/// names it by ID ([`names_items_by_id`]); other addresses are ignored.
pub fn remember_cloud_name(uri: &str, name: &str) {
    if !names_items_by_id(uri) || name.is_empty() {
        return;
    }
    let mut names = NAMES.lock().unwrap_or_else(PoisonError::into_inner);
    if names.len() >= MAX_NAMES {
        names.clear();
    }
    names.insert(strip_one_trailing_slash(uri).to_owned(), name.to_owned());
}

/// The folders from `uri` up to its drive's root, the root left out,
/// whose names no listing gave yet ([`remember_cloud_name`]): those a
/// title or a breadcrumb would show as IDs. Empty for an address that
/// does not name items by ID.
pub fn unnamed_cloud_folders(uri: &str) -> Vec<String> {
    let mut unnamed = Vec::new();
    let mut folder = Some(strip_one_trailing_slash(uri).to_owned());
    while let Some(current) = folder.filter(|current| names_items_by_id(current)) {
        let parent = super::parent_location(&current);
        if parent.is_none() {
            break;
        }
        if cloud_name(&current).is_none() {
            unnamed.push(current);
        }
        folder = parent;
    }
    unnamed
}

/// The name a listing gave the item at `uri`, if any.
pub(super) fn cloud_name(uri: &str) -> Option<String> {
    if !names_items_by_id(uri) {
        return None;
    }
    let names = NAMES.lock().unwrap_or_else(PoisonError::into_inner);
    names.get(strip_one_trailing_slash(uri)).cloned()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Only addresses that name items by ID keep a name.
    ///
    /// parity: DEV-013
    #[test]
    fn only_google_drive_names_are_remembered() {
        remember_cloud_name("google-drive://a@example.com/0AIbRoot/", "My Drive");
        remember_cloud_name("onedrive://a@example.com/Documents", "Elsewhere");
        assert_eq!(
            cloud_name("google-drive://a@example.com/0AIbRoot").as_deref(),
            Some("My Drive")
        );
        assert_eq!(cloud_name("onedrive://a@example.com/Documents"), None);
        assert_eq!(cloud_name("google-drive://a@example.com/unlisted"), None);
    }

    /// A folder opened by its address, not from a listing, has its
    /// unnamed folders up to the drive's root looked up.
    ///
    /// parity: DEV-013
    #[test]
    fn unnamed_folders_run_up_to_the_drive_root() {
        remember_cloud_name("google-drive://b@example.com/0AIbHome", "My Drive");
        assert_eq!(
            unnamed_cloud_folders("google-drive://b@example.com/0AIbHome/1Year/1Month/"),
            [
                "google-drive://b@example.com/0AIbHome/1Year/1Month",
                "google-drive://b@example.com/0AIbHome/1Year",
            ]
        );
        assert!(unnamed_cloud_folders("google-drive://b@example.com/").is_empty());
        assert!(unnamed_cloud_folders("onedrive://b@example.com/Documents/Q3").is_empty());
    }
}
