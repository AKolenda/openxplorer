// SPDX-License-Identifier: AGPL-3.0-only
//! The computers under Network in the navigation pane, as Windows 11
//! shows them: each with its own chevron that expands it to its shares.
//!
//! The rows are composed by [`network_tree`](super::sidebar::network_tree);
//! this module keeps the window's part: which computers are expanded,
//! what each one's listing gave, and the work that reads it. A computer's
//! shares are read the first time it is expanded, off the main loop (the
//! listing is GIO's asynchronous one, as for a folder), and again on
//! Refresh; a server that needs mounting first is mounted, asking for a
//! password only when the server wants one. Collapsing a computer, or a
//! newer listing, drops the one that runs. Expanding Network looks for
//! computers once, as Windows' navigation pane does, when nothing found
//! them yet this session.

use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;
use ox_core::entry::{Entry, EntryError};
use ox_core::network::computer_key_of;

use super::sidebar::network_tree::{network_tree, ComputerNode, ListedShare, NetworkTreeSources};
use super::BrowserWindow;
use crate::folder_view::loader;
use crate::places::Places;

/// The window's state of the computers under Network.
#[derive(Debug, Default)]
pub(super) struct NetworkPane {
    /// The keys of the computers expanded.
    expanded: std::cell::RefCell<Vec<String>>,
    /// Each computer's listing, by key, once it arrived.
    listed: std::cell::RefCell<Vec<(String, Vec<ListedShare>)>>,
    /// The listings that run, by key; dropping one cancels it.
    listings: std::cell::RefCell<Vec<(String, loader::Listing)>>,
    /// The discovery the navigation pane started, while it runs.
    discovery: std::cell::RefCell<Option<loader::Listing>>,
    /// Whether the navigation pane looked for computers this session.
    discovered: std::cell::Cell<bool>,
}

/// The share or service a listed row is, as the navigation pane lists it
/// inside a computer; `None` for a row that is not one.
fn listed_share(entry: &Entry) -> Option<ListedShare> {
    if !entry.is_dir {
        return None;
    }
    Some(ListedShare {
        name: entry.name.clone(),
        uri: entry.navigation_uri().to_owned(),
        is_hidden: entry.is_hidden,
        is_service: entry.kind == ox_core::entry::EntryKind::Shortcut,
    })
}

impl BrowserWindow {
    fn network_pane(&self) -> &NetworkPane {
        &self.imp().network_pane
    }

    /// The computers under Network, for `places`.
    pub(super) fn network_computers(&self, places: &Places) -> Vec<ComputerNode> {
        let pane = self.network_pane();
        let computers = self.context().network().known_computers();
        let show_hidden = self.context().settings_data().preferences.show_hidden;
        network_tree(NetworkTreeSources {
            computers: &computers,
            locations: &places.network,
            expanded: &pane.expanded.borrow(),
            listed: &pane.listed.borrow(),
            show_hidden,
        })
    }

    /// Expands the computer at `uri` to its shares, or collapses it, as
    /// its chevron does.
    pub(super) fn toggle_computer(&self, uri: &str) {
        let Some(key) = computer_key_of(uri) else { return };
        let expand = !self.network_pane().expanded.borrow().contains(&key);
        self.expand_computer(uri, expand);
    }

    /// Expands the computer at `uri` when `expand`, else collapses it;
    /// false when it already is.
    pub(super) fn expand_computer(&self, uri: &str, expand: bool) -> bool {
        let Some(key) = computer_key_of(uri) else {
            return false;
        };
        let pane = self.network_pane();
        {
            let mut expanded = pane.expanded.borrow_mut();
            if expanded.contains(&key) == expand {
                return false;
            }
            expanded.retain(|known| *known != key);
            if expand {
                expanded.push(key.clone());
            }
        }
        if expand {
            self.list_computer_for_pane(uri, &key);
        } else {
            pane.listings.borrow_mut().retain(|(known, _)| *known != key);
        }
        self.render_places();
        true
    }

    /// Whether the computer at `uri` is expanded, for tests.
    #[cfg(test)]
    pub(super) fn is_computer_expanded(&self, uri: &str) -> bool {
        computer_key_of(uri).is_some_and(|key| self.network_pane().expanded.borrow().contains(&key))
    }

    /// Reads the shares of the computer at `uri`, whose key is `key`, for
    /// the navigation pane, mounting its server first when it needs it.
    fn list_computer_for_pane(&self, uri: &str, key: &str) {
        let window = self.downgrade();
        let (uri, key) = (uri.to_owned(), key.to_owned());
        let task_key = key.clone();
        let listing = loader::Listing::spawn(async move {
            let Some(found) = read_computer(&window, &uri).await else {
                return;
            };
            let Some(window) = window.upgrade() else { return };
            let pane = window.network_pane();
            pane.listings.borrow_mut().retain(|(known, _)| *known != key);
            if let Some(found) = found {
                let mut listed = pane.listed.borrow_mut();
                listed.retain(|(known, _)| *known != key);
                listed.push((key, found));
            }
            window.render_places();
        });
        let mut listings = self.network_pane().listings.borrow_mut();
        listings.retain(|(known, _)| *known != task_key);
        listings.push((task_key, listing));
    }

    /// Reads every expanded computer again (Refresh on Network).
    pub(super) fn relist_expanded_computers(&self) {
        let expanded = self.network_pane().expanded.borrow().clone();
        for node in self.network_computers(&self.places()) {
            if expanded.contains(&node.key) {
                self.list_computer_for_pane(&node.uri, &node.key);
            }
        }
    }

    /// Network was expanded in the navigation pane: looks for computers
    /// once this session, when nothing found any yet.
    pub(super) fn discover_for_network_pane(&self) {
        let pane = self.network_pane();
        if pane.discovered.get() || !self.context().network().known_computers().is_empty() {
            return;
        }
        pane.discovered.set(true);
        let discoverer = self.context().network().discoverer();
        let window = self.downgrade();
        let task = loader::Listing::spawn(async move {
            let _ = crate::network::discover_computers(
                &discoverer,
                |_| {},
                |computers| {
                    if let Some(window) = window.upgrade() {
                        window.context().remember_computers(computers);
                    }
                },
            )
            .await;
        });
        pane.discovery.replace(Some(task));
    }
}

/// The shares and services of the computer at `uri`: `Some(None)` when it
/// could not be read, `None` when the window closed.
async fn read_computer(window: &glib::WeakRef<BrowserWindow>, uri: &str) -> Option<Option<Vec<ListedShare>>> {
    let mut mounted = false;
    loop {
        let rows = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let (sender, receiver) = async_channel::bounded(1);
        let _listing = {
            let window = window.upgrade()?;
            let collected = std::rc::Rc::clone(&rows);
            let on_batch = move |entries: Vec<Entry>| collected.borrow_mut().extend(entries);
            let on_done = move |result: Result<(), EntryError>| {
                // The task awaiting it may have been dropped.
                let _ = sender.try_send(result);
            };
            match window.context().network().computer_at(uri) {
                Some(computer) => loader::list_computer(computer, on_batch, on_done),
                None => loader::list_folder(uri, on_batch, on_done),
            }
        };
        let result = receiver.recv().await.unwrap_or(Err(EntryError::Cancelled));
        match result {
            Ok(()) => {
                let shares = rows.borrow().iter().filter_map(listed_share).collect();
                return Some(Some(shares));
            }
            Err(error) if error.needs_mount() && !mounted => {
                mounted = true;
                let mounting = window.upgrade()?.network().mount(uri);
                if mounting.await.is_err() {
                    return Some(None);
                }
            }
            Err(EntryError::Cancelled) => return None,
            Err(error) => {
                glib::g_debug!(
                    ox_core::LOG_DOMAIN,
                    "The navigation pane could not list {uri}: {error}"
                );
                return Some(None);
            }
        }
    }
}
