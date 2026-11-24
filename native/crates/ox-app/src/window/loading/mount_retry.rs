// SPDX-License-Identifier: AGPL-3.0-only
//! Mounting a share that a listing found unmounted, then listing it once
//! more.
//!
//! Ports `retry_list` and the `mount_retry` branch of `start_worker` in
//! `v2.0.0:desktop/winspace.py` (NET-004): the share is mounted once, asking for
//! credentials through the window's sign-in dialog if needed, and listed
//! again from empty rows. A second "not mounted" is reported, never
//! mounted again, and only listings come here: a write is never replayed
//! after a mount or a sign-in.
//!
//! An SFTP server opened at its root (`sftp://host/`, as Network's
//! service rows open it) is shown at the account's home folder once it is
//! mounted, where `GVfs` says the mount starts (B6), rather than at the
//! server's `/`.

use gtk::prelude::*;
use gtk::subclass::prelude::*;
use ox_core::entry::EntryError;
use ox_core::network::NetworkError;

use super::{LoadMode, LoadRun, LoadStart};
use crate::folder_view::loader;
use crate::window::session::TabId;
use crate::window::BrowserWindow;

/// Whether a listing that finds its share unmounted may mount it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum MountRetry {
    /// Not yet: mount it and list again.
    Allowed,
    /// The share was mounted for this listing; a second "not mounted" is
    /// an error, never another mount (`_mounted_once` in winspace.py).
    Used,
}

/// Why a mount failed, as the listing reports it.
fn listing_error(error: NetworkError) -> EntryError {
    if error.is_cancelled() {
        return EntryError::Cancelled;
    }
    match error {
        NetworkError::Gio(error) => EntryError::from(error),
        NetworkError::Location(error) => EntryError::Location(error),
        error => EntryError::Failed(error.to_string()),
    }
}

impl BrowserWindow {
    /// Mounts the share of `run`, which was not mounted, then lists it
    /// once more. The tab stays busy meanwhile; moving it elsewhere or
    /// closing it cancels the mount and any sign-in it asked for.
    pub(super) fn mount_and_list_again(&self, run: &LoadRun) {
        let mounting = self.network().mount(&run.uri);
        let window = self.downgrade();
        let tab = run.tab;
        let start = LoadStart {
            uri: run.uri.clone(),
            generation: run.generation,
            was_shown: run.was_shown,
        };
        let mode = run.mode;
        let listing = loader::Listing::spawn(async move {
            let mounted = mounting.await;
            let mut start = start;
            if mounted.is_ok() {
                if let Some(home) = remote_home(&start.uri).await {
                    start.uri = home;
                }
            }
            if let Some(window) = window.upgrade() {
                window.list_after_mount(tab, &start, mode, mounted);
            }
        });
        if let Some(tab) = self.imp().session.borrow_mut().tab_mut(tab) {
            tab.listing = Some(listing);
        }
    }

    /// Lists `start` again from empty rows after mounting it, or reports
    /// why the mount failed. A write is never replayed this way: only
    /// listings reach here (NET-004).
    fn list_after_mount(
        &self,
        id: TabId,
        start: &LoadStart,
        mode: LoadMode,
        mounted: Result<(), NetworkError>,
    ) {
        if !self.imp().session.borrow().accepts(id, start.generation) {
            return;
        }
        if let Err(error) = mounted {
            self.refuse_listing(id, mode, listing_error(error));
            return;
        }
        // An SFTP server opens at the account's home: the tab is there now.
        {
            let mut session = self.imp().session.borrow_mut();
            if let Some(tab) = session.tab_mut(id).filter(|tab| tab.uri() != start.uri) {
                tab.history.replace_current(&start.uri);
            }
        }
        if self.imp().session.borrow().is_active(id) {
            self.render_navigation();
        }
        // `retry_list` resets the rows a partial listing left.
        self.clear_rows(id);
        let listing = self.start_listing(id, start, LoadMode::Navigate, MountRetry::Used);
        if let Some(tab) = self.imp().session.borrow_mut().tab_mut(id) {
            tab.listing = Some(listing);
        }
    }
}

/// `uri`, or for an SFTP server's root, the account's home once it is
/// mounted ([`remote_home`]): where opening it from a row shows it.
pub(in crate::window) async fn at_remote_home(uri: String) -> String {
    remote_home(&uri).await.unwrap_or(uri)
}

/// How long finding where an SFTP mount starts may take.
const HOME_LOOKUP: std::time::Duration = std::time::Duration::from_secs(5);

/// Where an SFTP server opened at its root `uri` is shown once mounted:
/// the folder `GVfs` says its mount starts at, the account's home, when
/// that is not the root itself; `None` for another location, or when the
/// mount cannot say within five seconds. GIO is asked on a worker thread,
/// so the window never waits for it.
async fn remote_home(uri: &str) -> Option<String> {
    let is_root = |uri: &str| {
        ox_core::location::split_location(uri)
            .is_ok_and(|parts| parts.scheme == "sftp" && parts.path.trim_matches('/').is_empty())
    };
    if !is_root(uri) {
        return None;
    }
    let server = uri.to_owned();
    let lookup = gtk::gio::spawn_blocking(move || {
        use gtk::gio::prelude::{FileExt, MountExt};
        let mount = gtk::gio::File::for_uri(&server)
            .find_enclosing_mount(gtk::gio::Cancellable::NONE)
            .ok()?;
        Some(mount.default_location().uri().to_string())
    });
    let home = gtk::glib::future_with_timeout(HOME_LOOKUP, lookup)
        .await
        .ok()?
        .ok()??;
    // The home's path on the server as it was asked for, without the
    // account the mount's own address names (`sftp://ana@host/`).
    let path = ox_core::location::split_location(&home).ok()?.path;
    let home = ox_core::location::normalise(&format!("{}{path}", uri.trim_end_matches('/'))).ok()?;
    (!is_root(&home)).then_some(home)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A cancelled mount ends the listing quietly; GIO's refusals keep
    /// their kind, and the service's own refusals their words.
    #[test]
    fn a_failed_mount_is_reported_as_the_listing_reports_errors() {
        let not_found = gtk::glib::Error::new(gtk::gio::IOErrorEnum::NotFound, "No such share");

        assert_eq!(listing_error(NetworkError::Cancelled), EntryError::Cancelled);
        assert_eq!(
            listing_error(NetworkError::Gio(not_found)),
            EntryError::NotFound("No such share".into())
        );
        assert_eq!(
            listing_error(NetworkError::ServerSigningOut),
            EntryError::Failed("This server is being signed out. Reopen it after sign-out finishes.".into())
        );
    }
}
