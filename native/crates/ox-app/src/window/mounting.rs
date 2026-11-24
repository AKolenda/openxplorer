// SPDX-License-Identifier: AGPL-3.0-only
//! Connecting a drive or device from the sidebar or This PC, and taking
//! one away: Disconnect, Eject and Safely remove.
//!
//! Ports `mountVolume` and `unmount` in `v2.0.0:desktop/ui/app.js` and the
//! `mountVolume` and `unmount` operations of `v2.0.0:desktop/winspace.py`, and
//! adds Dolphin's Eject and Safely remove ([`Removal`]). Each runs through
//! GTK's mount operation without blocking the window: the desktop's own
//! dialogs ask for an encrypted disk's password, show the programs that
//! keep a drive busy and say when an ejected medium's writes are flushed.
//!
//! Before a drive is taken away, every tab showing a folder on it moves
//! to Home, as Dolphin does, so the window's own listings and folder
//! monitors never keep the drive busy (DEV-009).

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gtk::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{gio, glib};
use ox_core::location::file_uri;
use ox_core::network::{mount_volume, NetworkError, WriteActivity};

use crate::devices::Removal;
use crate::dialog::DialogButton;
use crate::locations::Page;
use crate::window::{ButtonStyle, Dialog};

use super::session::{Direction, TabId};
use super::BrowserWindow;

/// The tabs a removal moved to Home before it ran, to put back where they
/// were when the drive could not be removed.
#[derive(Debug)]
pub(super) struct MovedTabs {
    /// The home folder they were moved to.
    home: String,
    /// The tabs behind the one in front that were on the drive.
    behind: Vec<TabId>,
    /// Whether the tab in front was on the drive.
    front: bool,
}

/// The note of "Disconnect this mount?", under the location.
const DISCONNECT_NOTE: &str = crate::i18n::message_id(
    "Close files using this mount first. This disconnects the session mount for other applications too.",
);

/// True when `uri` is the mount root `root` or a location inside it, as
/// GIO compares files.
pub(super) fn is_inside(uri: &str, root: &str) -> bool {
    let file = gio::File::for_uri(uri);
    let root = gio::File::for_uri(root);
    file.equal(&root) || file.has_prefix(&root)
}

thread_local! {
    /// The roots of the drives the app is unmounting on purpose, to mount
    /// them again ([`unmount_on_purpose`]).
    static UNMOUNTED_ON_PURPOSE: std::cell::RefCell<Vec<String>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// While the returned guard lives, the drive at `root` going away is the
/// app's own doing (Rename unmounts an exFAT or NTFS drive to relabel
/// it), so its tabs stay to follow it rather than go to the Desktop.
pub(in crate::window) fn unmount_on_purpose(root: &str) -> UnmountedOnPurpose {
    UNMOUNTED_ON_PURPOSE.with(|roots| roots.borrow_mut().push(root.to_owned()));
    UnmountedOnPurpose(root.to_owned())
}

/// Whether the drive at `root` is being unmounted on purpose.
pub(in crate::window) fn is_unmounted_on_purpose(root: &str) -> bool {
    UNMOUNTED_ON_PURPOSE.with(|roots| {
        roots
            .borrow()
            .iter()
            .any(|own| is_inside(root, own) || is_inside(own, root))
    })
}

/// See [`unmount_on_purpose`].
pub(in crate::window) struct UnmountedOnPurpose(String);

impl Drop for UnmountedOnPurpose {
    fn drop(&mut self) {
        UNMOUNTED_ON_PURPOSE.with(|roots| {
            let mut roots = roots.borrow_mut();
            if let Some(index) = roots.iter().position(|root| *root == self.0) {
                roots.remove(index);
            }
        });
    }
}

impl BrowserWindow {
    /// Mounts the volume `id` (asking for a password through the
    /// desktop's dialog if needed), then opens it; a failure shows "Could
    /// not mount device" (`mountVolume` in app.js).
    pub(super) fn mount_volume(&self, id: &str) {
        let volumes = self.volume_monitor().volumes();
        let operation = gtk::MountOperation::new(Some(self));
        let id = id.to_owned();
        let window = self.downgrade();
        glib::spawn_future_local(async move {
            let mounted = mount_volume(&volumes, &id, Some(operation.upcast_ref())).await;
            let Some(window) = window.upgrade() else {
                return;
            };
            match mounted {
                Ok(root) => window.navigate_or_report(&root),
                Err(error) if error.is_cancelled() => {}
                Err(error) => window.show_failure(
                    ox_core::i18n::gettext_static("Could not mount device"),
                    &error.to_string(),
                ),
            }
        });
    }

    /// Mounts the volume `id` for a drop onto its sidebar row (DEV-010)
    /// and returns its root; a failure shows "Could not mount device".
    pub(super) async fn mount_for_drop(&self, id: &str) -> Option<String> {
        #[cfg(test)]
        if let Some((_, root)) = self
            .imp()
            .test_volume
            .borrow()
            .as_ref()
            .filter(|(test_id, _)| test_id == id)
        {
            return Some(root.clone());
        }
        let volumes = self.volume_monitor().volumes();
        let operation = gtk::MountOperation::new(Some(self));
        match mount_volume(&volumes, id, Some(operation.upcast_ref())).await {
            Ok(root) => Some(root),
            Err(error) if error.is_cancelled() => None,
            Err(error) => {
                self.show_failure(
                    ox_core::i18n::gettext_static("Could not mount device"),
                    &error.to_string(),
                );
                None
            }
        }
    }

    /// Takes away the drive or device that holds `uri` as `removal` says.
    /// Disconnect asks first, as This PC's card does; nothing starts while
    /// this window writes.
    pub(super) fn remove_drive(&self, uri: &str, removal: Removal) {
        if self.write_activity() == WriteActivity::Writing {
            self.show_message(&ox_core::i18n::gettext(
                "Finish the current operation before disconnecting.",
            ));
            return;
        }
        match removal {
            Removal::Disconnect => self.confirm_disconnect(uri),
            Removal::Eject | Removal::SafelyRemove => self.run_removal(uri, removal),
        }
    }

    /// "Disconnect this mount?" with the location, then Disconnect, which
    /// Enter presses, as Windows' Disconnect goes ahead.
    fn confirm_disconnect(&self, uri: &str) {
        let address = self.imp().locations.borrow().display_location(uri);
        let message = format!("{address}\n\n{DISCONNECT_NOTE}");
        let dialog = Dialog::new(self, &ox_core::i18n::gettext("Disconnect this mount?"), &message);
        dialog.add_cancel_button();
        dialog.add_button(&ox_core::i18n::gettext("Disconnect"), ButtonStyle::Accent);
        let uri = uri.to_owned();
        dialog.connect_confirmed(glib::clone!(
            #[weak(rename_to = window)]
            self,
            move |dialog| {
                dialog.finish();
                window.run_removal(&uri, Removal::Disconnect);
            }
        ));
        dialog.open_on_primary_button();
    }

    /// The mount operation of a removal of the drive `label`: GTK's, for
    /// passwords, but a drive a program still uses asks in Windows' words
    /// ([`Self::ask_about_busy_drive`]); Try Again sets `retry`.
    pub(super) fn removal_operation(&self, label: &str, retry: Rc<Cell<bool>>) -> gtk::MountOperation {
        let operation = gtk::MountOperation::new(Some(self));
        // The box that asks, while it shows: GIO asks again every few
        // seconds while the drive stays busy.
        let asking: Rc<RefCell<Option<Dialog>>> = Rc::default();
        let (window, label_text) = (self.downgrade(), label.to_owned());
        let shown = Rc::clone(&asking);
        operation.connect_local("show-processes", false, move |values| {
            let operation = values.first()?.get::<gio::MountOperation>().ok()?;
            operation.stop_signal_emission_by_name("show-processes");
            if shown.borrow().is_some() {
                return None;
            }
            let Some(window) = window.upgrade() else {
                operation.reply(gio::MountOperationResult::Aborted);
                return None;
            };
            let (dialog, again, carry_on) = window.busy_drive_box(&label_text);
            shown.replace(Some(dialog.clone()));
            let (shown, retry) = (Rc::clone(&shown), Rc::clone(&retry));
            glib::spawn_future_local(async move {
                let pressed = dialog.next_response().await;
                dialog.finish();
                // GIO went on by itself, the programs having let go.
                if shown.take().is_none() {
                    return;
                }
                if pressed == Some(carry_on) {
                    // GIO's first choice: "Unmount Anyway", "Eject Anyway".
                    operation.set_choice(0);
                    operation.reply(gio::MountOperationResult::Handled);
                } else {
                    retry.set(pressed == Some(again));
                    operation.reply(gio::MountOperationResult::Aborted);
                }
            });
            None
        });
        operation.connect_aborted(move |_| {
            if let Some(dialog) = asking.take() {
                dialog.finish();
            }
        });
        operation
    }

    /// Windows 11's box about the drive `label` a program still uses
    /// (checked in the VM on 2026-11-22): the warning icon, the drive in
    /// use, what to do, and that Continue closes the files, with Cancel,
    /// which has the keyboard, Try Again and Continue.
    fn busy_drive_box(&self, label: &str) -> (Dialog, DialogButton, DialogButton) {
        let text = [
            ox_core::i18n::format_message("'{drive}' is currently in use.", &[("drive", label)]),
            ox_core::i18n::gettext(
                "Save any open files on this disc, and then close the files or programs using the files \
                 before trying again.",
            ),
            ox_core::i18n::gettext(
                "If you choose to continue, the files will be closed, which might cause data to be lost.",
            ),
        ]
        .join("\n\n");
        let dialog = Dialog::message_box(self, ox_core::i18n::gettext_static("OpenXplorer"), &text);
        dialog.add_cancel_button();
        let again = dialog.add_button(ox_core::i18n::gettext_static("Try Again"), ButtonStyle::Bordered);
        let carry_on = dialog.add_button(ox_core::i18n::gettext_static("Continue"), ButtonStyle::Bordered);
        dialog.open();
        (dialog, again, carry_on)
    }

    /// Stops reading the tabs on the drive, then removes it.
    fn run_removal(&self, uri: &str, removal: Removal) {
        let label = self.drive_label(uri);
        let moved = self.move_tabs_home_from(uri);
        let mounts = self.volume_monitor().mounts();
        let retry = Rc::new(Cell::new(false));
        let operation = self.removal_operation(&label, Rc::clone(&retry));
        let activity = self.write_activity();
        let uri = uri.to_owned();
        let window = self.downgrade();
        glib::spawn_future_local(async move {
            let removed = removal
                .perform(&mounts, &uri, operation.upcast_ref(), activity)
                .await;
            if let Some(window) = window.upgrade() {
                window.finish_removal(&uri, &label, removal, removed, &moved);
                // Try Again in the box about a drive still in use.
                if retry.get() {
                    window.run_removal(&uri, removal);
                }
            }
        });
    }

    /// Shows what the removal did. After Disconnect the window shows
    /// Network, as app.js does; after Eject or Safely remove a tab that
    /// went back onto the drive meanwhile shows This PC. When the drive
    /// could not be removed, the tabs `moved` to Home go back to where
    /// they were, as Windows leaves its windows on a drive still in use.
    fn finish_removal(
        &self,
        uri: &str,
        label: &str,
        removal: Removal,
        removed: Result<(), NetworkError>,
        moved: &MovedTabs,
    ) {
        if let Err(error) = removed {
            self.return_moved_tabs(moved);
            if !Removal::is_reported_by_desktop(&error) {
                self.show_failure(removal.failure_title(), &Removal::failure_detail(&error, label));
            }
            return;
        }
        let was_inside = self.current_uri().is_some_and(|current| is_inside(&current, uri));
        self.mark_stale_inside(uri);
        match removal {
            Removal::Disconnect => {
                self.forget_mapped_drives_inside(uri);
                self.navigate_or_report(Page::Network.uri());
            }
            Removal::Eject | Removal::SafelyRemove if was_inside => {
                self.navigate_or_report(Page::ThisPc.uri());
            }
            Removal::Eject | Removal::SafelyRemove => {}
        }
        if let Some(message) = removal.done_message(label) {
            self.show_message(&message);
        }
    }

    /// Disconnect of a mapped drive ends its mapping, as in Windows: the
    /// shares mapped to a letter at or inside the disconnected `root`, for
    /// this session or saved, are forgotten, so no row of it is left in
    /// the navigation pane or This PC (B1). A saved network location
    /// without a letter stays, as Windows keeps network locations.
    fn forget_mapped_drives_inside(&self, root: &str) {
        let mapped: Vec<String> = self
            .network_locations()
            .into_iter()
            .filter(|location| location.is_saved)
            .filter(|location| crate::dialogs::drive_letter(&location.label).is_some())
            .filter(|location| ox_core::network::is_at_or_inside(&location.uri, root))
            .map(|location| location.uri)
            .collect();
        for uri in mapped {
            self.forget_saved_location(&uri);
        }
        self.context().forget_mapped_shares_inside(root);
    }

    /// The name of the drive that holds `uri`, for the message after it
    /// is removed.
    fn drive_label(&self, uri: &str) -> String {
        let volumes = self.imp().volumes.borrow();
        let holding = volumes
            .iter()
            .find(|row| row.uri().is_some_and(|root| is_inside(uri, root)));
        holding.map_or_else(|| uri.to_owned(), |row| row.label.clone())
    }

    /// Moves every tab inside `root` to Home: the tab in front navigates
    /// there, and a tab behind it stops reading the drive and lists Home
    /// when it is next shown. Returns the tabs it moved.
    fn move_tabs_home_from(&self, root: &str) -> MovedTabs {
        self.move_tabs_from(root, &file_uri(&glib::home_dir()))
    }

    /// Moves every tab inside `root` to `home`, as
    /// [`Self::move_tabs_home_from`] moves them to Home.
    pub(super) fn move_tabs_from(&self, root: &str, home: &str) -> MovedTabs {
        let home = home.to_owned();
        let mut behind = Vec::new();
        let (stale, is_active_inside): (Vec<gio::ListStore>, bool) = {
            let mut session = self.imp().session.borrow_mut();
            let active = session.active_id();
            let is_active_inside = session.active().is_some_and(|tab| is_inside(tab.uri(), root));
            let mut stale = Vec::new();
            session.change_panes(|tab| {
                if Some(tab.id) != active && is_inside(tab.uri(), root) && tab.history.push(&home) {
                    behind.push(tab.id);
                    tab.forget_location_state();
                    stale.push(tab.mark_stale());
                }
            });
            (stale, is_active_inside)
        };
        self.change_model(|| {
            for items in &stale {
                items.remove_all();
            }
        });
        if is_active_inside {
            self.navigate_or_report(&home);
        }
        self.render_tabs();
        MovedTabs {
            home,
            behind,
            front: is_active_inside,
        }
    }

    /// Puts the tabs `moved` to Home back on the folders they showed,
    /// unless they left Home meanwhile: the tab in front goes back, and a
    /// tab behind it lists its folder again when it is next shown.
    fn return_moved_tabs(&self, moved: &MovedTabs) {
        let stale: Vec<gio::ListStore> = {
            let mut session = self.imp().session.borrow_mut();
            let mut stale = Vec::new();
            session.change_panes(|tab| {
                if moved.behind.contains(&tab.id) && tab.uri() == moved.home && tab.history.drop_last() {
                    tab.forget_location_state();
                    stale.push(tab.mark_stale());
                }
            });
            stale
        };
        self.change_model(|| {
            for items in &stale {
                items.remove_all();
            }
        });
        let front_at_home = self.current_uri().is_some_and(|uri| uri == moved.home);
        if moved.front && front_at_home {
            self.go_history(Direction::Backward);
        }
        self.render_tabs();
    }

    /// Moves every tab inside `old_root` to the same place below
    /// `new_root`, where the same drive is mounted again after it was
    /// renamed: the tab in front navigates there, and a tab behind it
    /// lists it when it is next shown.
    pub(super) fn move_tabs_to_new_root(&self, old_root: &str, new_root: &str) {
        let (from, to) = (gio::File::for_uri(old_root), gio::File::for_uri(new_root));
        let moved = |uri: &str| -> Option<String> {
            let file = gio::File::for_uri(uri);
            let target = if file.equal(&from) {
                to.clone()
            } else {
                to.resolve_relative_path(from.relative_path(&file)?)
            };
            ox_core::location::normalise(&target.uri()).ok()
        };
        let (stale, active_to): (Vec<gio::ListStore>, Option<String>) = {
            let mut session = self.imp().session.borrow_mut();
            let active = session.active_id();
            let active_to = session.active().and_then(|tab| moved(tab.uri()));
            let mut stale = Vec::new();
            session.change_panes(|tab| {
                if Some(tab.id) == active {
                    return;
                }
                if let Some(target) = moved(tab.uri()) {
                    tab.history.push(&target);
                    tab.forget_location_state();
                    stale.push(tab.mark_stale());
                }
            });
            (stale, active_to)
        };
        self.change_model(|| {
            for items in &stale {
                items.remove_all();
            }
        });
        if let Some(target) = active_to {
            self.navigate_or_report(&target);
        }
        self.render_tabs();
    }

    /// Makes every tab inside `root` list again when it is next shown.
    fn mark_stale_inside(&self, root: &str) {
        self.mark_tabs_stale(|uri| is_inside(uri, root));
    }

    /// Makes every tab whose location passes `is_stale` list again when it
    /// is next shown, and drops what those tabs listed. The items are
    /// dropped after the session is released, because dropping the active
    /// tab's items runs the view's handlers, which read the session.
    pub(super) fn mark_tabs_stale(&self, is_stale: impl Fn(&str) -> bool) {
        let mut stale: Vec<gio::ListStore> = Vec::new();
        self.imp().session.borrow_mut().change_panes(|tab| {
            if is_stale(tab.uri()) {
                stale.push(tab.mark_stale());
            }
        });
        self.change_model(|| {
            for items in &stale {
                items.remove_all();
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_location_is_inside_its_mount_root_and_nowhere_else() {
        assert!(is_inside("file:///media/demo/USB", "file:///media/demo/USB"));
        assert!(is_inside(
            "file:///media/demo/USB/Photos",
            "file:///media/demo/USB"
        ));
        assert!(!is_inside("file:///media/demo/USB2", "file:///media/demo/USB"));
        assert!(!is_inside("file:///home/demo", "file:///media/demo/USB"));
    }
}
