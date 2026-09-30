// SPDX-License-Identifier: AGPL-3.0-only
//! What every window shares for network locations.
//!
//! Ports the application-wide network state of `desktop/winspace.py`: the
//! keyring and sign-out generations behind `SessionCredentials` (class
//! attributes there), the servers being signed out
//! (`signing_out_hosts`), the servers and shares browsed this session
//! (`visited_network`) and the kernel SMB mounts `environment` reads
//! (`stable`). [`crate::app_context::AppContext`] owns one
//! [`NetworkServices`]; each window builds its
//! [`WindowNetwork`](super::WindowNetwork) over it.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use gtk::{gio, glib};
use ox_core::network::{
    read_stable_smb_mounts, CredentialStore, RecentServers, SecretService, SignOutRegistry, VisitedNetwork,
};
use ox_core::places::StableMount;
use ox_core::settings::Bookmark;
use ox_core::LOG_DOMAIN;

use super::discovery::Discoverer;

/// The network state every window of the application shares.
#[derive(Debug)]
pub(crate) struct NetworkServices {
    /// The SMB credentials in the keyring, and their sign-out generations.
    credentials: Arc<CredentialStore>,
    /// The servers being signed out, which no window may list or mount.
    signing_out: Rc<SignOutRegistry>,
    /// The SMB servers and shares browsed this session.
    visited: RefCell<VisitedNetwork>,
    /// The kernel's CIFS and SMB3 mounts, as last read.
    stable_mounts: RefCell<Vec<StableMount>>,
    /// Where Discover servers looks.
    discoverer: RefCell<Discoverer>,
}

impl Default for NetworkServices {
    fn default() -> Self {
        Self {
            credentials: Arc::new(credential_store()),
            signing_out: Rc::default(),
            visited: RefCell::default(),
            stable_mounts: RefCell::default(),
            discoverer: RefCell::new(Discoverer::default()),
        }
    }
}

/// The keyring SMB credentials are saved in: the desktop's Secret
/// Service. Tests keep credentials in memory, so they never read or write
/// the keyring of the session they run in.
fn credential_store() -> CredentialStore {
    if cfg!(test) {
        CredentialStore::memory_only()
    } else {
        CredentialStore::new(Arc::new(SecretService))
    }
}

/// The recent-servers lists GTK's Other Locations shares (NET-019).
/// Tests get none, so they never read or write the GTK list of the
/// session they run in.
pub(crate) fn user_recent_servers() -> Option<RecentServers> {
    if cfg!(test) {
        None
    } else {
        Some(RecentServers::for_user())
    }
}

impl NetworkServices {
    /// The keyring store each window's credentials read and save through.
    pub(crate) fn credential_store(&self) -> Arc<CredentialStore> {
        Arc::clone(&self.credentials)
    }

    /// The servers being signed out, shared by every window.
    pub(crate) fn sign_out_registry(&self) -> Rc<SignOutRegistry> {
        Rc::clone(&self.signing_out)
    }

    /// Records the share or server of `uri`, a location that was listed;
    /// true when the Network list gains a row.
    pub(crate) fn remember_visited(&self, uri: &str) -> bool {
        self.visited.borrow_mut().remember(uri)
    }

    /// Forgets every browsed root on `host`, when signing out of it.
    pub(crate) fn forget_visited_host(&self, host: &str) {
        self.visited.borrow_mut().forget_host(host);
    }

    /// The browsed roots, oldest first, as the Network list merges them.
    pub(crate) fn visited_bookmarks(&self) -> Vec<Bookmark> {
        self.visited.borrow().to_bookmarks()
    }

    /// The kernel SMB mounts as last read.
    pub(crate) fn stable_mounts(&self) -> Vec<StableMount> {
        self.stable_mounts.borrow().clone()
    }

    /// Keeps `mounts` as the kernel SMB mounts; true when they changed.
    pub(crate) fn replace_stable_mounts(&self, mounts: Vec<StableMount>) -> bool {
        let changed = *self.stable_mounts.borrow() != mounts;
        self.stable_mounts.replace(mounts);
        changed
    }

    /// Where Discover servers looks.
    pub(crate) fn discoverer(&self) -> Discoverer {
        self.discoverer.borrow().clone()
    }

    /// Makes Discover servers find what `discoverer` returns, for tests.
    #[cfg(test)]
    pub(crate) fn set_discoverer(&self, discoverer: Discoverer) {
        self.discoverer.replace(discoverer);
    }
}

/// Reads the kernel SMB mounts off the main thread. An unreadable mount
/// table lists none, as an empty one would; the reason goes to the log.
pub(crate) async fn read_stable_mounts() -> Vec<StableMount> {
    let read = gio::spawn_blocking(read_stable_smb_mounts).await;
    match read {
        Ok(Ok(mounts)) => mounts,
        Ok(Err(error)) => {
            glib::g_warning!(LOG_DOMAIN, "Could not read the mount table: {error}");
            Vec::new()
        }
        // A panic in the reader is a bug; report it where it happened.
        Err(panic) => std::panic::resume_unwind(panic),
    }
}
