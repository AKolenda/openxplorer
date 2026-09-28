// SPDX-License-Identifier: AGPL-3.0-only
//! A browsing window with independent tab histories and listings.
//!
//! Ports the page structure and the controller of `desktop/ui/app.js`.
//! [`BrowserWindow`] is a `GtkApplicationWindow` subclass. Its parts live
//! in submodules, one job each: the frame ([`chrome`]), the folder pane
//! ([`content`]), the sidebar and landing pages ([`environment`]), tab
//! state ([`session`]), changing location ([`navigation`]) and drawing it
//! ([`location_view`]), listing ([`loading`]), the selection
//! ([`selection`]), the skin ([`appearance`]), activation, actions and
//! input. Widgets run window actions (`win.go-to`, `win.select-tab`, ...),
//! so the controller code does not reach into widget trees.

mod about;
mod actions;
mod activation;
mod address_bar;
mod appearance;
mod breakpoints;
mod button_style;
mod caption_buttons;
mod card_grid;
mod chrome;
mod command_bar;
mod content;
mod context_menu;
mod copy_path;
mod details_pane;
mod empty_page;
mod environment;
mod gestures;
mod input;
mod landing;
mod loading;
mod loading_line;
mod location_view;
mod menu_popover;
mod navigation;
mod network_page;
mod preferences;
mod search_box;
mod selection;
mod session;
mod sidebar;
mod status_bar;
mod tab_layout;
mod tab_strip;
mod title_bar;
mod toast;
mod unported;
mod widget_tree;

#[cfg(test)]
mod tests;

use gtk::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{gio, glib};

use crate::shared::AppContext;
use crate::theme::{ListenerId, Skin};
use crate::typeahead;

use chrome::Chrome;
use content::{Content, ContentPage};
use details_pane::DetailsPane;
use sidebar::Sidebar;

pub(crate) use actions::install_accelerators;
pub(crate) use content::FolderView;

/// Handlers this window registered on objects that outlive it.
#[derive(Debug, Default)]
struct ExternalHandlers {
    /// On the skin shared by every window.
    skin: Option<ListenerId>,
    /// On the application's `places-changed` signal.
    places: Option<glib::SignalHandlerId>,
    /// On the volume monitor's mount and volume signals.
    volumes: Vec<glib::SignalHandlerId>,
}

/// The type-to-select prefix and the timer that clears its hint.
#[derive(Debug, Default)]
struct TypeAhead {
    /// The typed prefix and the matching rules.
    controller: typeahead::Controller,
    /// Ends the prefix after a pause; it clears itself when it fires.
    timer: Option<glib::SourceId>,
}

mod imp {
    use std::cell::{Cell, OnceCell, RefCell};

    use gtk::subclass::prelude::*;
    use gtk::{gio, glib};
    use ox_core::location::LocationContext;

    use super::breakpoints::WindowWidth;
    use super::{Chrome, Content, DetailsPane, ExternalHandlers, Sidebar, TypeAhead};
    use crate::shared::AppContext;
    use crate::volumes::VolumeRow;
    use crate::window::session::Session;

    /// Private state of [`super::BrowserWindow`].
    #[derive(Debug, Default)]
    pub struct BrowserWindow {
        /// What every window shares: the skin, settings and places.
        pub(super) context: OnceCell<AppContext>,
        /// The frame around the workspace.
        pub(super) chrome: OnceCell<Chrome>,
        /// The folder pane.
        pub(super) content: OnceCell<Content>,
        /// The details pane beside the folder pane.
        pub(super) details_pane: OnceCell<DetailsPane>,
        /// The navigation pane.
        pub(super) sidebar: OnceCell<Sidebar>,
        /// The tabs and which one is active.
        pub(super) session: RefCell<Session>,
        /// Display names of the home folder and the mounted devices.
        pub(super) locations: RefCell<LocationContext>,
        /// The drives and devices the volume monitor reported last.
        pub(super) volumes: RefCell<Vec<VolumeRow>>,
        /// The desktop's volume monitor.
        pub(super) volume_monitor: OnceCell<gio::VolumeMonitor>,
        /// The type-to-select prefix of the folder views.
        pub(super) type_ahead: RefCell<TypeAhead>,
        /// Set while the window swaps or reloads the model, so the
        /// selection it restores is not saved over the tab's selection.
        pub(super) changing_model: Cell<bool>,
        /// Set until the file list first takes keyboard focus; see
        /// [`super::BrowserWindow::focus_new_file_list`].
        pub(super) file_list_awaits_focus: Cell<bool>,
        /// The width band the layout was last fitted to.
        pub(super) window_width: Cell<WindowWidth>,
        /// What the window must disconnect when it goes away.
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

    impl WidgetImpl for BrowserWindow {
        fn size_allocate(&self, width: i32, height: i32, baseline: i32) {
            self.parent_size_allocate(width, height, baseline);
            self.obj().follow_width(width);
        }
    }

    impl WindowImpl for BrowserWindow {
        fn close_request(&self) -> glib::Propagation {
            // Let go of keyboard focus first. On Wayland, GTK's input method
            // otherwise keeps the focused address entry and later asks a
            // destroyed widget for its cursor position (a Gtk-CRITICAL).
            gtk::prelude::GtkWindowExt::set_focus(&*self.obj(), None::<&gtk::Widget>);
            self.parent_close_request()
        }
    }
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
        imp.file_list_awaits_focus.set(true);
        // After GTK has finished showing the window, which ends by focusing
        // the first focusable widget.
        window.connect_map(|window| {
            glib::idle_add_local_once(glib::clone!(
                #[weak]
                window,
                move || window.focus_new_file_list()
            ));
        });
        window
    }

    /// Gives a new window's file list keyboard focus once the window is
    /// shown and its first location is listed, as `#main` has focus when
    /// app.js starts. A landing page or an empty folder has no list to
    /// focus, so nothing keeps focus: GTK would otherwise leave it on the
    /// first focusable widget, and a focused crumb draws the address bar's
    /// editing line.
    fn focus_new_file_list(&self) {
        let ready = self.is_mapped() && self.is_listed();
        if !ready || !self.imp().file_list_awaits_focus.replace(false) {
            return;
        }
        if self.content().page() == Some(ContentPage::Listing) {
            self.content().focus();
        } else {
            gtk::prelude::GtkWindowExt::set_focus(self, None::<&gtk::Widget>);
        }
    }

    fn lay_out_workspace(&self) {
        let pane = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        pane.append(&self.content().root);
        pane.append(&self.details_pane().root);
        let workspace = &self.chrome().workspace;
        workspace.set_start_child(Some(self.sidebar()));
        workspace.set_end_child(Some(&pane));
    }

    /// The state shared by every window of the application.
    fn context(&self) -> &AppContext {
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

    /// The active folder's sorted, filtered native selection model, for
    /// tests.
    #[cfg(test)]
    pub(crate) fn folder_model(&self) -> &crate::folder_view::model::FolderModel {
        &self.content().model
    }

    /// Number of tabs in this window.
    pub fn tab_count(&self) -> usize {
        self.imp().session.borrow().tabs.len()
    }

    /// The items of tab `id`, unfiltered and unsorted, while it is open.
    fn tab_store(&self, id: session::TabId) -> Option<gio::ListStore> {
        let session = self.imp().session.borrow();
        session.tab(id).map(|tab| tab.store.clone())
    }

    /// The active location, or `None` before the first tab is added.
    pub fn current_uri(&self) -> Option<String> {
        let session = self.imp().session.borrow();
        session.active().map(|tab| tab.uri().to_owned())
    }

    /// Whether the active tab has finished its first listing (a landing
    /// page counts as listed).
    pub fn is_listed(&self) -> bool {
        let session = self.imp().session.borrow();
        session.active().is_some_and(|tab| tab.loaded && !tab.loading)
    }

    /// Whether the active tab is still receiving directory entries.
    pub fn is_loading(&self) -> bool {
        self.imp()
            .session
            .borrow()
            .active()
            .is_some_and(|tab| tab.loading)
    }

    /// The active listing's failure, if one occurred, for tests.
    #[cfg(test)]
    fn load_error(&self) -> Option<String> {
        let session = self.imp().session.borrow();
        let error = session.active()?.error.as_ref()?;
        Some(error.to_string())
    }

    /// Shows a recoverable startup or integration message in the window.
    pub fn notify(&self, message: &str) {
        self.chrome().show_message(message);
    }

    fn connect_signals(&self) {
        self.follow_selection();
        self.connect_filter();
        self.connect_address_entry();
        self.connect_view_activation();
        self.follow_skin();
        gestures::connect_history_buttons(
            self,
            glib::clone!(
                #[weak(rename_to = window)]
                self,
                move |direction| window.go_history(direction)
            ),
        );
    }

    /// Filters the folder as the user types in the search box.
    fn connect_filter(&self) {
        self.chrome().search.entry.connect_search_changed(glib::clone!(
            #[weak(rename_to = window)]
            self,
            move |search| {
                window.content().model.set_query(search.text().as_str());
                window.update_content();
            }
        ));
    }

    /// Lets go of what the window registered on objects that outlive it:
    /// the skin, places and volume handlers and the type-to-select timer.
    fn disconnect_external_handlers(&self) {
        let handlers = self.imp().handlers.take();
        if let Some(listener) = handlers.skin {
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
}
