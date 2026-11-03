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
use ox_core::network::local_path;
use ox_core::ops::LinkRequest;
use ox_core::places::KnownFolder;
use ox_core::transfer::TransferMode;

use super::actions::{plain_action, text_action};
use super::context_menu::{SendToDrive, SendToTargets};
use super::file_ops::IncomingItems;
use super::window_action::WindowAction;
use super::BrowserWindow;
use crate::volumes::{VolumeKind, VolumeState};

/// Why Bluetooth device sent nothing: the sender needs files on this
/// computer or a mounted share.
const NOT_SENDABLE: &str =
    crate::i18n::message_id("Only files on this computer or on a mounted share can be sent by Bluetooth.");

/// Why Bluetooth device could not start the sender.
const SENDER_FAILED: &str = crate::i18n::message_id("The Bluetooth sender could not be started.");

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
                    is_connected: true,
                })
            }
            _ => None,
        });
        let network = places.saved_shares.iter().map(|share| SendToDrive {
            label: share.bookmark.label.clone(),
            uri: share.bookmark.uri.clone(),
            is_network: true,
            is_connected: share.is_connected,
        });
        let items = self.folder_pane().model().selected_items();
        SendToTargets {
            bluetooth: bluetooth_sender().is_some(),
            desktop: known(KnownFolder::Desktop).is_some(),
            documents: known(KnownFolder::Documents),
            mail: self.share_portal().is_some(),
            drives: removable.chain(network).collect(),
            has_folder: items.iter().any(|item| item.entry().is_dir),
        }
    }

    /// The URIs of the selected items.
    fn selected_uris(&self) -> Vec<String> {
        let items = self.folder_pane().model().selected_items();
        items.iter().map(|item| item.entry().uri.clone()).collect()
    }

    /// Send to ▸ Documents, a drive or a network location: copies the
    /// selection into `folder` as a paste there does, conflicts, progress
    /// and Undo included.
    async fn send_to_folder(&self, folder: String) {
        let uris = self.selected_uris();
        if uris.is_empty() {
            return;
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
        self.send_copies(incoming).await;
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
        let (Some(desktop), uris) = (desktop, self.selected_uris()) else {
            return;
        };
        if uris.is_empty() {
            return;
        }
        let request = LinkRequest {
            uris,
            destination_folder: desktop.uri,
        };
        self.run_shortcuts(&request).await;
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
