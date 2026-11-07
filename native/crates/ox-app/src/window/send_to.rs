// SPDX-License-Identifier: AGPL-3.0-only
//! Send to, the classic menu's submenu of Windows Explorer (CMD-038).
//!
//! Its entries ([`super::context_menu`] lists them) run here, each through
//! a path the app already has: a copy to Documents, a removable drive or a
//! saved network location goes through the transfer engine with the
//! name-conflict dialog, progress and Undo, as a paste into that folder
//! does, and the sent items stay selected as in Windows; Compressed (zipped) folder is Compress to ZIP file; Mail
//! recipient is Share, the desktop's Email portal; Desktop (create
//! shortcut) makes Windows-named shortcuts with ox-core's
//! [`ox_core::ops::create_shortcuts`], undone by moving them to the Trash; Bluetooth
//! device hands the files to the desktop's Bluetooth sender, which asks
//! for the device. Only what the computer has is offered
//! ([`BrowserWindow::send_to_targets`]).

use std::path::{Path, PathBuf};

use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;
use ox_core::integration::{BluetoothSender, ExecutableSearch, Sandbox};
use ox_core::location::same_location;
use ox_core::network::local_path;
use ox_core::ops::LinkRequest;
use ox_core::places::KnownFolder;
use ox_core::transfer::TransferMode;

use super::actions::{plain_action, text_action};
use super::context_menu::{SendToDrive, SendToTargets};
use super::file_ops::{IncomingItems, MAX_JOBS};
use super::window_action::WindowAction;
use super::BrowserWindow;
use crate::volumes::{VolumeKind, VolumeState};

/// Why Bluetooth device sent nothing: the sender needs files on this
/// computer or a mounted share.
const NOT_SENDABLE: &str =
    crate::i18n::message_id("Only files on this computer or on a mounted share can be sent by Bluetooth.");

/// Why a copy or a shortcut waits: the window's operations leave no room.
const WAIT_FOR_OPERATION: &str = crate::i18n::message_id("Wait for the running file operation to finish.");

/// Why a copy waits while every transfer slot is taken.
const TRANSFER_SLOTS_FULL: &str =
    crate::i18n::message_id("Four file operations are already running. Wait for one to finish.");

/// Why Bluetooth device could not start the sender.
const SENDER_FAILED: &str = crate::i18n::message_id("The Bluetooth sender could not be started.");

/// The address Send to sends for the row at `uri`: a Recent row stands
/// for its file, `recent_target`, which Desktop (create shortcut) and
/// Bluetooth device need a path for.
fn send_source(uri: &str, recent_target: Option<&str>) -> String {
    match recent_target {
        Some(target) if uri.starts_with("recent:") => target.to_owned(),
        _ => uri.to_owned(),
    }
}

/// Why a copy, and why a shortcut, cannot start now: `(copy, shortcut)`,
/// `None` where one can. `is_busy` is an exclusive operation running,
/// `jobs` the transfers running, a paste still checking or asking about
/// name conflicts among them, `panel_busy` another writer holding the
/// transfer panel.
fn capacity_refusals(
    is_busy: bool,
    jobs: usize,
    panel_busy: bool,
) -> (Option<&'static str>, Option<&'static str>) {
    let wait = Some(WAIT_FOR_OPERATION);
    // As `begin_transfer` admits a transfer.
    let copy = if is_busy || (jobs == 0 && panel_busy) {
        wait
    } else if jobs >= MAX_JOBS {
        Some(TRANSFER_SLOTS_FULL)
    } else {
        None
    };
    // As `begin_operation` admits the window's one exclusive operation.
    let shortcut = (is_busy || jobs > 0 || panel_busy).then_some(WAIT_FOR_OPERATION);
    (copy, shortcut)
}

/// The desktop's Bluetooth sender, where the computer has an adapter.
fn bluetooth_sender() -> Option<(BluetoothSender, PathBuf)> {
    BluetoothSender::find(&ExecutableSearch::for_sandbox(Sandbox::detect()), Path::new("/"))
}

impl BrowserWindow {
    /// Adds the actions of Send to's own entries.
    pub(super) fn install_send_to_actions(&self) {
        self.add_action_entries([
            text_action(WindowAction::SendTo, |window, folder| {
                let (window, folder) = (window.clone(), folder.to_owned());
                glib::spawn_future_local(async move { window.send_to_folder(folder).await });
            }),
            plain_action(WindowAction::SendToDesktop, |window| {
                let window = window.clone();
                glib::spawn_future_local(async move { window.send_to_desktop().await });
            }),
            plain_action(WindowAction::SendToBluetooth, BrowserWindow::send_to_bluetooth),
        ]);
        self.update_send_to_actions();
    }

    /// Enables Send to and its entries while something is selected.
    pub(super) fn update_send_to_actions(&self) {
        let selected = self.folder_pane().model().summary().count > 0;
        for action in [
            WindowAction::SendTo,
            WindowAction::SendToDesktop,
            WindowAction::SendToBluetooth,
        ] {
            self.set_action_enabled(action, selected);
        }
    }

    /// Where Send to can send the selection now: the Bluetooth sender, the
    /// Desktop and Documents folders and the Email portal where the
    /// desktop has them, then the removable drives and the saved network
    /// locations, named as This PC names them. Fixed disks are not
    /// offered, as Windows offers none.
    pub(super) fn send_to_targets(&self) -> SendToTargets {
        let places = self.places();
        let known_folders = self.context().known_folders();
        let known = |folder: KnownFolder| {
            known_folders
                .iter()
                .find(|place| place.known_folder == Some(folder))
                .map(|place| place.uri.clone())
        };
        let removable = places.drives.iter().filter_map(|row| match &row.state {
            VolumeState::Mounted { uri, controls }
                if row.kind == VolumeKind::Drive && (controls.can_eject || controls.can_stop) =>
            {
                Some(SendToDrive {
                    label: row.label.clone(),
                    uri: uri.clone(),
                    is_network: false,
                })
            }
            _ => None,
        });
        let network = places.saved_shares.iter().map(|share| SendToDrive {
            label: share.bookmark.label.clone(),
            uri: share.bookmark.uri.clone(),
            is_network: true,
        });
        let items = self.folder_pane().model().selected_items();
        let (copy_refusal, shortcut_refusal) = {
            let operations = self.imp().file_operations.borrow();
            capacity_refusals(
                operations.is_busy(),
                operations.job_count(),
                self.transfer_panel().is_busy(),
            )
        };
        SendToTargets {
            bluetooth: bluetooth_sender().is_some(),
            desktop: known(KnownFolder::Desktop).is_some(),
            documents: known(KnownFolder::Documents),
            mail: self.share_portal().is_some(),
            drives: removable.chain(network).collect(),
            has_folder: items.iter().any(|item| item.entry().is_dir),
            copy_refusal,
            shortcut_refusal,
        }
    }

    /// The addresses Send to sends for the selected items: a Recent row's
    /// file instead of the row ([`send_source`]).
    fn selected_uris(&self) -> Vec<String> {
        self.selected_rows_and_sources().1
    }

    /// The selected rows' own addresses, to select them again, and the
    /// addresses Send to sends for them ([`send_source`]).
    fn selected_rows_and_sources(&self) -> (Vec<String>, Vec<String>) {
        let items = self.folder_pane().model().selected_items();
        items
            .iter()
            .map(|item| {
                let entry = item.entry();
                let source = send_source(&entry.uri, entry.meta.recent_target.as_deref());
                (entry.uri.clone(), source)
            })
            .unzip()
    }

    /// Whether `folder` is a saved network location that is not connected
    /// now, as This PC shows it.
    fn is_unconnected_share(&self, folder: &str) -> bool {
        self.places()
            .saved_shares
            .iter()
            .any(|share| !share.is_connected && same_location(&share.bookmark.uri, folder))
    }

    /// Send to ▸ Documents, a drive or a network location: copies the
    /// selection into `folder` as a paste there does, conflicts, progress
    /// and Undo included. A saved network location that is not connected
    /// is connected first, as Windows reconnects a mapped drive, through
    /// the window's usual mount and its sign-in dialog; the copy starts
    /// only once that succeeds, so no write is ever replayed after a mount
    /// (NET-004).
    async fn send_to_folder(&self, folder: String) {
        let (rows, uris) = self.selected_rows_and_sources();
        if uris.is_empty() {
            return;
        }
        if self.is_unconnected_share(&folder) {
            if let Err(error) = self.network().mount(&folder).await {
                if !error.is_cancelled() {
                    self.show_message(&error.to_string());
                }
                return;
            }
        }
        if self.command_facts().selection.has_inoperable {
            self.show_message(ox_core::i18n::gettext_static(
                "Open the share first, then select its files or folders.",
            ));
            return;
        }
        if !self.imp().locations.borrow().is_writable_location(&folder) {
            self.show_message(ox_core::i18n::gettext_static("This folder is read-only."));
            return;
        }
        let incoming = IncomingItems {
            mode: TransferMode::Copy,
            uris,
            destination_folder: folder,
        };
        // A Recent row sends its file, but the row stays selected.
        self.send_copies(incoming, rows).await;
    }

    /// Send to ▸ Desktop (create shortcut): a shortcut on the Desktop to
    /// each selected item, named as Windows names it, as the window's one
    /// operation; Undo moves the shortcuts to the Trash.
    async fn send_to_desktop(&self) {
        let desktop = self
            .context()
            .known_folders()
            .into_iter()
            .find(|place| place.known_folder == Some(KnownFolder::Desktop));
        let (Some(desktop), (rows, uris)) = (desktop, self.selected_rows_and_sources()) else {
            return;
        };
        if uris.is_empty() {
            return;
        }
        let request = LinkRequest {
            uris,
            destination_folder: desktop.uri,
        };
        // A Recent row sends its file, but the row stays selected.
        self.run_shortcuts(&request, rows).await;
    }

    /// Send to ▸ Bluetooth device: starts the desktop's Bluetooth sender
    /// with the selected files, which asks for the device and sends them.
    fn send_to_bluetooth(&self) {
        let items = self.folder_pane().model().selected_items();
        if items.is_empty() || items.iter().any(|item| item.entry().is_dir) {
            return;
        }
        let Some((sender, program)) = bluetooth_sender() else {
            return;
        };
        let uris = self.selected_uris();
        let window = self.downgrade();
        glib::spawn_future_local(async move {
            // A share's mount path may take a moment to find.
            let started = gtk::gio::spawn_blocking(move || {
                let files = uris
                    .iter()
                    .map(|uri| local_path(uri))
                    .collect::<Option<Vec<PathBuf>>>()
                    .ok_or(NOT_SENDABLE)?;
                sender
                    .launch(&program, &files, Sandbox::detect())
                    .map_err(|_| SENDER_FAILED)
            })
            .await;
            let refusal = match started {
                Ok(Ok(())) => return,
                Ok(Err(reason)) => reason,
                Err(_) => SENDER_FAILED,
            };
            if let Some(window) = window.upgrade() {
                window.show_message(ox_core::i18n::gettext_static(refusal));
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A Recent row sends the file it stands for; any other row, and a
    /// Recent row that names no file, send themselves.
    ///
    /// parity: CMD-038
    #[test]
    fn a_recent_row_sends_the_file_it_stands_for() {
        let recent = "recent:///3f1c9a";
        assert_eq!(
            send_source(recent, Some("file:///home/demo/plan.txt")),
            "file:///home/demo/plan.txt"
        );
        assert_eq!(send_source(recent, None), recent);
        assert_eq!(
            send_source("file:///home/demo/plan.txt", None),
            "file:///home/demo/plan.txt"
        );
    }

    /// A copy waits for an exclusive operation, a full set of transfer
    /// slots, or another writer holding the panel; a shortcut, the window's
    /// one exclusive operation, waits for any write. Each says why.
    ///
    /// parity: CMD-038, OPS-024
    #[test]
    fn copies_and_shortcuts_wait_for_room_and_say_why() {
        let wait = Some(WAIT_FOR_OPERATION);
        let full = Some(TRANSFER_SLOTS_FULL);
        assert_eq!(capacity_refusals(false, 0, false), (None, None));
        assert_eq!(
            capacity_refusals(false, 2, false),
            (None, wait),
            "a copy runs beside others"
        );
        assert_eq!(capacity_refusals(false, MAX_JOBS, false), (full, wait));
        assert_eq!(capacity_refusals(true, 0, false), (wait, wait));
        assert_eq!(capacity_refusals(false, 0, true), (wait, wait));
    }
}
