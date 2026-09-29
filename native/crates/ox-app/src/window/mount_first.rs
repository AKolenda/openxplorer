// SPDX-License-Identifier: AGPL-3.0-only
//! Reads other than listings mount an unmounted share first (NET-004).
//!
//! Listings mount their share and list again ([`super::loading`]); the
//! other reads of an item on a server, such as opening a file, Open with,
//! Properties, Open in Terminal and opening an archive, ask here first.
//! When `GVfs` says the share is not mounted (another app or a sign-out
//! unmounted it since it was listed), it is mounted once through the
//! window's sign-in dialog, as `mount` in `desktop/winspace.py` does
//! before retrying a read, and the read then runs. A failed or cancelled
//! mount is reported and the read never runs.
//!
//! Safety rule (NET-004): only reads come here, so a write is never
//! replayed after a mount or a sign-in.

use gtk::prelude::*;
use gtk::{gio, glib};
use ox_core::location::is_server_location;

use super::BrowserWindow;

impl BrowserWindow {
    /// Runs `read` now for a location on this computer or a device, and
    /// once the share holding `uri` is mounted for a server location.
    pub(super) fn after_mounting(&self, uri: &str, read: impl FnOnce(&BrowserWindow) + 'static) {
        if !is_server_location(uri) {
            read(self);
            return;
        }
        let file = gio::File::for_uri(uri);
        let uri = uri.to_owned();
        let window = self.downgrade();
        glib::spawn_future_local(async move {
            // The cheapest read tells whether the share is mounted.
            let probe = file
                .query_info_future(
                    "standard::type",
                    gio::FileQueryInfoFlags::NONE,
                    glib::Priority::DEFAULT,
                )
                .await;
            let needs_mount = matches!(&probe, Err(error) if error.matches(gio::IOErrorEnum::NotMounted));
            if needs_mount {
                let Some(mounting) = window.upgrade().map(|window| window.network().mount(&uri)) else {
                    return;
                };
                let mounted = mounting.await;
                let Some(window) = window.upgrade() else {
                    return;
                };
                if let Err(error) = mounted {
                    if !error.is_cancelled() {
                        window.show_message(&error.to_string());
                    }
                    return;
                }
                read(&window);
            } else if let Some(window) = window.upgrade() {
                read(&window);
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::rc::Rc;

    use ox_core::network::NetworkError;

    use crate::test_support::harness::{wait_until, Fixture, TestWindow};

    /// Ported from `desktop/tests/test_v05.py`'s mount-then-retry checks
    /// for reads other than listings. `example.invalid` never resolves, so
    /// `GVfs` reports it not mounted without reaching a network.
    ///
    /// parity: NET-004
    #[gtk::test]
    fn a_read_on_an_unmounted_share_mounts_it_once_first() {
        let fixture = Fixture::standard();
        let test = TestWindow::open(&fixture.uri());
        let mounts = Rc::new(Cell::new(0));
        let counted = Rc::clone(&mounts);
        test.window.network().answer_mounts_with(move || {
            counted.set(counted.get() + 1);
            Ok(())
        });
        let reads = Rc::new(Cell::new(0));

        let read = Rc::clone(&reads);
        test.window
            .after_mounting("smb://example.invalid/share/plan.odt", move |_| read.set(read.get() + 1));
        wait_until("the read after the mount", || reads.get() == 1);
        assert_eq!(mounts.get(), 1);

        let read = Rc::clone(&reads);
        test.window.after_mounting(&fixture.uri(), move |_| read.set(read.get() + 1));
        assert_eq!(reads.get(), 2, "a local read runs at once");
        assert_eq!(mounts.get(), 1, "and mounts nothing");

        test.window
            .network()
            .answer_mounts_with(|| Err(NetworkError::NotAFolder));
        let read = Rc::clone(&reads);
        test.window
            .after_mounting("smb://example.invalid/other/a.txt", move |_| read.set(read.get() + 1));
        wait_until("the failed mount", || {
            test.window.shown_message().as_str() == "This location is not a folder."
        });
        assert_eq!(reads.get(), 2, "a failed mount runs no read");
    }
}
