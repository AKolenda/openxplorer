// SPDX-License-Identifier: AGPL-3.0-only
//! The folder a tab shows was deleted, renamed or unmounted by something
//! else: the tab moves to the nearest folder above it that still exists
//! and says so, as Dolphin does (NAV-039). app.js stayed on the "This
//! location is unavailable" page, which a folder with no existing
//! ancestor, and a first listing of a missing folder, still show.

use gtk::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{gio, glib};
use ox_core::entry::EntryError;
use ox_core::location::parent_location;

use super::{LoadMode, LoadRun};
use crate::window::listing_state::ListingEnd;
use crate::window::session::TabId;
use crate::window::BrowserWindow;

/// The nearest folder above `uri` that exists, `None` when none does.
async fn nearest_existing_folder(uri: &str) -> Option<String> {
    let mut candidate = parent_location(uri);
    while let Some(folder) = candidate {
        let info = gio::File::for_uri(&folder)
            .query_info_future(
                gio::FILE_ATTRIBUTE_STANDARD_TYPE,
                gio::FileQueryInfoFlags::NONE,
                glib::Priority::DEFAULT,
            )
            .await;
        if info.is_ok_and(|info| info.file_type() == gio::FileType::Directory) {
            return Some(folder);
        }
        candidate = parent_location(&folder);
    }
    None
}

impl BrowserWindow {
    /// A reload of `run`'s folder found it gone (`error`): moves the tab
    /// to the nearest existing folder above it, or shows `error` when
    /// there is none.
    pub(super) fn leave_removed_folder(&self, run: &LoadRun, error: EntryError) {
        let id = run.tab;
        if self.imp().session.borrow_mut().end_listing(id) == ListingEnd::TabClosed {
            return;
        }
        let removed = run.uri.clone();
        glib::spawn_future_local(glib::clone!(
            #[weak(rename_to = window)]
            self,
            async move {
                match nearest_existing_folder(&removed).await {
                    Some(folder) => window.move_tab_out_of_removed_folder(id, &removed, &folder),
                    None => window.show_load_error(id, error),
                }
            }
        ));
    }

    /// Moves tab `id` from the `removed` folder to `folder`, with the
    /// warning in the message line when the tab is in front.
    fn move_tab_out_of_removed_folder(&self, id: TabId, removed: &str, folder: &str) {
        let still_there = self
            .imp()
            .session
            .borrow()
            .tab(id)
            .is_some_and(|tab| tab.uri() == removed);
        if !still_there {
            return;
        }
        if self.imp().session.borrow().is_active(id) {
            self.navigate_or_report(folder);
            let path = self.imp().locations.borrow().display_location(removed);
            self.show_message(&format!(
                "Current location changed, {path} is no longer accessible."
            ));
            return;
        }
        if let Some(tab) = self.imp().session.borrow_mut().tab_mut(id) {
            tab.history.push(folder);
            tab.forget_location_state();
        }
        self.render_tabs();
        self.load_tab(id, LoadMode::Navigate);
    }

    /// Shows why tab `id`'s folder could not be listed again.
    fn show_load_error(&self, id: TabId, error: EntryError) {
        self.fail_load(id, LoadMode::Reload, error);
        self.redraw_pane(id);
    }
}
