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
//! - A share that is not mounted is mounted once, asking for credentials
//!   if needed, and listed again from empty rows (`retry_list` in
//!   winspace.py, NET-004). A server being signed out is not listed
//!   (NET-023), and a listed SMB location joins the session's Network
//!   list (NET-016).

mod mount_retry;

use std::cell::RefCell;
use std::rc::Rc;

use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;
use ox_core::entry::{Entry, EntryError};
use ox_core::location::{parent_location, same_location, TRASH_URI};

use crate::folder_view::item::FileItem;
use crate::folder_view::{loader, reconcile, watch};
use crate::locations::Page;

use super::empty_page::EmptyState;
use super::folder_pane::PanePage;
use super::listing_state::{ListingEnd, ListingState, ReloadTiming};
use super::session::TabId;
use super::BrowserWindow;
use mount_retry::MountRetry;

/// Why a tab is listed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum LoadMode {
    /// The tab moved to this location: start empty, show rows as they come.
    Navigate,
    /// The same location again: keep the rows until the listing is done.
    Reload,
}

/// The start of a load: what is listed, and which load it is.
#[derive(Debug)]
struct LoadStart {
    /// The location the tab shows.
    uri: String,
    /// Results of an older generation are ignored.
    generation: u64,
}

/// One listing of one tab.
#[derive(Debug)]
struct LoadRun {
    tab: TabId,
    /// The location listed.
    uri: String,
    /// Results of an older generation are ignored.
    generation: u64,
    mode: LoadMode,
    /// Whether an unmounted share may still be mounted.
    mount_retry: MountRetry,
    /// A reload's rows, held back until the listing is complete.
    held_rows: RefCell<Vec<Entry>>,
}

impl BrowserWindow {
    /// Lists tab `id`'s location.
    pub(super) fn load_tab(&self, id: TabId, mode: LoadMode) {
        let is_active = self.imp().session.borrow().is_active(id);
        if is_active {
            self.reset_typeahead();
            self.hide_message();
        }
        let Some(start) = self.begin_load(id) else {
            return;
        };
        if let Some(page) = Page::from_uri(&start.uri) {
            self.finish_page(id, page);
            return;
        }
        if mode == LoadMode::Navigate {
            self.clear_rows(id);
        }
        let signing_out = self.context().network().sign_out_registry();
        if let Err(refusal) = signing_out.check_listing(&start.uri) {
            self.refuse_listing(id, mode, EntryError::Failed(refusal.to_string()));
            return;
        }
        self.keep_watching(id, &start.uri);
        if is_active {
            self.update_content();
        }
        let listing = self.start_listing(id, &start, mode, MountRetry::Allowed);
        if let Some(tab) = self.imp().session.borrow_mut().tab_mut(id) {
            tab.listing = Some(listing);
        }
    }

    /// Ends tab `id`'s listing with `error` before anything was read; a
    /// cancelled one ends without a message.
    fn refuse_listing(&self, id: TabId, mode: LoadMode, error: EntryError) {
        if error != EntryError::Cancelled {
            self.fail_load(id, mode, error);
        }
        let end = self.imp().session.borrow_mut().end_listing(id);
        if end != ListingEnd::TabClosed && self.imp().session.borrow().is_active(id) {
            self.update_content();
        }
    }

    /// Starts a load of tab `id`, or `None` once the tab has closed.
    fn begin_load(&self, id: TabId) -> Option<LoadStart> {
        let mut session = self.imp().session.borrow_mut();
        let tab = session.tab_mut(id)?;
        let generation = tab.begin_load();
        let uri = tab.uri().to_owned();
        Some(LoadStart { uri, generation })
    }

    /// A landing page needs no listing and no watch: it is listed as soon
    /// as it is shown. The Network page's first showing starts discovery.
    fn finish_page(&self, id: TabId, page: Page) {
        if let Some(tab) = self.imp().session.borrow_mut().tab_mut(id) {
            tab.listing_state = ListingState::Listed;
            tab.watch = None;
        }
        if !self.imp().session.borrow().is_active(id) {
            return;
        }
        if page == Page::Network {
            self.discover_servers_once();
        }
        self.render_landing();
        self.update_content();
    }

    fn clear_rows(&self, id: TabId) {
        let Some(store) = self.tab_store(id) else { return };
        self.change_model(|| store.remove_all());
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
        let timing = {
            let mut session = self.imp().session.borrow_mut();
            let Some(tab) = session.tab_mut(id) else { return };
            tab.listing_state.schedule_reload()
        };
        if timing == ReloadTiming::AfterRunningListing {
            return;
        }
        if self.imp().session.borrow().is_active(id) {
            self.save_selection();
        }
        self.load_tab(id, LoadMode::Reload);
    }

    fn start_listing(
        &self,
        id: TabId,
        start: &LoadStart,
        mode: LoadMode,
        mount_retry: MountRetry,
    ) -> loader::Listing {
        let run = Rc::new(LoadRun {
            tab: id,
            uri: start.uri.clone(),
            generation: start.generation,
            mode,
            mount_retry,
            held_rows: RefCell::default(),
        });
        let batch_run = Rc::clone(&run);
        loader::list_folder(
            &start.uri,
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
        let items: Vec<FileItem> = entries.into_iter().map(FileItem::new).collect();
        if let Some(store) = self.tab_store(run.tab) {
            store.splice(store.n_items(), 0, &items);
        }
        if self.imp().session.borrow().is_active(run.tab) {
            self.update_content();
        }
    }

    fn finish_load(&self, run: &LoadRun, result: Result<(), EntryError>) {
        let id = run.tab;
        if !self.imp().session.borrow().accepts(id, run.generation) {
            return;
        }
        let is_listed = result.is_ok();
        match result {
            Ok(()) if run.mode == LoadMode::Reload => self.merge_rows(id, run.held_rows.take()),
            Ok(()) | Err(EntryError::Cancelled) => {}
            Err(EntryError::NotDirectory(_)) => {
                self.open_folder_of_file(id, run.mode);
                return;
            }
            Err(error) if error.needs_mount() && run.mount_retry == MountRetry::Allowed => {
                self.mount_and_list_again(run);
                return;
            }
            Err(error) => self.fail_load(id, run.mode, error),
        }
        if is_listed {
            // NET-016: a listed share joins Network for the session only.
            self.context().remember_network(&run.uri);
        }
        let end = self.imp().session.borrow_mut().end_listing(id);
        if end == ListingEnd::TabClosed {
            return;
        }
        self.apply_measured_folder_sizes(id);
        if self.imp().session.borrow().is_active(id) {
            self.restore_selection(id);
            self.update_content();
            self.update_details_pane();
            self.focus_new_file_list();
            self.restore_scroll_after_listing(id);
            self.reveal_located_item(id);
        }
        if end == ListingEnd::ListAgain {
            self.folder_changed(id);
        }
    }

    /// Merges a completed reload into the rows, keeping unchanged items.
    fn merge_rows(&self, id: TabId, entries: Vec<Entry>) {
        let Some(store) = self.tab_store(id) else { return };
        self.change_model(|| reconcile::update_in_place(&store, entries));
    }

    /// Records why a listing failed and stops watching a folder that cannot
    /// be read. A failed reload shows the error instead of stale rows; a
    /// first listing keeps the rows that arrived, with the error above them.
    fn fail_load(&self, id: TabId, mode: LoadMode, error: EntryError) {
        if mode == LoadMode::Reload {
            self.clear_rows(id);
        }
        if let Some(tab) = self.imp().session.borrow_mut().tab_mut(id) {
            tab.error = Some(error);
            tab.watch = None;
        }
    }

    /// What an empty folder says: "Recycle Bin is empty" there, as
    /// Dolphin's "Trash is empty" (OPS-040), else "This folder is empty".
    fn empty_folder_state(&self) -> EmptyState {
        let shows_recycle_bin = self
            .current_uri()
            .is_some_and(|uri| same_location(&uri, TRASH_URI));
        if shows_recycle_bin {
            EmptyState::EmptyRecycleBin
        } else {
            EmptyState::EmptyFolder
        }
    }

    /// Selects the tab's saved selection again, and scrolls to its first
    /// item when a Show in folder request asked for that, or starts
    /// renaming it when Tab moved a rename on to it (OPS-012).
    fn restore_selection(&self, id: TabId) {
        let (selected, reveals, renames) = {
            let mut session = self.imp().session.borrow_mut();
            let Some(tab) = session.tab_mut(id) else { return };
            (
                tab.selected.clone(),
                std::mem::take(&mut tab.reveals_selection),
                std::mem::take(&mut tab.renames_selection),
            )
        };
        self.change_model(|| self.folder_pane().model().select_uris(&selected));
        let first = self.folder_pane().model().first_selected();
        if let (true, Some(position)) = (reveals || renames, first) {
            self.folder_pane().reveal(position);
        }
        if renames && first.is_some() {
            self.continue_renaming();
        }
    }

    /// Scrolls to the position a moved tab brought along, now that its
    /// items are listed (TAB-039).
    fn restore_scroll_after_listing(&self, id: TabId) {
        let scroll = {
            let mut session = self.imp().session.borrow_mut();
            session
                .tab_mut(id)
                .and_then(|tab| tab.scroll_after_listing.take())
        };
        if let Some(scroll) = scroll {
            self.folder_pane().restore_scroll_position(scroll);
        }
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
            tab.listing_state.stop();
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
            let loading = tab.listing_state.is_listing();
            (page, loading, tab.error.as_ref().map(ToString::to_string))
        };
        let pane = self.folder_pane();
        pane.set_loading(loading && page.is_none());
        if page.is_some() {
            pane.show_page(PanePage::Landing);
        } else if pane.model().n_items() > 0 {
            pane.show_page(PanePage::Listing);
            if let Some(error) = error {
                self.show_message(&error);
            }
        } else {
            let state = match error {
                Some(error) => EmptyState::Unavailable(error),
                None if loading => EmptyState::Loading,
                None => self
                    .search_empty_state()
                    .unwrap_or_else(|| self.empty_folder_state()),
            };
            pane.show_empty(&state);
        }
        self.update_status();
        self.update_file_commands();
        self.learn_trash_support();
    }
}
