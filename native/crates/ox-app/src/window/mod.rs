// SPDX-License-Identifier: AGPL-3.0-only
//! A browsing window with independent tab histories and listings.
//!
//! [`BrowserWindow`] is a `GtkApplicationWindow` subclass. Its parts live
//! in submodules, one job each: the frame ([`chrome`]), the folder pane
//! ([`content`]), the sidebar and landing pages ([`environment`]), tab
//! state ([`session`]), navigation and loading, activation, actions and
//! input. Widgets run window actions (`win.go-to`, `win.select-tab`, ...),
//! so the controller code does not reach into widget trees.

mod actions;
mod activation;
mod address_bar;
mod chrome;
mod content;
mod details_pane;
mod environment;
mod gestures;
mod input;
mod landing;
mod loading;
mod navigation;
mod preferences;
mod session;
mod sidebar;
mod tab_strip;

#[cfg(test)]
mod tests;

use std::cell::{Cell, OnceCell, RefCell};

use gtk::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{gio, glib};
use ox_core::location::LocationContext;

use crate::folder_view::model::FolderModel;
use crate::shared::AppContext;
use crate::theme::{Appearance, ListenerId, Skin};
use crate::typeahead;
use crate::volumes::VolumeRow;

use chrome::Chrome;
use content::Content;
use details_pane::DetailsPane;
use session::Session;
use sidebar::Sidebar;

pub(crate) use actions::install_accelerators;

/// Handlers this window registered on objects that outlive it.
#[derive(Debug, Default)]
struct ExternalHandlers {
    appearance: Option<ListenerId>,
    places: Option<glib::SignalHandlerId>,
    volumes: Vec<glib::SignalHandlerId>,
}

/// The type-to-select prefix and the timer that clears its hint.
#[derive(Debug, Default)]
struct TypeAhead {
    controller: typeahead::Controller,
    timer: Option<glib::SourceId>,
}

mod imp {
    use super::*;

    /// Private state of [`super::BrowserWindow`].
    #[derive(Debug, Default)]
    pub struct BrowserWindow {
        pub(super) context: OnceCell<AppContext>,
        pub(super) chrome: OnceCell<Chrome>,
        pub(super) content: OnceCell<Content>,
        pub(super) details_pane: OnceCell<DetailsPane>,
        pub(super) sidebar: OnceCell<Sidebar>,
        pub(super) session: RefCell<Session>,
        pub(super) locations: RefCell<LocationContext>,
        pub(super) volumes: RefCell<Vec<VolumeRow>>,
        pub(super) volume_monitor: OnceCell<gio::VolumeMonitor>,
        pub(super) type_ahead: RefCell<TypeAhead>,
        /// Set while the window swaps or reloads the model, so the
        /// selection it restores is not saved over the tab's selection.
        pub(super) changing_model: Cell<bool>,
        pub(super) handlers: RefCell<ExternalHandlers>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for BrowserWindow {
        const NAME: &'static str = "OxBrowserWindow";
        type Type = super::BrowserWindow;
        type ParentType = gtk::ApplicationWindow;
    }

    impl ObjectImpl for BrowserWindow {
        fn dispose(&self) {
            self.obj().disconnect_external_handlers();
            // Dropping the tabs cancels their listings and folder watches.
            self.session.take();
        }
    }

    impl WidgetImpl for BrowserWindow {}
    impl WindowImpl for BrowserWindow {}
    impl ApplicationWindowImpl for BrowserWindow {}
}

glib::wrapper! {
    /// One OpenXplorer window: tabs, sidebar, folder views and details.
    pub struct BrowserWindow(ObjectSubclass<imp::BrowserWindow>)
        @extends gtk::ApplicationWindow, gtk::Window, gtk::Widget,
        @implements gio::ActionGroup, gio::ActionMap, gtk::Accessible, gtk::Buildable,
            gtk::ConstraintTarget, gtk::Native, gtk::Root, gtk::ShortcutManager;
}

impl BrowserWindow {
    /// Creates an empty window of `app` sharing `context`. Add a tab with
    /// [`Self::add_tab`] before presenting it.
    ///
    /// # Panics
    ///
    /// Never: a new window has none of its parts yet.
    pub fn new(app: &gtk::Application, context: &AppContext) -> Self {
        let window: Self = glib::Object::builder()
            .property("application", app)
            .property("title", "OpenXplorer")
            .property("default-width", 1320)
            .property("default-height", 810)
            .build();
        window.add_css_class("ox");
        let imp = window.imp();
        let appearance = context.skin().appearance();
        let chrome = Chrome::new(window.upcast_ref());
        let content = Content::new(appearance);
        let details_pane = DetailsPane::new(appearance);
        let sidebar = Sidebar::new();
        let parts_are_new = imp.context.set(context.clone()).is_ok()
            && imp.chrome.set(chrome).is_ok()
            && imp.content.set(content).is_ok()
            && imp.details_pane.set(details_pane).is_ok()
            && imp.sidebar.set(sidebar).is_ok()
            && imp.volume_monitor.set(gio::VolumeMonitor::get()).is_ok();
        assert!(parts_are_new, "a new window has none of its parts yet");
        window.lay_out_workspace();
        window.install_actions();
        window.install_input();
        window.connect_signals();
        window.watch_environment();
        window.apply_preferences();
        window
    }

    fn lay_out_workspace(&self) {
        let pane = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        pane.append(&self.content().root);
        pane.append(&self.details_pane().root);
        let workspace = &self.chrome().workspace;
        workspace.set_start_child(Some(&self.sidebar().root));
        workspace.set_end_child(Some(&pane));
    }

    /// The state shared by every window of the application.
    pub(crate) fn context(&self) -> &AppContext {
        self.imp()
            .context
            .get()
            .expect("BrowserWindow::new sets the context")
    }

    fn skin(&self) -> &Skin {
        self.context().skin()
    }

    fn chrome(&self) -> &Chrome {
        self.imp()
            .chrome
            .get()
            .expect("BrowserWindow::new builds the chrome")
    }

    fn content(&self) -> &Content {
        self.imp()
            .content
            .get()
            .expect("BrowserWindow::new builds the content")
    }

    fn details_pane(&self) -> &DetailsPane {
        self.imp()
            .details_pane
            .get()
            .expect("BrowserWindow::new builds the details pane")
    }

    fn sidebar(&self) -> &Sidebar {
        self.imp()
            .sidebar
            .get()
            .expect("BrowserWindow::new builds the sidebar")
    }

    fn volume_monitor(&self) -> &gio::VolumeMonitor {
        self.imp()
            .volume_monitor
            .get()
            .expect("BrowserWindow::new gets the volume monitor")
    }

    /// The active folder's sorted, filtered native selection model.
    pub fn folder_model(&self) -> &FolderModel {
        &self.content().model
    }

    /// Number of tabs in this window.
    pub fn tab_count(&self) -> usize {
        self.imp().session.borrow().tabs.len()
    }

    /// The active location, or `None` before the first tab is added.
    pub fn current_uri(&self) -> Option<String> {
        let session = self.imp().session.borrow();
        session.active().map(|tab| tab.uri().to_owned())
    }

    /// Whether the active tab is still receiving directory entries.
    pub fn is_loading(&self) -> bool {
        self.imp()
            .session
            .borrow()
            .active()
            .is_some_and(|tab| tab.loading)
    }

    /// The active listing's failure, if one occurred.
    pub fn load_error(&self) -> Option<String> {
        let session = self.imp().session.borrow();
        let error = session.active()?.error.as_ref()?;
        Some(error.to_string())
    }

    /// Shows a recoverable startup or integration message in the window.
    pub fn notify(&self, message: &str) {
        self.chrome().show_message(message);
    }

    fn connect_signals(&self) {
        self.connect_selection_signals();
        self.connect_filter();
        self.connect_address_entry();
        self.connect_view_activation();
        self.connect_appearance();
        gestures::connect_history_buttons(
            self,
            glib::clone!(
                #[weak(rename_to = window)]
                self,
                move |step| window.go_history(step)
            ),
        );
    }

    fn connect_selection_signals(&self) {
        let model = &self.content().model;
        model.selection().connect_selection_changed(glib::clone!(
            #[weak(rename_to = window)]
            self,
            move |_, _, _| window.selection_changed()
        ));
        model.sorted().connect_items_changed(glib::clone!(
            #[weak(rename_to = window)]
            self,
            move |_, _, _, _| window.update_status()
        ));
    }

    fn selection_changed(&self) {
        if !self.imp().changing_model.get() {
            self.save_selection();
        }
        self.update_status();
        self.update_details_pane();
        let exactly_one = self.content().model.summary().count == 1;
        self.set_action_enabled("open", exactly_one);
    }

    fn connect_filter(&self) {
        self.chrome().search.connect_search_changed(glib::clone!(
            #[weak(rename_to = window)]
            self,
            move |search| {
                window.content().model.set_query(search.text().as_str());
                window.update_content();
            }
        ));
    }

    fn connect_appearance(&self) {
        let listener = self.skin().connect_changed(glib::clone!(
            #[weak(rename_to = window)]
            self,
            move |appearance| window.appearance_changed(appearance)
        ));
        self.imp().handlers.borrow_mut().appearance = Some(listener);
        self.show_appearance_choice();
        self.connect_scale_factor_notify(|window| {
            window.content().icons.redraw();
            window.render_places();
            window.update_details_pane();
        });
    }

    fn appearance_changed(&self, appearance: Appearance) {
        self.content().icons.set_appearance(appearance);
        self.render_places();
        self.render_tabs();
        self.update_details_pane();
        self.show_appearance_choice();
    }

    /// Shows the chosen and drawn appearance on the Appearance button and
    /// in the Appearance menu.
    fn show_appearance_choice(&self) {
        let preference = self.skin().preference();
        let appearance = self.skin().appearance();
        self.chrome()
            .show_appearance(appearance, &preference.tooltip(appearance));
        self.set_action_state("theme", &preference.key().to_variant());
    }

    fn disconnect_external_handlers(&self) {
        let handlers = self.imp().handlers.take();
        if let Some(listener) = handlers.appearance {
            self.skin().disconnect_changed(listener);
        }
        if let Some(handler) = handlers.places {
            self.context().disconnect(handler);
        }
        for handler in handlers.volumes {
            self.volume_monitor().disconnect(handler);
        }
        if let Some(timer) = self.imp().type_ahead.borrow_mut().timer.take() {
            timer.remove();
        }
    }

    fn update_status(&self) {
        let count = self.content().model.n_items();
        let selected = self.content().model.summary();
        let mut parts = vec![if count == 1 {
            "1 item".to_owned()
        } else {
            format!("{count} items")
        }];
        if selected.count > 0 {
            parts.push(format!("{} selected", selected.count));
        }
        if selected.count > 0 && selected.has_files {
            parts.push(ox_core::format::pretty_bytes(selected.bytes));
        }
        if self.is_loading() {
            parts.push("Loading…".to_owned());
        }
        self.chrome().status.set_text(&parts.join("  ·  "));
    }

    fn update_details_pane(&self) {
        let selection = self.content().model.selected_items();
        let Some(folder_uri) = self.current_uri() else {
            return;
        };
        let folder_item_count = self
            .imp()
            .session
            .borrow()
            .active()
            .map_or(0, |tab| tab.store.n_items());
        let locations = self.imp().locations.borrow();
        let content = details_pane::pane_content(&details_pane::PaneFacts {
            selection: &selection,
            folder_uri: &folder_uri,
            folder_item_count,
            locations: &locations,
        });
        let appearance = self.skin().appearance();
        self.details_pane()
            .show(&content, appearance, self.scale_factor());
    }
}
