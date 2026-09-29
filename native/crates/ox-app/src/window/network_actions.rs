// SPDX-License-Identifier: AGPL-3.0-only
//! The window's network and device commands: Map network location, Keep
//! in Network, Remove saved location, Open in new window, Discover
//! servers, Sign out, Disconnect, Eject and Safely remove.
//!
//! Ports `connectDialog`, `removeBookmark`, the Keep in Network item of
//! `networkLocationMenu` and `discoverNetwork` in `desktop/ui/app.js`, and
//! `connect` and `after_connect` in `desktop/winspace.py`. Sign out is in
//! [`super::network_sign_out`], the device commands in
//! [`super::mounting`].

use gtk::glib;
use gtk::prelude::*;
use ox_core::network::{connect_share, ConnectedShare};
use ox_core::settings::{BookmarkAction, BookmarkKind, BookmarkRequest, SettingsError};

use crate::devices::Removal;
use crate::dialogs::{map_network_dialog, MapRequest, NetworkFormDialog, ShareKeeping};
use crate::places::network_row;
use crate::settings_store::Change;

use super::actions::{plain_action, text_action};
use super::window_action::WindowAction;
use super::BrowserWindow;

/// Connect's label while Map network location connects.
const CONNECTING: &str = "Connecting…";

/// A settings change that adds or removes the share `uri` under Network.
fn share_change(action: BookmarkAction, uri: String, label: String) -> Change {
    Box::new(move |settings| {
        let request = BookmarkRequest::new(uri, label);
        settings.bookmark(action, BookmarkKind::Share, &request)
    })
}

impl BrowserWindow {
    /// Adds the network and device actions.
    pub(super) fn install_network_actions(&self) {
        self.add_action_entries([
            plain_action(
                WindowAction::MapNetworkLocation,
                BrowserWindow::open_map_network_dialog,
            ),
            plain_action(WindowAction::DiscoverServers, BrowserWindow::discover_servers),
            plain_action(WindowAction::StopDiscovery, BrowserWindow::stop_discovery),
            text_action(WindowAction::KeepInNetwork, BrowserWindow::keep_in_network),
            text_action(
                WindowAction::RemoveSavedLocation,
                BrowserWindow::remove_saved_location,
            ),
            text_action(WindowAction::OpenWindow, BrowserWindow::open_in_new_window),
            text_action(WindowAction::SignOut, BrowserWindow::sign_out_of_server),
            text_action(WindowAction::Disconnect, |window, uri| {
                window.remove_drive(uri, Removal::Disconnect);
            }),
            text_action(WindowAction::Eject, |window, uri| {
                window.remove_drive(uri, Removal::Eject);
            }),
            text_action(WindowAction::SafelyRemove, |window, uri| {
                window.remove_drive(uri, Removal::SafelyRemove);
            }),
        ]);
    }

    /// Map network location: asks for a share, then connects it.
    fn open_map_network_dialog(&self) {
        let dialog = map_network_dialog(
            self,
            glib::clone!(
                #[weak(rename_to = window)]
                self,
                move |dialog, request| window.connect_mapped_share(dialog, request)
            ),
        );
        dialog.present();
    }

    /// Mounts the share `request` names and checks it is a folder, with
    /// "Connecting…" meanwhile; an error stays in `dialog`. Closing the
    /// dialog cancels the connection and ignores a late success.
    fn connect_mapped_share(&self, dialog: &NetworkFormDialog, request: MapRequest) {
        let prompts = self.network().prompts().clone();
        let signing_out = self.context().network().sign_out_registry();
        let window = self.downgrade();
        let form = dialog.downgrade();
        dialog.run(CONNECTING, async move {
            let connected = connect_share(&prompts, &signing_out, &request.address, &request.label).await;
            let (Some(window), Some(dialog)) = (window.upgrade(), form.upgrade()) else {
                return;
            };
            match connected {
                Ok(share) => window.keep_mapped_share(&dialog, share, request.keeping),
                Err(error) => dialog.show_error(&error.to_string()),
            }
        });
    }

    /// The share is connected: resumes indexing its server, lists it under
    /// Network, saves it when the user asked, then opens it
    /// (`after_connect`). A save that fails keeps the dialog open with the
    /// reason.
    pub(super) fn keep_mapped_share(
        &self,
        dialog: &NetworkFormDialog,
        share: ConnectedShare,
        keeping: ShareKeeping,
    ) {
        self.context().signed_in_to(&share.uri);
        self.context().remember_network(&share.uri);
        if keeping == ShareKeeping::ThisSessionOnly {
            dialog.finish();
            self.navigate_or_report(&share.uri);
            return;
        }
        let uri = share.uri.clone();
        let change = share_change(BookmarkAction::Add, share.uri, share.label);
        self.context().change_settings(
            change,
            glib::clone!(
                #[weak(rename_to = window)]
                self,
                #[weak]
                dialog,
                move |saved: Result<(), SettingsError>| match saved {
                    Ok(()) => {
                        dialog.finish();
                        window.navigate_or_report(&uri);
                    }
                    Err(error) => dialog.show_error(&error.to_string()),
                }
            ),
        );
    }

    /// Keep in Network: saves the browsed share `uri` under its label,
    /// without credentials (NET-017).
    fn keep_in_network(&self, uri: &str) {
        let network = self.network_locations();
        let label = network_row(&network, uri).map(|row| row.label.clone());
        let change = share_change(BookmarkAction::Add, uri.to_owned(), label.unwrap_or_default());
        self.change_network_settings(change, None);
    }

    /// Remove saved location: deletes the saved entry only; the share stays
    /// mounted and its credentials stay saved (NET-017).
    fn remove_saved_location(&self, uri: &str) {
        let change = share_change(BookmarkAction::Remove, uri.to_owned(), String::new());
        self.change_network_settings(change, Some("Saved location removed."));
    }

    /// Saves `change`, then shows `done`, or why it failed.
    fn change_network_settings(&self, change: Change, done: Option<&'static str>) {
        self.context().change_settings(
            change,
            glib::clone!(
                #[weak(rename_to = window)]
                self,
                move |result: Result<(), SettingsError>| match (result, done) {
                    (Ok(()), Some(message)) => window.show_message(message),
                    (Ok(()), None) => {}
                    (Err(error), _) => window.show_message(&error.to_string()),
                }
            ),
        );
    }

    /// Open in new window: another window of the app showing `uri`.
    fn open_in_new_window(&self, uri: &str) {
        let Some(app) = self.application() else {
            return;
        };
        let window = BrowserWindow::new(&app, self.context());
        if let Err(error) = window.add_tab(uri) {
            self.show_message(&error.to_string());
            window.destroy();
            return;
        }
        window.present();
    }

    /// Discover servers: looks for advertised SMB servers, redrawing the
    /// Network page as they are found.
    pub(super) fn discover_servers(&self) {
        let discoverer = self.context().network().discoverer();
        self.network().discovery().start(
            discoverer,
            glib::clone!(
                #[weak(rename_to = window)]
                self,
                move || window.render_landing()
            ),
        );
        self.render_landing();
    }

    /// Starts discovery the first time the window shows the Network page
    /// (`load` in app.js).
    pub(super) fn discover_servers_once(&self) {
        if !self.network().discovery().state().has_started {
            self.discover_servers();
        }
    }

    /// Stop: ends discovery and ignores what it would still find.
    fn stop_discovery(&self) {
        self.network().discovery().stop();
        self.render_landing();
    }
}
