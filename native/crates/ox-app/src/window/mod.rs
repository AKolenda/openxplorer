// SPDX-License-Identifier: AGPL-3.0-only
//! A browsing window with independent tab histories and listings.
//!
//! Ports the page structure and the controller of `desktop/ui/app.js`.
//! [`BrowserWindow`] is a `GtkApplicationWindow` subclass whose frame, the
//! static layout of `desktop/ui/index.html`, is the template
//! `resources/ui/window.ui`. Each part of the frame is a widget with a
//! module of its own: the title bar ([`title_bar`], [`tab_strip`],
//! [`caption_buttons`]), the navigation row ([`navigation_buttons`],
//! [`address_bar`], [`search_box`]), the [`command_bar`], the [`sidebar`],
//! the [`folder_pane`], the [`details_pane`], the [`status_bar`] and the
//! [`toast`].
//!
//! The controller lives in submodules, one job each: tab state
//! ([`session`], read through [`active_tab`]), changing location
//! ([`navigation`]) and drawing it ([`location_view`]), listing
//! ([`loading`]), the selection ([`selection`]), the desktop's volumes and
//! places ([`environment`]), Quick access ([`quick_access`]), mounting
//! ([`mounting`]), the skin ([`appearance`]), activation, actions and
//! input. Widgets run window actions (`win.go-to`, `win.select-tab`, ...)
//! and report typing through calls of their own (such as
//! [`search_box::SearchBox::connect_query_changed`]), so the controller
//! never reaches into another widget's children; it connects directly only
//! to the window's own template children, such as the workspace split.
//!
//! Every module here is private, so a `pub` item could never be used
//! outside the crate; `unreachable_pub` makes the compiler ask for the
//! visibility each item really has.
#![warn(unreachable_pub)]

mod about;
mod actions;
mod activation;
mod active_tab;
mod address_bar;
mod appearance;
mod breakpoints;
mod button_style;
mod caption_buttons;
mod card_grid;
mod command_bar;
mod context_menu;
mod copy_path;
mod details_pane;
mod empty_page;
mod environment;
mod folder_pane;
mod gestures;
mod input;
mod landing;
mod listing_state;
mod loading;
mod loading_line;
mod location_kind;
mod location_view;
mod menu_popover;
mod mounting;
mod navigation;
mod navigation_buttons;
mod network_page;
mod preferences;
mod quick_access;
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
mod window_action;

#[cfg(test)]
mod tests;

use gtk::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{gio, glib};

use crate::shared::AppContext;
use crate::theme::{ListenerId, Skin};
use crate::typeahead;

use address_bar::AddressBar;
use command_bar::CommandBar;
use details_pane::DetailsPane;
use folder_pane::FolderPane;
use search_box::SearchBox;
use sidebar::Sidebar;
use status_bar::StatusBar;
use tab_strip::TabStrip;

pub(crate) use actions::install_accelerators;
pub(crate) use folder_pane::FolderView;
pub(crate) use location_kind::is_local_or_smb_location;

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
struct Typeahead {
    /// The typed prefix and the matching rules.
    controller: typeahead::Controller,
    /// Ends the prefix after a pause; it clears itself when it fires.
    timer: Option<glib::SourceId>,
}

mod imp {
    use std::cell::{Cell, OnceCell, RefCell};

    use gtk::prelude::*;
    use gtk::subclass::prelude::*;
    use gtk::{gio, glib};
    use ox_core::location::LocationContext;

    use super::address_bar::AddressBar;
    use super::breakpoints::WindowWidth;
    use super::caption_buttons::CaptionButtons;
    use super::command_bar::CommandBar;
    use super::details_pane::DetailsPane;
    use super::folder_pane::FolderPane;
    use super::search_box::SearchBox;
    use super::session::Session;
    use super::sidebar::Sidebar;
    use super::status_bar::StatusBar;
    use super::tab_strip::TabStrip;
    use super::toast::Toast;
    use super::{ExternalHandlers, Typeahead};
    use crate::shared::AppContext;
    use crate::volumes::VolumeRow;

    /// Private state of [`super::BrowserWindow`]: the parts of the frame
    /// it updates, then the state of its tabs.
    #[derive(Debug, Default, gtk::CompositeTemplate)]
    #[template(file = "../../resources/ui/window.ui")]
    pub(crate) struct BrowserWindow {
        /// The tabs in the title bar.
        #[template_child]
        pub(super) tab_strip: TemplateChild<TabStrip>,
        /// "+", right after the last tab.
        #[template_child]
        pub(super) new_tab_button: TemplateChild<gtk::Button>,
        /// Lists the open windows.
        #[template_child]
        pub(super) open_windows_button: TemplateChild<gtk::MenuButton>,
        /// Back, Forward, Up and Refresh.
        #[template_child]
        pub(super) navigation_buttons: TemplateChild<gtk::Box>,
        /// Breadcrumbs or the editable address.
        #[template_child]
        pub(super) address_bar: TemplateChild<AddressBar>,
        /// The search box that filters the folder.
        #[template_child]
        pub(super) search_box: TemplateChild<SearchBox>,
        /// New, the edit commands, Sort, View, More, appearance and Details.
        #[template_child]
        pub(super) command_bar: TemplateChild<CommandBar>,
        /// The split between the sidebar and the folder and details panes
        /// (`.sidebar-resizer`).
        #[template_child]
        pub(super) workspace: TemplateChild<gtk::Paned>,
        /// The navigation pane.
        #[template_child]
        pub(super) sidebar: TemplateChild<Sidebar>,
        /// The folder pane.
        #[template_child]
        pub(super) folder_pane: TemplateChild<FolderPane>,
        /// The details pane beside the folder pane.
        #[template_child]
        pub(super) details_pane: TemplateChild<DetailsPane>,
        /// The message at the bottom of the workspace.
        #[template_child]
        pub(super) toast: TemplateChild<Toast>,
        /// Counts, the type-to-select hint and the view buttons.
        #[template_child]
        pub(super) status_bar: TemplateChild<StatusBar>,
        /// What every window shares: the skin, settings and places. It
        /// comes from the application, so [`super::BrowserWindow::new`]
        /// sets it.
        pub(super) context: OnceCell<AppContext>,
        /// The desktop's volume monitor, set by `constructed`. Holding it
        /// keeps the monitor, and so its signals, alive.
        pub(super) volume_monitor: OnceCell<gio::VolumeMonitor>,
        /// The tabs and which one is active.
        pub(super) session: RefCell<Session>,
        /// Display names of the home folder and the mounted devices.
        pub(super) locations: RefCell<LocationContext>,
        /// The drives and devices the volume monitor reported last.
        pub(super) volumes: RefCell<Vec<VolumeRow>>,
        /// The type-to-select prefix of the folder views.
        pub(super) typeahead: RefCell<Typeahead>,
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

        fn class_init(klass: &mut Self::Class) {
            // GtkBuilder finds the template's own types by name, so they
            // must be registered before the template is parsed.
            CaptionButtons::ensure_type();
            TabStrip::ensure_type();
            AddressBar::ensure_type();
            SearchBox::ensure_type();
            CommandBar::ensure_type();
            Sidebar::ensure_type();
            FolderPane::ensure_type();
            DetailsPane::ensure_type();
            Toast::ensure_type();
            StatusBar::ensure_type();
            klass.bind_template();
        }

        fn instance_init(window: &glib::subclass::InitializingObject<Self>) {
            window.init_template();
        }
    }

    impl ObjectImpl for BrowserWindow {
        fn constructed(&self) {
            self.parent_constructed();
            let window = self.obj();
            window.finish_title_bar();
            window.add_navigation_buttons();
            self.volume_monitor
                .set(gio::VolumeMonitor::get())
                .expect("constructed runs once per object");
        }

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
            GtkWindowExt::set_focus(&*self.obj(), None::<&gtk::Widget>);
            self.parent_close_request()
        }
    }

    impl ApplicationWindowImpl for BrowserWindow {}
}

glib::wrapper! {
    /// One OpenXplorer window: tabs, sidebar, folder views and details.
    pub(crate) struct BrowserWindow(ObjectSubclass<imp::BrowserWindow>)
        @extends gtk::ApplicationWindow, gtk::Window, gtk::Widget,
        @implements gio::ActionGroup, gio::ActionMap, gtk::Accessible, gtk::Buildable,
            gtk::ConstraintTarget, gtk::Native, gtk::Root, gtk::ShortcutManager;
}

impl BrowserWindow {
    /// Creates an empty window of `app` sharing `context`. Add a tab with
    /// [`Self::add_tab`] before presenting it.
    pub(crate) fn new(app: &gtk::Application, context: &AppContext) -> Self {
        let window: Self = glib::Object::builder().property("application", app).build();
        window
            .imp()
            .context
            .set(context.clone())
            .expect("a new window has no context yet");
        window.install_actions();
        window.install_input();
        window.connect_signals();
        window.watch_environment();
        window.apply_preferences();
        window.focus_file_list_once_shown();
        window
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

    fn volume_monitor(&self) -> &gio::VolumeMonitor {
        self.imp()
            .volume_monitor
            .get()
            .expect("constructed gets the volume monitor")
    }

    /// The tabs in the title bar.
    fn tab_strip(&self) -> &TabStrip {
        &self.imp().tab_strip
    }

    /// The breadcrumbs or the editable address.
    fn address_bar(&self) -> &AddressBar {
        &self.imp().address_bar
    }

    /// The search box that filters the folder.
    fn search_box(&self) -> &SearchBox {
        &self.imp().search_box
    }

    /// The command bar under the navigation row.
    fn command_bar(&self) -> &CommandBar {
        &self.imp().command_bar
    }

    /// The split between the sidebar and the panes beside it.
    fn workspace(&self) -> &gtk::Paned {
        &self.imp().workspace
    }

    /// The navigation pane.
    fn sidebar(&self) -> &Sidebar {
        &self.imp().sidebar
    }

    /// The folder pane.
    fn folder_pane(&self) -> &FolderPane {
        &self.imp().folder_pane
    }

    /// The details pane.
    fn details_pane(&self) -> &DetailsPane {
        &self.imp().details_pane
    }

    /// The status bar.
    fn status_bar(&self) -> &StatusBar {
        &self.imp().status_bar
    }

    /// The active folder's sorted, filtered native selection model, for
    /// tests.
    #[cfg(test)]
    pub(crate) fn folder_model(&self) -> &crate::folder_view::model::FolderModel {
        self.folder_pane().model()
    }

    /// Shows a message in the window's toast: a refused command, a
    /// failure, or a recoverable startup or integration problem.
    pub(crate) fn show_message(&self, message: &str) {
        self.imp().toast.show(message);
    }

    /// Hides the toast's message at once, as moving to another folder or
    /// tab does.
    fn hide_message(&self) {
        self.imp().toast.hide();
    }

    /// The message the toast showed last, for tests.
    #[cfg(test)]
    fn shown_message(&self) -> glib::GString {
        self.imp().toast.text()
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
        self.search_box().connect_query_changed(glib::clone!(
            #[weak(rename_to = window)]
            self,
            move |query| {
                window.folder_pane().model().set_query(query);
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
        if let Some(timer) = self.imp().typeahead.borrow_mut().timer.take() {
            timer.remove();
        }
    }
}
