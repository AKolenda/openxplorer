// SPDX-License-Identifier: AGPL-3.0-only
//! The live walk of a folder nobody indexed, with its subfolders
//! (SRCH-035): the listing's matches are shown at once, and the walk's
//! matches are added as they are found. The walk itself is
//! [`ox_core::search::walk_search`].

use gtk::gio;
use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;
use std::collections::{HashMap, HashSet};

use ox_core::entry::Entry;
use ox_core::search::{
    matches_by_contents, walk_search, walks_subfolders, LiveSearch, NamePattern, SearchError, SearchIn,
};

use crate::folder_view::filter::Visibility;
use crate::folder_view::item::FileItem;
use crate::search::{listed_name_matches, Listing, SearchRun, RESULT_LIMIT};

use super::BrowserWindow;

/// A file's size and modification time, which tell whether it changed
/// since a search of its contents read it.
pub(super) type FileStamp = (Option<u64>, Option<u64>);

/// The size and modification time of `entry`.
fn stamp(entry: &Entry) -> FileStamp {
    (entry.size, entry.modified)
}

/// The items of the folder's listing `store` a search sees: the hidden
/// ones only while hidden files are shown.
fn searched_entries(store: &gio::ListStore, shows_hidden: bool) -> impl Iterator<Item = Entry> + '_ {
    store
        .iter::<FileItem>()
        .filter_map(Result::ok)
        .filter(move |item| shows_hidden || item.visibility() == Visibility::Visible)
        .map(|item| item.entry().clone())
}

impl BrowserWindow {
    /// Walks the folder of `run` and its subfolders off the main thread,
    /// adding the matches to the rows as they are found, after the
    /// listing's own (SRCH-035). A newer edit cancels the walk.
    pub(super) fn walk_folder(&self, run: SearchRun) {
        let rows = gio::ListStore::new::<FileItem>();
        let mut shown = HashSet::new();
        let mut is_truncated = false;
        let active = self.imp().session.borrow().active_id();
        let mut checked = HashMap::new();
        if let Some(store) = active.and_then(|id| self.tab_store(id)) {
            let shows_hidden = self.hidden_files_shown();
            // The walk reads each of these: a later listing tells the
            // ones that changed or came since.
            if run.search_in == SearchIn::NamesAndContents {
                checked = searched_entries(&store, shows_hidden)
                    .map(|entry| (entry.uri.clone(), stamp(&entry)))
                    .collect();
            }
            let listing = Listing {
                items: &store,
                folder: &run.folder,
                shows_hidden,
            };
            let listed = listed_name_matches(listing, &run.text);
            is_truncated = listed.is_truncated;
            shown.extend(listed.items.iter().map(|item| item.entry().uri.clone()));
            rows.extend_from_slice(&listed.items);
        }
        self.imp().contents_checked.replace(checked);
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
    /// added, in place, so the rows the walk found below it stay. A search
    /// of names and contents keeps a file it found by its text while the
    /// file is still listed, as it was: its name alone does not match. One
    /// that changed is read again, and goes if its text no longer matches;
    /// one of the folder's files the results lack is read when it changed
    /// or came since it was last read, and added if its text matches now.
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
        let shows_hidden = self.hidden_files_shown();
        let listing = Listing {
            items: &store,
            folder: &folder,
            shows_hidden,
        };
        let listed = listed_name_matches(listing, &text).items;
        let listed_uris: HashSet<String> = listed.iter().map(|item| item.entry().uri.clone()).collect();
        // A search of contents: the folder's files as listed now, for the
        // ones it found by their text.
        let searches_contents = self.imp().search.borrow().search_in() == SearchIn::NamesAndContents;
        let still_listed: HashMap<String, Entry> = if searches_contents {
            searched_entries(&store, shows_hidden)
                .map(|entry| (entry.uri.clone(), entry))
                .collect()
        } else {
            HashMap::new()
        };
        let mut changed = Vec::new();
        let parent = gio::File::for_uri(&folder);
        for position in (0..rows.n_items()).rev() {
            let Some(item) = rows.item(position).and_downcast::<FileItem>() else {
                continue;
            };
            let uri = &item.entry().uri;
            let in_folder = gio::File::for_uri(uri)
                .parent()
                .is_some_and(|file| file.equal(&parent));
            if !in_folder || listed_uris.contains(uri) {
                continue;
            }
            match still_listed.get(uri) {
                Some(now) if is_unchanged(item.entry(), now) => {}
                Some(now) => changed.push(now.clone()),
                None => rows.remove(position),
            }
        }
        let shown: HashSet<String> = rows
            .iter::<FileItem>()
            .filter_map(Result::ok)
            .map(|item| item.entry().uri.clone())
            .collect();
        let unlisted = self.contents_to_check(&folder, &still_listed, &shown, &listed_uris);
        self.recheck_contents(&rows, &text, changed, unlisted);
        let room = RESULT_LIMIT.saturating_sub(rows.n_items() as usize);
        let new: Vec<FileItem> = listed
            .into_iter()
            .filter(|item| !shown.contains(&item.entry().uri))
            .take(room)
            .collect();
        rows.extend_from_slice(&new);
    }

    /// The folder's files listed as `still_listed` that a search of
    /// contents should read: not among the results `shown` nor matching
    /// by name (`listed`), and changed or new since they were last read.
    /// The others count as read now. Only files on this computer's own
    /// disks are read, as the walk reads them.
    fn contents_to_check(
        &self,
        folder: &str,
        still_listed: &HashMap<String, Entry>,
        shown: &HashSet<String>,
        listed: &HashSet<String>,
    ) -> Vec<Entry> {
        let mut checked = self.imp().contents_checked.borrow_mut();
        if still_listed.is_empty() || !walks_subfolders(folder) {
            checked.clear();
            return Vec::new();
        }
        let unlisted: Vec<Entry> = still_listed
            .values()
            .filter(|entry| !shown.contains(&entry.uri) && !listed.contains(&entry.uri))
            .filter(|entry| checked.get(&entry.uri) != Some(&stamp(entry)))
            .cloned()
            .collect();
        let unread: HashSet<&str> = unlisted.iter().map(|entry| entry.uri.as_str()).collect();
        // A file still to read keeps its old stamp until it is read, so a
        // newer listing reads it again if this read is dropped.
        checked.retain(|uri, _| unread.contains(uri.as_str()));
        checked.extend(
            still_listed
                .iter()
                .filter(|(uri, _)| !unread.contains(uri.as_str()))
                .map(|(uri, entry)| (uri.clone(), stamp(entry))),
        );
        unlisted
    }

    /// Reads the files of `changed` and `unlisted` off the main thread,
    /// removes from `rows` those of `changed` whose text no longer matches
    /// `text`, and adds those of `unlisted` whose text matches now, unless
    /// the search, or a newer listing of its folder, has moved on
    /// meanwhile.
    fn recheck_contents(&self, rows: &gio::ListStore, text: &str, changed: Vec<Entry>, unlisted: Vec<Entry>) {
        let generation = self.imp().content_recheck.get().wrapping_add(1);
        self.imp().content_recheck.set(generation);
        if changed.is_empty() && unlisted.is_empty() {
            return;
        }
        let pattern = NamePattern::new(text);
        let check = gio::spawn_blocking(move || {
            let unmatched: HashSet<String> = changed
                .into_iter()
                .filter(|entry| !matches_by_contents(&pattern, entry))
                .map(|entry| entry.uri)
                .collect();
            let read: Vec<(String, FileStamp)> = unlisted
                .iter()
                .map(|entry| (entry.uri.clone(), stamp(entry)))
                .collect();
            let matched: Vec<Entry> = unlisted
                .into_iter()
                .filter(|entry| matches_by_contents(&pattern, entry))
                .collect();
            (unmatched, read, matched)
        });
        let window = self.downgrade();
        let rows = rows.clone();
        glib::spawn_future_local(async move {
            let Ok((unmatched, read, matched)) = check.await else {
                return;
            };
            let Some(window) = window.upgrade() else { return };
            let is_shown = window.imp().search.borrow().results() == Some(&rows);
            let is_latest = window.imp().content_recheck.get() == generation;
            if !is_shown || !is_latest {
                return;
            }
            window.imp().contents_checked.borrow_mut().extend(read);
            if unmatched.is_empty() && matched.is_empty() {
                return;
            }
            for position in (0..rows.n_items()).rev() {
                let item = rows.item(position).and_downcast::<FileItem>();
                if item.is_some_and(|item| unmatched.contains(&item.entry().uri)) {
                    rows.remove(position);
                }
            }
            let shown: HashSet<String> = rows
                .iter::<FileItem>()
                .filter_map(Result::ok)
                .map(|item| item.entry().uri.clone())
                .collect();
            let room = RESULT_LIMIT.saturating_sub(rows.n_items() as usize);
            let found: Vec<FileItem> = matched
                .into_iter()
                .filter(|entry| !shown.contains(&entry.uri))
                .take(room)
                .map(FileItem::new)
                .collect();
            rows.extend_from_slice(&found);
            window.show_search_state();
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
        self.restore_results_view();
    }
}

/// Whether a file listed `now` is the one a search found as `found`: the
/// same size and modification time.
fn is_unchanged(found: &Entry, now: &Entry) -> bool {
    found.size == now.size && found.modified == now.modified
}
