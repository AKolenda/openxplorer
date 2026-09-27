// SPDX-License-Identifier: AGPL-3.0-only
//! Location changes and cancellation-safe asynchronous directory loading.

use std::rc::Rc;

use gtk::prelude::*;
use gtk::{gio, glib};
use ox_core::location::{self, LocationError};

use crate::folder_view::{item::FileItem, loader};
use crate::locations::Page;
use crate::{icons, locations};

use super::session::TabId;
use super::BrowserWindow;

impl BrowserWindow {
    fn normalize_address(&self, text: &str) -> Result<String, LocationError> {
        if let Some(page) = Page::from_address(text) {
            return Ok(page.uri().to_string());
        }
        let base = self.current_uri();
        let base = base
            .as_deref()
            .filter(|uri| Page::from_uri(uri).is_none())
            .unwrap_or(&self.home_uri);
        location::normalise_location(text, Some(base), &glib::home_dir())
    }

    /// Adds and activates a tab after validating its location.
    pub fn add_tab(self: &Rc<Self>, address: &str) -> Result<(), LocationError> {
        let uri = self.normalize_address(address)?;
        self.save_selection();
        let id = self.session.borrow_mut().add(&uri);
        self.show_tab(id);
        self.load_tab(id);
        Ok(())
    }

    /// Navigates the active tab. Invalid addresses leave the current folder intact.
    pub fn navigate(self: &Rc<Self>, address: &str) -> Result<(), LocationError> {
        let uri = self.normalize_address(address)?;
        let id = {
            let mut session = self.session.borrow_mut();
            let Some(id) = session.active else {
                drop(session);
                return self.add_tab(&uri);
            };
            let tab = session.tab_mut(id).expect("active tab exists");
            tab.history.push(&uri);
            tab.selected.clear();
            id
        };
        self.content.model.select_none();
        self.chrome.search.set_text("");
        self.content.model.set_query("");
        self.render_navigation();
        self.load_tab(id);
        Ok(())
    }

    pub(super) fn navigate_or_report(self: &Rc<Self>, address: &str) {
        if let Err(error) = self.navigate(address) {
            self.show_message(error.message());
        }
    }

    pub(super) fn save_selection(&self) {
        let selected = self
            .content
            .model
            .selected_items()
            .iter()
            .map(|item| item.entry().uri.clone())
            .collect();
        let mut session = self.session.borrow_mut();
        if let Some(tab) = session.active.and_then(|id| session.tab_mut(id)) {
            tab.selected = selected;
        }
    }

    pub(super) fn switch_tab(self: &Rc<Self>, id: TabId) {
        self.save_selection();
        if self.session.borrow().tab(id).is_none() {
            return;
        }
        self.session.borrow_mut().active = Some(id);
        self.show_tab(id);
    }

    fn show_tab(self: &Rc<Self>, id: TabId) {
        self.reset_typeahead();
        self.show_message("");
        let (store, selected) = {
            let session = self.session.borrow();
            let Some(tab) = session.tab(id) else { return };
            (tab.store.clone(), tab.selected.clone())
        };
        self.changing_model.set(true);
        self.chrome.search.set_text("");
        self.content.model.set_query("");
        self.content.model.set_store(Some(&store));
        self.content.model.select_uris(&selected);
        self.changing_model.set(false);
        self.render_navigation();
        self.update_content();
        self.update_inspector();
    }

    pub(super) fn close_tab(self: &Rc<Self>, id: TabId) {
        self.save_selection();
        let was_active = self.session.borrow().active == Some(id);
        self.session.borrow_mut().remove(id);
        if !was_active {
            self.render_tabs();
            return;
        }
        let active = self.session.borrow().active;
        match active {
            Some(id) => self.show_tab(id),
            None => self.window.close(),
        }
    }

    /// Reloads the active folder, preserving selection by URI.
    pub fn refresh(self: &Rc<Self>) {
        self.save_selection();
        let active = self.session.borrow().active;
        if let Some(id) = active {
            self.load_tab(id);
        }
    }

    /// Moves through the current tab's history; out-of-range movement is ignored.
    pub fn go_history(self: &Rc<Self>, delta: isize) {
        let id = {
            let mut session = self.session.borrow_mut();
            let Some(id) = session.active else { return };
            let tab = session.tab_mut(id).expect("active tab exists");
            if tab.history.go(delta).is_none() {
                return;
            }
            tab.selected.clear();
            id
        };
        self.chrome.search.set_text("");
        self.content.model.set_query("");
        self.content.model.select_none();
        self.render_navigation();
        self.load_tab(id);
    }

    fn load_tab(self: &Rc<Self>, id: TabId) {
        let is_active = self.session.borrow().active == Some(id);
        if is_active {
            self.reset_typeahead();
            self.show_message("");
        }
        let (uri, store, generation) = {
            let mut session = self.session.borrow_mut();
            let Some(tab) = session.tab_mut(id) else { return };
            let generation = tab.begin_load();
            (tab.history.current().to_string(), tab.store.clone(), generation)
        };
        self.changing_model.set(true);
        store.remove_all();
        self.changing_model.set(false);
        if Page::from_uri(&uri).is_some() {
            if let Some(tab) = self.session.borrow_mut().tab_mut(id) {
                tab.loading = false;
                tab.loaded = true;
            }
            if is_active {
                self.render_landing();
                self.update_content();
            }
            return;
        }
        if is_active {
            self.update_content();
        }
        let batch_window = Rc::downgrade(self);
        let done_window = Rc::downgrade(self);
        let listing = loader::list_folder(
            &uri,
            move |entries| {
                let Some(browser) = batch_window.upgrade() else {
                    return;
                };
                if !browser.session.borrow().accepts(id, generation) {
                    return;
                }
                let items: Vec<FileItem> = entries.into_iter().map(FileItem::new).collect();
                store.splice(store.n_items(), 0, &items);
                if browser.session.borrow().active == Some(id) {
                    browser.update_content();
                }
            },
            move |result| {
                let Some(browser) = done_window.upgrade() else {
                    return;
                };
                browser.finish_load(id, generation, result);
            },
        );
        if let Some(tab) = self.session.borrow_mut().tab_mut(id) {
            tab.listing = Some(listing);
        }
    }

    fn finish_load(self: &Rc<Self>, id: TabId, generation: u64, result: Result<(), loader::LoadError>) {
        let (uri, selected, watch) = {
            let mut session = self.session.borrow_mut();
            if !session.accepts(id, generation) {
                return;
            }
            let tab = session.tab_mut(id).expect("accepted tab exists");
            tab.loading = false;
            tab.loaded = true;
            tab.error = result.err().map(|error| error.message);
            (
                tab.history.current().to_string(),
                tab.selected.clone(),
                tab.error.is_none(),
            )
        };
        if watch {
            let weak = Rc::downgrade(self);
            let monitor = loader::watch_folder(&uri, move || {
                if let Some(browser) = weak.upgrade() {
                    if browser.session.borrow().active == Some(id) {
                        browser.save_selection();
                    }
                    browser.load_tab(id);
                }
            });
            if let Some(tab) = self.session.borrow_mut().tab_mut(id) {
                tab.watch = monitor;
            }
        }
        if self.session.borrow().active == Some(id) {
            self.content.model.select_uris(&selected);
            self.update_content();
            self.update_inspector();
        }
    }

    pub(super) fn update_content(&self) {
        let (page, loading, error) = {
            let session = self.session.borrow();
            let Some(tab) = session.active() else { return };
            (
                Page::from_uri(tab.history.current()),
                tab.loading,
                tab.error.clone(),
            )
        };
        self.chrome.search.set_sensitive(page.is_none());
        self.content.spinner.set_spinning(loading);
        self.content.spinner.set_visible(loading);
        if page.is_some() {
            self.content.stack.set_visible_child_name("landing");
        } else if self.content.model.n_items() > 0 {
            self.content.stack.set_visible_child_name("listing");
            if let Some(error) = error {
                self.show_message(&error);
            }
        } else {
            let (title, message) = match error {
                Some(error) => ("Could not open this folder", error),
                None if loading => ("Loading…", String::new()),
                None if self.content.model.is_searching() => {
                    ("No matching items", "Try a different filter.".to_string())
                }
                None => ("This folder is empty", String::new()),
            };
            self.content.empty_title.set_text(title);
            self.content.empty_message.set_text(&message);
            self.content.stack.set_visible_child_name("empty");
        }
        self.update_status();
    }

    pub(super) fn render_navigation(self: &Rc<Self>) {
        let (uri, back, forward) = {
            let session = self.session.borrow();
            let Some(tab) = session.active() else { return };
            (
                tab.history.current().to_string(),
                tab.history.can_go_back(),
                tab.history.can_go_forward(),
            )
        };
        self.window.set_title(Some(&format!(
            "{} — OpenXplorer",
            locations::title_for(&uri, &self.home_uri)
        )));
        self.chrome.entry.set_text(&locations::address_text(&uri));
        self.set_enabled("back", back);
        self.set_enabled("forward", forward);
        self.set_enabled("up", locations::parent(&uri).is_some());
        while let Some(child) = self.chrome.crumbs.first_child() {
            self.chrome.crumbs.remove(&child);
        }
        for (index, crumb) in locations::crumbs(&uri).into_iter().enumerate() {
            if index > 0 {
                self.chrome.crumbs.append(&icons::glyph("chevron", 11));
            }
            let button = gtk::Button::with_label(&crumb.label);
            button.add_css_class("crumb");
            let weak = Rc::downgrade(self);
            button.connect_clicked(move |_| {
                if let Some(browser) = weak.upgrade() {
                    browser.navigate_or_report(&crumb.uri);
                }
            });
            self.chrome.crumbs.append(&button);
        }
        self.render_tabs();
        self.select_sidebar(&uri);
        if Page::from_uri(&uri).is_some() {
            self.render_landing();
        }
    }

    fn render_tabs(self: &Rc<Self>) {
        while let Some(child) = self.chrome.tabs.first_child() {
            self.chrome.tabs.remove(&child);
        }
        let tabs: Vec<_> = self
            .session
            .borrow()
            .tabs
            .iter()
            .map(|tab| (tab.id, tab.history.current().to_string()))
            .collect();
        let active = self.session.borrow().active;
        for (id, uri) in tabs {
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
            row.add_css_class("tab");
            if active == Some(id) {
                row.add_css_class("active");
            }
            let title = locations::title_for(&uri, &self.home_uri);
            let label = gtk::Label::builder()
                .label(&title)
                .ellipsize(gtk::pango::EllipsizeMode::End)
                .max_width_chars(19)
                .build();
            let select = gtk::Button::builder()
                .child(&label)
                .tooltip_text(locations::address_text(&uri))
                .build();
            let weak = Rc::downgrade(self);
            select.connect_clicked(move |_| {
                if let Some(browser) = weak.upgrade() {
                    browser.switch_tab(id);
                }
            });
            row.append(&icons::glyph(
                Page::from_uri(&uri).map_or("folderline", Page::glyph),
                16,
            ));
            row.append(&select);
            let close = gtk::Button::builder()
                .child(&icons::glyph("close", 12))
                .tooltip_text(format!("Close {title}"))
                .build();
            close.add_css_class("tab-close");
            let weak = Rc::downgrade(self);
            close.connect_clicked(move |_| {
                if let Some(browser) = weak.upgrade() {
                    browser.close_tab(id);
                }
            });
            row.append(&close);
            self.chrome.tabs.append(&row);
        }
    }

    pub(super) fn edit_address(&self) {
        self.chrome.address.set_visible_child_name("entry");
        self.chrome.entry.grab_focus();
        self.chrome.entry.select_region(0, -1);
    }

    pub(super) fn finish_address(&self) {
        self.chrome.address.set_visible_child_name("crumbs");
        self.content.focus();
    }

    pub(super) fn activate_item(self: &Rc<Self>, position: u32) {
        let Some(item) = self.content.model.item(position) else {
            return;
        };
        if item.entry().is_dir {
            self.navigate_or_report(item.open_uri());
            return;
        }
        // Reaching this path requires an explicit activation (Enter, double
        // click or Open). Listing and selecting files never launches an app.
        let uri = item.open_uri().to_string();
        let context = gtk::prelude::WidgetExt::display(&self.window).app_launch_context();
        let weak = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            if let Err(error) = gio::AppInfo::launch_default_for_uri_future(&uri, Some(&context)).await {
                if let Some(browser) = weak.upgrade() {
                    browser.show_message(error.message());
                }
            }
        });
    }
}
