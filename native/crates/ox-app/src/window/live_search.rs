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
        let mut is_truncated = false;
        let active = self.imp().session.borrow().active_id();
        if let Some(store) = active.and_then(|id| self.tab_store(id)) {
            let listing = Listing {
                items: &store,
                folder: &run.folder,
                shows_hidden: self.hidden_files_shown(),
            };
            let listed = listed_name_matches(listing, &run.text);
            is_truncated = listed.is_truncated;
            shown.extend(listed.items.iter().map(|item| item.entry().uri.clone()));
            rows.extend_from_slice(&listed.items);
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
            while let Ok(batch) = batches.recv().await {
                let new = batch.into_iter().filter(|entry| shown.insert(entry.uri.clone()));
                let room = RESULT_LIMIT.saturating_sub(rows.n_items() as usize);
                let mut found: Vec<Entry> = new.take(room + 1).collect();
                is_truncated |= found.len() > room;
                found.truncate(room);
                let items: Vec<FileItem> = found.into_iter().map(FileItem::new).collect();
                rows.extend_from_slice(&items);
            }
            let outcome = walk.await.unwrap_or(Err(SearchError::Cancelled));
            if let Some(window) = window.upgrade() {
                let outcome = outcome.map(|end| end.is_truncated || is_truncated);
                window.finish_walk(&run, rows, outcome);
            }
        });
    }

    /// Updates the rows of a finished walk after the folder was listed
    /// again: its own matches that went away are removed and new ones
    /// added, in place, so the rows the walk found below it stay.
    pub(super) fn update_walked_listing(&self) {
        let Some(rows) = self.imp().search.borrow().results().cloned() else {
            return;
        };
        let text = self.imp().search.borrow().query().trim().to_owned();
        let Some(folder) = self.current_uri() else {
            return;
        };
        let active = self.imp().session.borrow().active_id();
        let Some(store) = active.and_then(|id| self.tab_store(id)) else {
            return;
        };
        let listing = Listing {
            items: &store,
            folder: &folder,
            shows_hidden: self.hidden_files_shown(),
        };
        let listed = listed_name_matches(listing, &text).items;
        let listed_uris: HashSet<String> = listed.iter().map(|item| item.entry().uri.clone()).collect();
        let parent = gio::File::for_uri(&folder);
        for position in (0..rows.n_items()).rev() {
            let Some(item) = rows.item(position).and_downcast::<FileItem>() else {
                continue;
            };
            let uri = &item.entry().uri;
            let in_folder = gio::File::for_uri(uri)
                .parent()
                .is_some_and(|file| file.equal(&parent));
            if in_folder && !listed_uris.contains(uri) {
                rows.remove(position);
            }
        }
        let shown: HashSet<String> = rows
            .iter::<FileItem>()
            .filter_map(Result::ok)
            .map(|item| item.entry().uri.clone())
            .collect();
        let room = RESULT_LIMIT.saturating_sub(rows.n_items() as usize);
        let new: Vec<FileItem> = listed
            .into_iter()
            .filter(|item| !shown.contains(&item.entry().uri))
            .take(room)
            .collect();
        rows.extend_from_slice(&new);
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
