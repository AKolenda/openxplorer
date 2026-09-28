// SPDX-License-Identifier: AGPL-3.0-only
//! A browsing window with independent tab histories and listings.
//!
//! [`BrowserWindow`] is a `GtkApplicationWindow` subclass. Its parts live
//! in submodules, one job each: the frame ([`chrome`]), the folder pane
//! ([`content`]), the sidebar and landing pages ([`environment`]), tab
//! state ([`session`]), navigation and loading, activation, actions and
//! input. Widgets run window actions (`win.go-to`, `win.select-tab`, ...),
//! so the controller code does not reach into widget trees.

mod about;
mod actions;
mod activation;
mod address_bar;
mod art_style;
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
mod menu_popover;
mod navigation;
mod network_page;
mod preferences;
mod search_box;
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

use crate::folder_view::model::FolderModel;
use crate::locations::Page;
use crate::shared::AppContext;
use crate::theme::{Appearance, ListenerId, Skin, SkinChange};
use crate::typeahead;

use chrome::Chrome;
use content::{Content, ContentPage};
use details_pane::{DetailsPane, PaneFacts};
use sidebar::Sidebar;
use status_bar::StatusSubject;

pub(crate) use actions::install_accelerators;
pub(crate) use content::FolderView;

/// Handlers this window registered on objects that outlive it.
#[derive(Debug, Default)]
struct ExternalHandlers {
    skin: Option<ListenerId>,
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
        /// Set until the file list first takes keyboard focus; see
        /// [`super::BrowserWindow::focus_new_file_list`].
        pub(super) file_list_awaits_focus: Cell<bool>,
        /// The width band the layout was last fitted to.
        pub(super) window_width: Cell<WindowWidth>,
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
        self.connect_skin();
        gestures::connect_history_buttons(
            self,
            glib::clone!(
                #[weak(rename_to = window)]
                self,
                move |direction| window.go_history(direction)
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
        let selected = self.content().model.summary().count;
        self.set_action_enabled("open", selected == 1);
        // Copy path copies one item, or the folder when none is selected.
        self.set_action_enabled("copy-path", selected <= 1);
    }

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

    fn connect_skin(&self) {
        let listener = self.skin().connect_changed(glib::clone!(
            #[weak(rename_to = window)]
            self,
            move |change| window.skin_changed(change)
        ));
        self.imp().handlers.borrow_mut().skin = Some(listener);
        self.show_appearance_choice();
        self.content().set_text_size(self.skin().text_size());
        self.connect_scale_factor_notify(|window| {
            window.content().icons.redraw();
            window.render_places();
            window.update_details_pane();
        });
    }

    fn skin_changed(&self, change: SkinChange) {
        match change {
            SkinChange::Appearance(appearance) => self.appearance_changed(appearance),
            SkinChange::TextSize(percent) => self.content().set_text_size(percent),
        }
    }

    fn appearance_changed(&self, appearance: Appearance) {
        self.content().icons.set_appearance(appearance);
        self.render_places();
        // Redraws the tabs' and the address bar's colour art too.
        self.render_location();
        self.update_details_pane();
        self.show_appearance_choice();
    }

    /// Shows the chosen and drawn appearance on the Appearance button and
    /// in the Appearance menu.
    fn show_appearance_choice(&self) {
        let preference = self.skin().preference();
        let appearance = self.skin().appearance();
        self.chrome()
            .commands
            .show_appearance(appearance, &preference.tooltip(appearance));
        self.set_action_state("theme", &preference.key().to_variant());
    }

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

    fn update_status(&self) {
        let on_page = self.current_uri().as_deref().and_then(Page::from_uri).is_some();
        let subject = if on_page {
            StatusSubject::Page
        } else {
            StatusSubject::Folder {
                shown: self.content().model.n_items(),
                loading: self.is_loading(),
            }
        };
        let selected = self.content().model.summary();
        self.chrome().status.show(subject, selected);
    }

    fn update_details_pane(&self) {
        let selection = self.content().model.selected_items();
        let Some(folder_uri) = self.current_uri() else {
            return;
        };
        let store = self.imp().session.borrow().active().map(|tab| tab.store.clone());
        let model = &self.content().model;
        let folder_item_count = store.map_or(0, |store| model.listed_count(&store));
        let locations = self.imp().locations.borrow();
        let content = details_pane::pane_content(&PaneFacts {
            selection: &selection,
            folder_uri: &folder_uri,
            folder_item_count,
            locations: &locations,
        });
        self.details_pane().show(&content, self.art_style());
    }
}
