// SPDX-License-Identifier: AGPL-3.0-only
//! The network state every window shares: the keyring and sign-outs
//! ([`NetworkServices`]), the SMB servers and shares browsed this session
//! and the kernel's SMB mounts.
//!
//! Ports `visited_network`, `remember_network` and the stable-mount part
//! of `environment` in `v2.0.0:desktop/winspace.py`. A change tells every window
//! through `places-changed`; a finished sign-out through
//! `server-signed-out`.

use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;
use ox_core::settings::Bookmark;

use super::{AppContext, SERVER_SIGNED_OUT};
use crate::dialogs::SearchCacheChoice;
use crate::network::{self, NetworkServices};

impl AppContext {
    /// The network services every window shares.
    pub(crate) fn network(&self) -> &NetworkServices {
        &self.imp().network
    }

    /// The SMB servers and shares browsed this session, oldest first.
    pub(crate) fn visited_network(&self) -> Vec<Bookmark> {
        self.network().visited_bookmarks()
    }

    /// Records a listed SMB location under Network for this session, as
    /// `remember_network` in winspace.py: the server, or the share the
    /// location is on. Browsing never saves a bookmark (NET-016).
    pub(crate) fn remember_network(&self, uri: &str) {
        if self.network().remember_visited(uri) {
            self.notify_places_changed();
        }
    }

    /// The server `host` is being signed out: forgets the servers and
    /// shares browsed on it, and pauses its indexing until the next
    /// successful mount of it (`serverSigningOut` and `pause_server` in
    /// winspace.py, NET-022).
    pub(crate) fn forget_network_host(&self, host: &str) {
        self.network().forget_visited_host(host);
        self.search_cache().pause_server(host);
        self.notify_places_changed();
    }

    /// The server `host` was signed out: clears its cached file names when
    /// the user asked to ("Also clear cached filenames for this server",
    /// NET-022), and tells the windows.
    pub(crate) fn announce_server_signed_out(&self, host: &str, search_cache: SearchCacheChoice) {
        let clear_names = search_cache == SearchCacheChoice::Clear;
        if clear_names {
            self.search_cache().clear_server(host);
        }
        self.emit_by_name::<()>(SERVER_SIGNED_OUT, &[&host, &clear_names]);
    }

    /// Reads the kernel's SMB mounts off the main thread, and tells every
    /// window when they changed, as `environment` in winspace.py reads
    /// them on every change.
    pub(crate) fn refresh_stable_mounts(&self) {
        let context = self.downgrade();
        glib::spawn_future_local(async move {
            let mounts = network::read_stable_mounts().await;
            let Some(context) = context.upgrade() else {
                return;
            };
            if context.network().replace_stable_mounts(mounts) {
                context.notify_places_changed();
            }
        });
    }
}
