// SPDX-License-Identifier: AGPL-3.0-only
//! The live walk of a folder nobody indexed, with its subfolders
//! (SRCH-035): the listing's matches are shown at once, and the walk's
//! matches are added as they are found. The walk itself is
//! [`ox_core::search::walk_search`].

use gtk::gio;
use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;
use std::collections::HashSet;

use ox_core::entry::Entry;
use ox_core::search::{walk_search, LiveSearch, NamePattern, SearchError};

use crate::folder_view::item::FileItem;
use crate::search::{listed_name_matches, Listing, SearchRun, RESULT_LIMIT};

use super::BrowserWindow;

impl BrowserWindow {
    /// Walks the folder of `run` and its subfolders off the main thread,
    /// adding the matches to the rows as they are found, after the
    /// listing's own (SRCH-035). A newer edit cancels the walk.
    pub(super) fn walk_folder(&self, run: SearchRun) {
        let rows = gio::ListStore::new::<FileItem>();
        let mut shown = HashSet::new();
        let active = self.imp().session.borrow().active_id();
        if let Some(store) = active.and_then(|id| self.tab_store(id)) {
            let listing = Listing {
                items: &store,
                folder: &run.folder,
                shows_hidden: self.hidden_files_shown(),
            };
            let listed = listed_name_matches(listing, &run.text);
            shown.extend(listed.iter().map(|item| item.entry().uri.clone()));
            rows.extend_from_slice(&listed);
        }
        self.imp().search.borrow_mut().show_found(&run, rows.clone());
        self.show_searched_items();
        let search = LiveSearch {
            folder: run.folder.clone(),
            pattern: NamePattern::new(&run.text),
            hidden_items: self.hidden_items(),
            limit: RESULT_LIMIT,
            search_in: run.search_in,
        };
        let (sender, batches) = async_channel::unbounded::<Vec<Entry>>();
        let cancellable = run.cancellable.clone();
        let walk = gio::spawn_blocking(move || {
            walk_search(&search, &cancellable, &mut |batch| {
                // Fails only once the window stopped listening.
                let _ = sender.send_blocking(batch);
            })
        });
        let window = self.downgrade();
        glib::spawn_future_local(async move {
            let mut is_truncated = false;
            while let Ok(batch) = batches.recv().await {
                let new = batch.into_iter().filter(|entry| shown.insert(entry.uri.clone()));
                let room = RESULT_LIMIT.saturating_sub(rows.n_items() as usize);
                let found: Vec<FileItem> = new.map(FileItem::new).collect();
                is_truncated |= found.len() > room;
                rows.extend_from_slice(&found[..found.len().min(room)]);
            }
            let outcome = walk.await.unwrap_or(Err(SearchError::Cancelled));
            if let Some(window) = window.upgrade() {
                let outcome = outcome.map(|end| end.is_truncated || is_truncated);
                window.finish_walk(&run, rows, outcome);
            }
        });
    }

    /// Shows the rows the walk of `run` found, or why it failed; a late
    /// answer never replaces a newer one (SAFE-013). `outcome` says
    /// whether more matched than are shown.
    fn finish_walk(&self, run: &SearchRun, rows: gio::ListStore, outcome: Result<bool, SearchError>) {
        let is_current = self.imp().search.borrow().is_current(run);
        let is_same_folder = self.current_uri().as_deref() == Some(run.folder.as_str());
        if !is_current || !is_same_folder {
            return;
        }
        match outcome {
            Ok(is_truncated) => self.imp().search.borrow_mut().finish(run, rows, is_truncated),
            Err(SearchError::Cancelled) => return,
            Err(error) => self.imp().search.borrow_mut().fail(run, error.to_string()),
        }
        self.show_searched_items();
        self.show_search_state();
    }
}
