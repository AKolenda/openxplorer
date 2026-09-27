// SPDX-License-Identifier: AGPL-3.0-only
//! Listing a tab's folder and keeping it current.
//!
//! Ports `load` and the directory-monitor refresh in `desktop/ui/app.js`
//! and `desktop/winspace.py`:
//!
//! - Moving to a folder clears the rows and fills them batch by batch.
//! - Listing the same folder again (F5, or a change the monitor saw) keeps
//!   the rows on screen and merges the new listing in when it is complete,
//!   so scroll position, keyboard focus and selection survive.
//! - The folder watch lives as long as the tab shows the folder, so changes
//!   made while a listing runs are not lost: they list it once more after.
//! - A location that turns out to be a file opens its folder instead, and
//!   the file itself when the user asked for it.

use std::cell::RefCell;
use std::rc::Rc;

use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;
use ox_core::entry::{Entry, EnumerateError};
use ox_core::location::parent_location;

use crate::folder_view::item::FileItem;
use crate::folder_view::{loader, reconcile, watch};
use crate::locations::Page;

use super::content::{ContentPage, EmptyState};
use super::session::TabId;
use super::BrowserWindow;

/// Why a tab is listed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum LoadMode {
    /// The tab moved to this location: start empty, show rows as they come.
    Navigate,
    /// The same location again: keep the rows until the listing is done.
    Reload,
}

/// One listing of one tab.
#[derive(Debug)]
struct LoadRun {
    tab: TabId,
    /// Results of an older generation are ignored.
    generation: u64,
    mode: LoadMode,
    /// A reload's rows, held back until the listing is complete.
    held_rows: RefCell<Vec<Entry>>,
}

impl BrowserWindow {
    /// Lists tab `id`'s location.
    pub(super) fn load_tab(&self, id: TabId, mode: LoadMode) {
        let is_active = self.imp().session.borrow().is_active(id);
        if is_active {
            self.reset_typeahead();
            self.chrome().show_message("");
        }
        let Some((uri, generation)) = self.begin_load(id) else {
            return;
        };
        if Page::from_uri(&uri).is_some() {
            self.finish_page(id, is_active);
            return;
        }
        self.keep_watching(id, &uri);
        if mode == LoadMode::Navigate {
            self.clear_rows(id);
        }
        if is_active {
            self.update_content();
        }
        let listing = self.start_listing(id, &uri, generation, mode);
        if let Some(tab) = self.imp().session.borrow_mut().tab_mut(id) {
            tab.listing = Some(listing);
        }
    }

    /// Starts a load of tab `id`; its location and generation.
    fn begin_load(&self, id: TabId) -> Option<(String, u64)> {
        let mut session = self.imp().session.borrow_mut();
        let tab = session.tab_mut(id)?;
        let generation = tab.begin_load();
        Some((tab.uri().to_owned(), generation))
    }

    /// A landing page needs no listing and no watch.
    fn finish_page(&self, id: TabId, is_active: bool) {
        if let Some(tab) = self.imp().session.borrow_mut().tab_mut(id) {
            tab.loading = false;
            tab.loaded = true;
            tab.watch = None;
        }
        if is_active {
            self.render_landing();
            self.update_content();
        }
    }

    fn clear_rows(&self, id: TabId) {
        let store = self.imp().session.borrow().tab(id).map(|tab| tab.store.clone());
        let Some(store) = store else { return };
        self.imp().changing_model.set(true);
        store.remove_all();
        self.imp().changing_model.set(false);
    }

    /// Watches `uri` for tab `id`, keeping the existing watch when the tab
    /// already watches it. Watching starts before the listing, so changes
    /// made during a first listing are seen too.
    fn keep_watching(&self, id: TabId, uri: &str) {
        let watched = self
            .imp()
            .session
            .borrow()
            .tab(id)
            .and_then(|tab| tab.watch.as_ref().map(|watch| watch.uri().to_owned()));
        if watched.as_deref() == Some(uri) {
            return;
        }
        let watch = watch::watch_folder(
            uri,
            glib::clone!(
                #[weak(rename_to = window)]
                self,
                move || window.folder_changed(id)
            ),
        );
        if let Some(tab) = self.imp().session.borrow_mut().tab_mut(id) {
            tab.watch = Some(watch);
        }
    }

    /// The watched folder changed: list it again, or once more after the
    /// listing that is running now.
    pub(super) fn folder_changed(&self, id: TabId) {
        let loading = {
            let mut session = self.imp().session.borrow_mut();
            let Some(tab) = session.tab_mut(id) else { return };
            tab.reload_pending = tab.loading;
            tab.loading
        };
        if loading {
            return;
        }
        if self.imp().session.borrow().is_active(id) {
            self.save_selection();
        }
        self.load_tab(id, LoadMode::Reload);
    }

    fn start_listing(&self, id: TabId, uri: &str, generation: u64, mode: LoadMode) -> loader::Listing {
        let run = Rc::new(LoadRun {
            tab: id,
            generation,
            mode,
            held_rows: RefCell::default(),
        });
        let batch_run = Rc::clone(&run);
        loader::list_folder(
            uri,
            glib::clone!(
                #[weak(rename_to = window)]
                self,
                move |entries| window.receive_batch(&batch_run, entries)
            ),
            glib::clone!(
                #[weak(rename_to = window)]
                self,
                move |result| window.finish_load(&run, result)
            ),
        )
    }

    fn receive_batch(&self, run: &LoadRun, entries: Vec<Entry>) {
        if !self.imp().session.borrow().accepts(run.tab, run.generation) {
            return;
        }
        if run.mode == LoadMode::Reload {
            run.held_rows.borrow_mut().extend(entries);
            return;
        }
        let store = self
            .imp()
            .session
            .borrow()
            .tab(run.tab)
            .map(|tab| tab.store.clone());
        let items: Vec<FileItem> = entries.into_iter().map(FileItem::new).collect();
        if let Some(store) = store {
            store.splice(store.n_items(), 0, &items);
        }
        if self.imp().session.borrow().is_active(run.tab) {
            self.update_content();
        }
    }

    fn finish_load(&self, run: &LoadRun, result: Result<(), EnumerateError>) {
        let id = run.tab;
        if !self.imp().session.borrow().accepts(id, run.generation) {
            return;
        }
        match result {
            Ok(()) if run.mode == LoadMode::Reload => self.merge_rows(id, run.held_rows.take()),
            Ok(()) | Err(EnumerateError::Cancelled) => {}
            Err(EnumerateError::NotDirectory(_)) => {
                self.open_folder_of_file(id, run.mode);
                return;
            }
            Err(error) => self.fail_load(id, run.mode, error),
        }
        let reload_again = {
            let mut session = self.imp().session.borrow_mut();
            let Some(tab) = session.tab_mut(id) else { return };
            tab.loading = false;
            tab.loaded = true;
            std::mem::take(&mut tab.reload_pending)
        };
        if self.imp().session.borrow().is_active(id) {
            self.restore_selection(id);
            self.update_content();
            self.update_details_pane();
        }
        if reload_again {
            self.folder_changed(id);
        }
    }

    /// Merges a completed reload into the rows, keeping unchanged items.
    fn merge_rows(&self, id: TabId, entries: Vec<Entry>) {
        let store = self.imp().session.borrow().tab(id).map(|tab| tab.store.clone());
        let Some(store) = store else { return };
        self.imp().changing_model.set(true);
        reconcile::update_in_place(&store, entries);
        self.imp().changing_model.set(false);
    }

    /// Records why a listing failed and stops watching a folder that cannot
    /// be read. A failed reload shows the error instead of stale rows; a
    /// first listing keeps the rows that arrived, with the error above them.
    fn fail_load(&self, id: TabId, mode: LoadMode, error: EnumerateError) {
        if mode == LoadMode::Reload {
            self.clear_rows(id);
        }
        if let Some(tab) = self.imp().session.borrow_mut().tab_mut(id) {
            tab.error = Some(error);
            tab.watch = None;
        }
    }

    fn restore_selection(&self, id: TabId) {
        let selected = self
            .imp()
            .session
            .borrow()
            .tab(id)
            .map(|tab| tab.selected.clone())
            .unwrap_or_default();
        self.imp().changing_model.set(true);
        self.content().model.select_uris(&selected);
        self.imp().changing_model.set(false);
    }

    /// The tab's location is a file: show its folder (or home) in place of
    /// the file, and open the file when the user navigated to it. A reload
    /// never opens anything, so a folder replaced by a file cannot start an
    /// application by itself (`load()` in app.js, `not-directory`).
    fn open_folder_of_file(&self, id: TabId, mode: LoadMode) {
        let home = self.imp().locations.borrow().home_uri();
        let file = {
            let mut session = self.imp().session.borrow_mut();
            let Some(tab) = session.tab_mut(id) else { return };
            let file = tab.uri().to_owned();
            let folder = parent_location(&file).unwrap_or(home);
            tab.history.replace_current(&folder);
            tab.loading = false;
            tab.watch = None;
            file
        };
        if self.imp().session.borrow().is_active(id) {
            self.render_navigation();
        }
        self.load_tab(id, LoadMode::Navigate);
        if mode == LoadMode::Navigate {
            self.open_file_location(&file);
        }
    }

    /// Shows the folder pane state that fits the active tab.
    pub(super) fn update_content(&self) {
        let (page, loading, error) = {
            let session = self.imp().session.borrow();
            let Some(tab) = session.active() else { return };
            let page = Page::from_uri(tab.uri());
            (page, tab.loading, tab.error.as_ref().map(ToString::to_string))
        };
        let content = self.content();
        content.show_loading_line(loading && page.is_none());
        if page.is_some() {
            content.show_page(ContentPage::Landing);
        } else if content.model.n_items() > 0 {
            content.show_page(ContentPage::Listing);
            if let Some(error) = error {
                self.chrome().show_message(&error);
            }
        } else {
            let state = match error {
                Some(error) => EmptyState::Unavailable(error),
                None if loading => EmptyState::Loading,
                None if content.model.is_searching() => EmptyState::NoMatches,
                None => EmptyState::EmptyFolder,
            };
            content.show_empty(&state);
        }
        self.update_status();
    }
}
