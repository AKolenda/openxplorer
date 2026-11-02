// SPDX-License-Identifier: AGPL-3.0-only
//! Locations that answer only when a test lets them: a slow server
//! without a server. While a location is held, looking it up, listing it
//! and mounting it wait, on the main loop, as a slow share would keep
//! GIO's reply. Dropping the [`Hold`] lets them answer.

use std::cell::RefCell;
use std::collections::HashMap;
use std::time::Duration;

use gtk::glib;

/// How often a held lookup, listing or mount checks whether it may go on.
const POLL: Duration = Duration::from_millis(5);

thread_local! {
    /// Each held location, with how many times something waited on it.
    static HELD: RefCell<HashMap<String, u32>> = RefCell::default();
    /// How many times something waited on each location, held or not
    /// any more.
    static WAITS: RefCell<HashMap<String, u32>> = RefCell::default();
}

/// Holds `uri` until dropped.
#[derive(Debug)]
#[must_use = "the location answers as soon as the hold is dropped"]
pub(crate) struct Hold(String);

impl Drop for Hold {
    fn drop(&mut self) {
        HELD.with(|held| held.borrow_mut().remove(&self.0));
    }
}

/// Holds every lookup, listing and mount of `uri` until the returned
/// [`Hold`] is dropped.
pub(crate) fn hold(uri: &str) -> Hold {
    HELD.with(|held| held.borrow_mut().insert(uri.to_owned(), 0));
    WAITS.with(|waits| waits.borrow_mut().remove(uri));
    Hold(uri.to_owned())
}

/// Holds every mount of the share that holds `uri`, and nothing else,
/// until the returned [`Hold`] is dropped: the share's listing finds it
/// unmounted at once, and the mount waits, as a slow server or a sign-in
/// keeps it.
pub(crate) fn hold_mount(uri: &str) -> Hold {
    hold(&mount_key(uri))
}

/// What a mount of `uri` waits on.
pub(crate) fn mount_key(uri: &str) -> String {
    format!("mount {uri}")
}

/// How many lookups, listings and mounts of `uri` have waited on its hold
/// since it was held.
pub(crate) fn waits(uri: &str) -> u32 {
    WAITS.with(|waits| waits.borrow().get(uri).copied().unwrap_or_default())
}

fn is_held(uri: &str) -> bool {
    HELD.with(|held| held.borrow().contains_key(uri))
}

/// Waits while `uri` is held; returns at once when it is not.
pub(crate) async fn wait_while_held(uri: &str) {
    if !is_held(uri) {
        return;
    }
    WAITS.with(|waits| *waits.borrow_mut().entry(uri.to_owned()).or_default() += 1);
    while is_held(uri) {
        glib::timeout_future(POLL).await;
    }
}
