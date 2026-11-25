// SPDX-License-Identifier: AGPL-3.0-only
//! The drive or share a tab's folder is on went away: a USB stick pulled
//! out, a disk unmounted from a terminal, a share disconnected by another
//! program or by the server. Its old rows would stay on screen, looking
//! current, since nothing lists the folder again; instead every such tab
//! drops them and shows the "This location is unavailable" page, saying
//! the drive or share was disconnected, with Try again.
//!
//! Two things report it: the volume monitor's `mount-removed` signal, for
//! every tab inside the mount, and a folder watch's unmount event, for a
//! mount the volume monitor does not show. Try again lists the folder as
//! F5 does, so a share that is back is mounted again, asking for its
//! sign-in if needed, and a drive that is plugged in again is listed.
//!
//! Disconnect, Eject and Safely remove in the window move the tabs off
//! the drive first (DEV-009), and Sign out marks its server's tabs to be
//! listed again, so those tabs are not touched here: only tabs that are
//! listed or listing, without an error, are.

use gtk::gio;
use gtk::subclass::prelude::*;
use ox_core::entry::EntryError;

use crate::window::listing_state::ListingState;
use crate::window::mounting::is_inside;
use crate::window::session::TabId;
use crate::window::BrowserWindow;

/// What the "This location is unavailable" page says about a folder whose
/// drive or share went away.
pub(in crate::window) fn disconnected_error() -> EntryError {
    EntryError::NotMounted(disconnected_message())
}

/// The words of [`disconnected_error`], already said for people.
pub(in crate::window) fn disconnected_message() -> String {
    ox_core::i18n::gettext("The drive or network share that holds this folder was disconnected.")
}

impl BrowserWindow {
    /// The mount at `root` went away: every tab showing a folder inside
    /// it says so.
    ///
    /// A drive on this computer, such as a USB stick pulled out, sends its
    /// tabs to the Desktop at once, with no message, as Windows 11's
    /// window on a stick does (checked in the VM on 2026-11-22); a share
    /// says it was disconnected, with Try again. A drive the app unmounts
    /// on purpose to mount it again, as Rename does, keeps its tabs for
    /// that.
    pub(in crate::window) fn mount_removed(&self, root: &str) {
        if self.is_local_drive(root) && !crate::window::mounting::is_unmounted_on_purpose(root) {
            let desktop = self.desktop_location();
            self.move_tabs_from(root, &desktop);
        } else {
            self.show_disconnected(|_, uri| is_inside(uri, root));
        }
    }

    /// The watch of tab `id` saw its folder's drive or share unmounted.
    pub(in crate::window) fn folder_unmounted(&self, id: TabId) {
        let folder = {
            let session = self.imp().session.borrow();
            session.tab(id).map(|tab| tab.uri().to_owned())
        };
        match folder {
            Some(folder) if self.is_local_drive(&folder) => {
                let desktop = self.desktop_location();
                self.move_tabs_from(&folder, &desktop);
            }
            _ => self.show_disconnected(|tab, _| tab == id),
        }
    }

    /// Whether `uri` is on a drive of this computer rather than a share.
    fn is_local_drive(&self, uri: &str) -> bool {
        uri.starts_with("file:") && !self.imp().locations.borrow().is_network_location(uri)
    }

    /// The Desktop folder, where the tabs on a drive that was pulled out
    /// go; the home folder where there is no Desktop folder.
    pub(in crate::window) fn desktop_location(&self) -> String {
        let desktop = self
            .context()
            .known_folders()
            .into_iter()
            .find(|place| place.known_folder == Some(ox_core::places::KnownFolder::Desktop))
            .map(|place| place.uri)
            .filter(|uri| {
                gtk::prelude::FileExt::path(&gtk::gio::File::for_uri(uri)).is_some_and(|path| path.is_dir())
            });
        desktop.unwrap_or_else(|| self.imp().locations.borrow().home_uri())
    }

    /// Drops the rows of every listed tab that `is_gone` picks, stops
    /// reading its folder and shows that its drive or share was
    /// disconnected. The items are dropped after the session is released,
    /// because dropping the active tab's items runs the view's handlers,
    /// which read the session.
    fn show_disconnected(&self, is_gone: impl Fn(TabId, &str) -> bool) {
        let mut gone: Vec<(TabId, gio::ListStore)> = Vec::new();
        self.imp().session.borrow_mut().change_panes(|tab| {
            let is_shown = tab.error.is_none() && !tab.listing_state.needs_listing();
            if is_shown && is_gone(tab.id, tab.uri()) {
                tab.stop_reading();
                tab.listing_state = ListingState::Listed;
                tab.changed_while_hidden = false;
                tab.reloading = false;
                tab.error = Some(disconnected_error());
                gone.push((tab.id, tab.store.clone()));
            }
        });
        if gone.is_empty() {
            return;
        }
        self.change_model(|| {
            for (_, items) in &gone {
                items.remove_all();
            }
        });
        for (id, _) in &gone {
            self.redraw_pane(*id);
        }
        self.update_status();
    }
}
