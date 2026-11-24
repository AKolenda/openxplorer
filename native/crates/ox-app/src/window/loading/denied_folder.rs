// SPDX-License-Identifier: AGPL-3.0-only
//! A folder the user went to that may not be read: as Windows 11 does, the
//! tab stays where it was and "Location is not available" says
//! "<path> is not accessible. Access is denied." in a message box with the
//! error icon and one OK, as Windows' says it, with nothing under it.
//! app.js showed the "This location is unavailable"
//! page with the raw error, which a tab with nowhere to stay (a new tab or
//! window, Back or Forward to the folder, a reload) and a network location
//! still show.

use gtk::glib;
use gtk::subclass::prelude::*;
use ox_core::entry::EntryError;
use ox_core::i18n::{format_message, gettext};

use super::{LoadMode, LoadRun};
use crate::dialog::Dialog;
use crate::locations::Page;
use crate::window::listing_state::ListingEnd;
use crate::window::session::TabId;
use crate::window::{BrowserWindow, ButtonStyle};

/// The caption of the question.
const TITLE: &str = crate::i18n::message_id("Location is not available");

/// "/home/sample/Private is not accessible.", a blank line, then "Access
/// is denied.", as Windows says it.
fn denied_message(path: &str) -> String {
    format_message(
        "{path} is not accessible.\n\nAccess is denied.",
        &[("path", path)],
    )
}

impl BrowserWindow {
    /// Whether `run`, a listing that was refused, may leave its folder:
    /// the tab in front went to a folder on this computer just now, so it
    /// has a folder to stay in, and no other dialog is up. Network, This
    /// PC, shares (`GVfs`'s FUSE folders too) and every other location keep
    /// the unavailable page.
    pub(super) fn may_leave_denied_folder(&self, run: &LoadRun) -> bool {
        let local = run.uri.starts_with("file://") && !run.uri.contains("/gvfs/");
        if !local || Page::from_uri(&run.uri).is_some() {
            return false;
        }
        self.may_step_back(run.tab, &run.uri, run.mode)
    }

    /// Whether tab `id`, which went to `uri` in `mode`, may go back to the
    /// folder it came from: the tab in front went there just now, so it has
    /// a folder to stay in, and no other dialog is up.
    pub(super) fn may_step_back(&self, id: TabId, uri: &str, mode: LoadMode) -> bool {
        if mode != LoadMode::Navigate || self.has_open_dialog() {
            return false;
        }
        let session = self.imp().session.borrow();
        if !session.is_active(id) {
            return false;
        }
        // The current step must still be the navigation itself: a search
        // started there meanwhile is a step of its own, and keeps the page.
        session.tab(id).is_some_and(|tab| {
            let history = &tab.history;
            tab.uri() == uri
                && history.current_search().is_none()
                && history.can_go_back()
                && !history.can_go_forward()
        })
    }

    /// Takes the tab back to the folder it came from, forgetting the
    /// navigation, and says why, as Windows does.
    pub(super) fn leave_denied_folder(&self, run: &LoadRun, error: &EntryError) {
        if !self.step_back(run.tab) {
            return;
        }
        let path = self.imp().locations.borrow().display_location(&run.uri);
        glib::g_debug!(ox_core::LOG_DOMAIN, "{path} is not accessible: {error}");
        glib::spawn_future_local(glib::clone!(
            #[weak(rename_to = window)]
            self,
            async move { window.say_location_is_not_available(&path).await }
        ));
    }

    /// Ends tab `id`'s listing and takes it back to the folder it came
    /// from, forgetting the navigation, as Windows does when the folder
    /// it went to cannot be shown; false when the tab closed or has no
    /// step to go back to.
    pub(super) fn step_back(&self, id: TabId) -> bool {
        if self.imp().session.borrow_mut().end_listing(id) == ListingEnd::TabClosed {
            return false;
        }
        {
            let mut session = self.imp().session.borrow_mut();
            let Some(tab) = session.tab_mut(id) else {
                return false;
            };
            if !tab.history.drop_last() {
                return false;
            }
            tab.forget_location_state();
            if let Some(returned) = tab.left_views.take(tab.history.position()) {
                tab.selected = returned.current.into_iter().collect();
                tab.scroll_after_listing = Some(returned.scroll);
                tab.expand_after_listing = returned.expanded;
            }
        }
        self.render_navigation();
        self.load_tab(id, LoadMode::Navigate);
        // A search the tab came from shows again, as Back shows it.
        self.show_step_search();
        true
    }

    /// "Location is not available" for `path`, in Windows' message box:
    /// the error icon, the message and OK.
    async fn say_location_is_not_available(&self, path: &str) {
        let dialog = Dialog::error_box(self, ox_core::i18n::gettext_static(TITLE), &denied_message(path));
        dialog.add_button(&gettext("OK"), ButtonStyle::Accent);
        dialog.open();
        dialog.next_response().await;
        dialog.finish();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The words are Windows'.
    ///
    /// parity: VIEW-047
    #[test]
    fn a_denied_folder_is_said_in_windows_words() {
        assert_eq!(
            denied_message("/home/sample/Private"),
            "/home/sample/Private is not accessible.\n\nAccess is denied."
        );
    }
}
