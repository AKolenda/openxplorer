// SPDX-License-Identifier: AGPL-3.0-only
//! The window's side of the network service: its sign-in prompts and
//! their dialogs.
//!
//! Ports the `MountPrompts` wiring of `desktop/winspace.py`
//! (`self.prompts`, `authReply` and `close`). The window creates its
//! [`WindowNetwork`] when it is built and closes it when it goes, which
//! aborts every open sign-in and wipes the window's credentials
//! (SAFE-011, TAB-050).

use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;
use ox_core::network::{SignInError, WriteActivity};

use crate::network::WindowNetwork;

use super::BrowserWindow;

impl BrowserWindow {
    /// Creates the window's network state; its sign-in dialogs answer
    /// through the window's prompts and its notices use the toast.
    pub(super) fn start_network(&self) {
        let network = WindowNetwork::new(self, self.context().network());
        network.sign_in().answer_with(glib::clone!(
            #[weak(rename_to = window)]
            self,
            #[upgrade_or]
            Err(SignInError::Expired),
            move |id, answer| window.network().prompts().answer(id, answer)
        ));
        network.sign_in().show_notices_with(glib::clone!(
            #[weak(rename_to = window)]
            self,
            move |message| window.show_message(message)
        ));
        // Every successful mount of an SMB server through this window,
        // whatever read or connection asked for it, resumes indexing that
        // server after a sign-out (`mount` in winspace.py, NET-022,
        // SRCH-040).
        let context = self.context().downgrade();
        network.prompts().connect_server_mounted(move |host| {
            if let Some(context) = context.upgrade() {
                context.search_cache().resume_server(host);
            }
        });
        self.imp()
            .network
            .set(network)
            .expect("BrowserWindow::new starts the network once");
    }

    /// The window's prompts, sign-in dialogs and discovery.
    pub(super) fn network(&self) -> &WindowNetwork {
        self.imp()
            .network
            .get()
            .expect("BrowserWindow::new starts the network")
    }

    /// The window goes: every open sign-in is cancelled, its mount
    /// aborted, and the window's credentials wiped from memory.
    pub(super) fn close_network(&self) {
        if let Some(network) = self.imp().network.get() {
            network.close();
        }
    }

    /// Whether this window writes files, which Disconnect and Sign out
    /// wait for, so a mount never goes away under a write (OPS-024).
    pub(super) fn write_activity(&self) -> WriteActivity {
        if self.is_writing_files() {
            WriteActivity::Writing
        } else {
            WriteActivity::Idle
        }
    }
}
