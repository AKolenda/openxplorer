// SPDX-License-Identifier: AGPL-3.0-only
//! The search commands of the window's menus: caching the current folder
//! for search, and opening a search result's folder.
//!
//! Ports `cacheMenuItems`, `setCache` and the "Open file location" item of
//! `entryMenu` in `desktop/ui/app.js` (SRCH-015, SRCH-020). "Cache this
//! folder for search" is a check item: checked while the folder is an
//! indexed folder, and unchecking it stops caching it, as "Stop caching
//! this folder" did. The search strip's "Cache this folder" runs the same
//! action. Neither is offered for pages, phones, cameras or a server's
//! share list, which cannot be indexed.

use gtk::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{gio, glib};
use ox_core::location::{is_device_location, is_smb_server, same_location};
use ox_core::search::Caching;

use crate::locations::Page;

use super::actions::plain_action;
use super::window_action::WindowAction;
use super::BrowserWindow;

/// What the window says after caching was switched on (`setCache`).
const CACHING_STARTED: &str =
    "Caching filenames and paths in the background. No file contents are downloaded.";
/// What the window says after caching was switched off.
const CACHING_STOPPED: &str = "Cache disabled; this root’s indexed names were removed.";

impl BrowserWindow {
    /// Adds "Cache this folder for search" and "Open file location".
    pub(super) fn install_search_actions(&self) {
        let cache_folder = gio::ActionEntry::builder(WindowAction::CacheFolder.name())
            .state(false.to_variant())
            .activate(|window: &BrowserWindow, action, _| {
                let is_cached = action.state().and_then(|state| state.get::<bool>());
                window.set_folder_caching(!is_cached.unwrap_or(false));
            })
            .build();
        let open_location = plain_action(
            WindowAction::OpenFileLocation,
            BrowserWindow::open_result_location,
        );
        self.add_action_entries([cache_folder, open_location]);
        self.set_action_enabled(WindowAction::OpenFileLocation, false);
    }

    /// Checks "Cache this folder for search" while the folder is an
    /// indexed folder, and offers it only where a folder can be indexed.
    pub(super) fn update_cache_folder_action(&self) {
        let folder = self.current_uri().filter(|uri| can_be_indexed(uri));
        let roots = self.context().search_cache().roots();
        let is_cached = folder.as_deref().is_some_and(|folder| {
            roots
                .iter()
                .any(|root| root.is_enabled() && same_location(&root.uri, folder))
        });
        self.set_action_enabled(WindowAction::CacheFolder, folder.is_some());
        self.set_action_state(WindowAction::CacheFolder, &is_cached.to_variant());
    }

    /// Offers "Open file location" for one selected search result.
    pub(super) fn update_open_location_action(&self, selected: u32) {
        let offers = self.is_searching() && selected == 1;
        self.set_action_enabled(WindowAction::OpenFileLocation, offers);
    }

    /// Starts or stops caching the current folder, and says so.
    fn set_folder_caching(&self, is_wanted: bool) {
        let Some(folder) = self.current_uri().filter(|uri| can_be_indexed(uri)) else {
            return;
        };
        let caching = if is_wanted {
            Caching::Enabled
        } else {
            Caching::Disabled
        };
        let label = self.imp().locations.borrow().title_for(&folder);
        let cache = self.context().search_cache().clone();
        let window = self.downgrade();
        glib::spawn_future_local(async move {
            let outcome = cache.set_caching(&folder, caching, &label).await;
            let Some(window) = window.upgrade() else {
                return;
            };
            let message = match (outcome, caching) {
                (Ok(()), Caching::Enabled) => CACHING_STARTED.to_owned(),
                (Ok(()), Caching::Disabled) => CACHING_STOPPED.to_owned(),
                (Err(error), _) => error.to_string(),
            };
            window.show_message(&message);
        });
    }
}

/// Whether `uri` is a folder the search cache can take: not a page, a
/// phone or camera, or a server's share list (`cacheMenuItems`).
fn can_be_indexed(uri: &str) -> bool {
    Page::from_uri(uri).is_none() && !is_device_location(uri) && !is_smb_server(uri)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// parity: SRCH-021
    #[test]
    fn pages_devices_and_servers_are_not_offered_for_caching() {
        assert!(can_be_indexed("file:///home/demo/Work"));
        assert!(can_be_indexed("smb://nas/share"));
        assert!(!can_be_indexed("smb://nas/"));
        assert!(!can_be_indexed("mtp://%5Busb%3A001%2C010%5D/"));
        assert!(!can_be_indexed(Page::Settings.uri()));
    }
}
