// SPDX-License-Identifier: AGPL-3.0-only
//! The folder a tab shows was deleted, renamed or unmounted by something
//! else. As in Windows 11 File Explorer, the tab stays where it is, with
//! the old address, and nothing is said (NAV-039): it does not move to the
//! folder above, as Dolphin and the app before 2026-11-09 did. When the
//! user then refreshes it (F5, the Refresh button, Try again), "Location
//! is not available" says the folder is unavailable, in Windows' words
//! with the folder's own path, and the tab, still at the old address,
//! shows the "This location is unavailable" page behind it.
//!
//! Windows' box also has an error sign beside its text; the app shows the
//! red Fluent dismiss circle there. A refresh while a box is already open
//! opens no second one.

use gtk::glib;
use gtk::subclass::prelude::*;
use ox_core::i18n::{format_message, gettext};

use super::LoadRun;
use crate::dialog::Dialog;
use crate::window::listing_state::ListingEnd;
use crate::window::{BrowserWindow, ButtonStyle};

/// The caption of the box, Windows' own.
const TITLE: &str = crate::i18n::message_id("Location is not available");

/// What the box says about `path`, in Windows 11's words. Windows names
/// a drive letter path; the app names the folder's own path.
fn unavailable_message(path: &str) -> String {
    format_message(
        "{path} is unavailable. If the location is on this PC, make sure the device or drive is \
         connected or the disc is inserted, and then try again. If the location is on a network, \
         make sure you're connected to the network or Internet, and then try again. If the \
         location still can't be found, it might have been moved or deleted.",
        &[("path", path)],
    )
}

impl BrowserWindow {
    /// A reload of `run`'s folder that nobody asked for (the folder watch,
    /// or a background tab listed again) found it gone: the tab keeps its
    /// address and its rows, and stops watching a folder that is no longer
    /// there. F5 lists it again.
    pub(super) fn keep_removed_folder(&self, run: &LoadRun) {
        let id = run.tab;
        if let Some(tab) = self.imp().session.borrow_mut().tab_mut(id) {
            tab.watch = None;
            tab.folder_gone = true;
        }
        if self.imp().session.borrow_mut().end_listing(id) == ListingEnd::TabClosed {
            return;
        }
        self.redraw_pane(id);
        if self.imp().session.borrow().is_active(id) {
            self.update_status();
        }
    }

    /// Whether the active tab shows a search in a folder found gone: F5
    /// then lists the folder instead of running the search again, so it
    /// says the folder is unavailable.
    pub(in crate::window) fn searches_in_gone_folder(&self) -> bool {
        let session = self.imp().session.borrow();
        self.is_searching() && session.active().is_some_and(|tab| tab.folder_gone)
    }

    /// Ends the search of the active tab, whose folder is gone, keeping
    /// the tab at the folder's address, as leaving the search does.
    pub(in crate::window) fn end_search_in_gone_folder(&self) {
        if let Some(tab) = self.imp().session.borrow_mut().active_mut() {
            tab.history.set_current_search(None);
        }
        self.end_search();
        self.render_location();
    }

    /// The user refreshed `run`'s folder and it is not there: says so in
    /// "Location is not available", unless the tab is no longer in front
    /// or a dialog is already open.
    pub(super) fn say_folder_is_unavailable(&self, run: &LoadRun) {
        if !self.imp().session.borrow().is_active(run.tab) || self.has_open_dialog() {
            return;
        }
        let path = self.imp().locations.borrow().display_location(&run.uri);
        let dialog = Dialog::new(
            self,
            ox_core::i18n::gettext_static(TITLE),
            &unavailable_message(&path),
        );
        dialog.set_error_sign();
        dialog.add_button(&gettext("OK"), ButtonStyle::Accent);
        dialog.open();
        glib::spawn_future_local(async move {
            dialog.next_response().await;
            dialog.finish();
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The words are Windows 11's, with the folder's path for its drive
    /// letter path.
    ///
    /// parity: NAV-039
    #[test]
    fn an_unavailable_folder_is_said_in_windows_words() {
        assert_eq!(
            unavailable_message("/home/sample/Projects/Sub"),
            "/home/sample/Projects/Sub is unavailable. If the location is on this PC, make sure \
             the device or drive is connected or the disc is inserted, and then try again. If the \
             location is on a network, make sure you're connected to the network or Internet, and \
             then try again. If the location still can't be found, it might have been moved or \
             deleted."
        );
    }
}
