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
//! [`toast`], and over the folder pane the [`transfer_panel`] of the
//! running file operation. On the Settings tab the [`SettingsPage`] takes
//! the place of everything under the title bar ([`settings_tab`]).
//!
//! The controller lives in submodules, one job each: tab state
//! ([`session`], read through [`active_tab`]), changing location
//! ([`navigation`]) and drawing it ([`location_view`]), listing
//! ([`loading`]), the selection ([`selection`]), the desktop's volumes and
//! places ([`environment`]), Quick access ([`quick_access`]), connecting
//! and removing drives ([`mounting`]), network sign-in
//! ([`network_session`]), the network commands ([`network_actions`],
//! [`network_sign_out`]) and the places' menus ([`place_menus`]), the
//! skin ([`appearance`]), activation, actions, input
//! ([`type_to_select`]), the file operations and their [`dialog`]s
//! ([`file_ops`]), dragging and dropping files ([`file_drag`],
//! [`file_drop`]), moving tabs ([`tab_moves`]), the context menus
//! ([`context_menu`], [`tab_menu`]), and what the window connects and lets
//! go of ([`connections`]).
//! Widgets run window actions (`win.go-to`, `win.select-tab`, ...)
//! and report typing through calls of their own (such as
//! [`search_box::SearchBox::connect_query_changed`]), so the controller
//! never reaches into another widget's children; it connects directly only
//! to the window's own template children, such as the workspace split.

mod about;
mod actions;
mod activation;
mod active_tab;
mod address_bar;
mod appearance;
mod breakpoints;
mod button_style;
mod cache_folder;
mod caption_buttons;
mod card_grid;
mod command_bar;
mod connections;
mod context_menu;
mod copy_path;
mod details_pane;
mod dialog;
mod empty_page;
mod environment;
mod file_drag;
mod file_drop;
mod file_ops;
mod folder_pane;
mod folder_search;
mod gestures;
mod input;
mod landing;
mod listing_state;
mod loading;
mod loading_line;
mod location_view;
mod menu_popover;
mod mounting;
mod navigation;
mod navigation_buttons;
mod network_actions;
mod network_page;
mod network_session;
mod network_sign_out;
mod place_menus;
mod preferences;
mod quick_access;
mod search_box;
mod selection;
mod session;
mod settings_tab;
mod sidebar;
mod status_bar;
mod tab_layout;
mod tab_menu;
mod tab_moves;
mod tab_strip;
mod title_bar;
mod toast;
mod transfer_panel;
mod type_to_select;
mod unported;
mod widget_tree;
mod window_action;

#[cfg(test)]
mod tests;

use gtk::subclass::prelude::*;
use gtk::{gio, glib};

use crate::app_context::AppContext;
use crate::settings_page::SettingsPage;
use crate::theme::Skin;

use address_bar::AddressBar;
use command_bar::CommandBar;
use details_pane::DetailsPane;
use folder_pane::FolderPane;
use search_box::SearchBox;
use sidebar::Sidebar;
use status_bar::StatusBar;
use tab_strip::TabStrip;

pub(crate) use actions::install_accelerators;
pub(crate) use button_style::ButtonStyle;
pub(crate) use folder_pane::FolderView;
pub(crate) use search_box::{show_bundled_clear_icon, show_bundled_magnifier};
pub(crate) use title_bar::list_open_windows_on_click;
pub(crate) use unported::Milestone;
pub(crate) use widget_tree::children;
pub(crate) use window_action::WindowAction;

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
    use super::connections::ExternalHandlers;
    use super::details_pane::DetailsPane;
    use super::file_drag::OutgoingDrag;
    use super::file_drop::{FirstOffer, PendingDrop, ProgramChecks};
    use super::file_ops::FileOperations;
    use super::folder_pane::FolderPane;
    use super::menu_popover::MenuPopover;
    use super::search_box::SearchBox;
    use super::session::Session;
    use super::session::TabId;
    use super::settings_tab::SettingsTabState;
    use super::sidebar::Sidebar;
    use super::status_bar::StatusBar;
    use super::tab_moves::OutgoingTabDrag;
    use super::tab_strip::TabStrip;
    use super::toast::Toast;
    use super::transfer_panel::TransferPanel;
    use super::type_to_select::Typeahead;
    use crate::app_context::AppContext;
    use crate::network::WindowNetwork;
    use crate::search::{FolderSearch, SearchInfoStrip};
    use crate::settings_page::SettingsPage;
    use crate::volumes::VolumeRow;

    /// Private state of [`super::BrowserWindow`]: the parts of the frame
    /// it updates, then the state of its tabs.
    #[derive(Debug, Default, gtk::CompositeTemplate)]
    #[template(file = "../resources/ui/window.ui")]
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
        /// The history buttons, the address and the search box.
        #[template_child]
        pub(super) navigation_row: TemplateChild<gtk::Box>,
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
        /// The workspace, or the Settings page on the Settings tab.
        #[template_child]
        pub(super) surfaces: TemplateChild<gtk::Stack>,
        /// The split between the sidebar and the folder and details panes
        /// (`.sidebar-resizer`).
        #[template_child]
        pub(super) workspace: TemplateChild<gtk::Paned>,
        /// The navigation pane.
        #[template_child]
        pub(super) sidebar: TemplateChild<Sidebar>,
        /// What a search looked at, above the columns while searching.
        #[template_child]
        pub(super) search_strip: TemplateChild<SearchInfoStrip>,
        /// The folder pane.
        #[template_child]
        pub(super) folder_pane: TemplateChild<FolderPane>,
        /// The details pane beside the folder pane.
        #[template_child]
        pub(super) details_pane: TemplateChild<DetailsPane>,
        /// The running file operation's progress and Cancel, over the
        /// folder pane.
        #[template_child]
        pub(super) transfer_panel: TemplateChild<TransferPanel>,
        /// The message at the bottom of the workspace.
        #[template_child]
        pub(super) toast: TemplateChild<Toast>,
        /// Counts, the type-to-select hint and the view buttons.
        #[template_child]
        pub(super) status_bar: TemplateChild<StatusBar>,
        /// The Settings page, shown on the Settings tab.
        #[template_child]
        pub(super) settings_page: TemplateChild<SettingsPage>,
        /// The folder shown before Settings and the places Settings offers
        /// the search index.
        pub(super) settings_tab: RefCell<SettingsTabState>,
        /// What every window shares: the skin, settings and places. It
        /// comes from the application, so [`super::BrowserWindow::new`]
        /// sets it.
        pub(super) context: OnceCell<AppContext>,
        /// The desktop's volume monitor, set by `constructed`. Holding it
        /// keeps the monitor, and so its signals, alive.
        pub(super) volume_monitor: OnceCell<gio::VolumeMonitor>,
        /// The window's sign-in prompts and dialogs and its server
        /// discovery; [`super::BrowserWindow::new`] sets it.
        pub(super) network: OnceCell<WindowNetwork>,
        /// The tabs and which one is active.
        pub(super) session: RefCell<Session>,
        /// Display names of the home folder and the mounted devices.
        pub(super) locations: RefCell<LocationContext>,
        /// The drives and devices the volume monitor reported last.
        pub(super) volumes: RefCell<Vec<VolumeRow>>,
        /// The type-to-select prefix of the folder views.
        pub(super) typeahead: RefCell<Typeahead>,
        /// The search box's search.
        pub(super) search: RefCell<FolderSearch>,
        /// Set while the window swaps or reloads the model, so the
        /// selection it restores is not saved over the tab's selection.
        pub(super) changing_model: Cell<bool>,
        /// Set until the file list takes keyboard focus in a new window or
        /// after Settings hides; see
        /// [`super::BrowserWindow::focus_new_file_list`].
        pub(super) file_list_awaits_focus: Cell<bool>,
        /// The width band the layout was last fitted to.
        pub(super) window_width: Cell<WindowWidth>,
        /// What the window must disconnect when it goes away.
        pub(super) handlers: RefCell<ExternalHandlers>,
        /// The running file operation, Trash support and the file
        /// clipboard.
        pub(super) file_operations: RefCell<FileOperations>,
        /// The file drag this window started, while it lasts.
        pub(super) outgoing_drag: RefCell<Option<OutgoingDrag>>,
        /// Until when clicks that open items are ignored, around a drag.
        pub(super) item_clicks_resume_at: Cell<Option<std::time::Instant>>,
        /// Which files under a drag are programs (DND-026).
        pub(super) program_checks: RefCell<ProgramChecks>,
        /// Where the last drop happened, in the folder pane's
        /// coordinates, for the drop menu.
        pub(super) drop_point: Cell<(f64, f64)>,
        /// The drop that waits for the drop menu's answer.
        pub(super) pending_drop: RefCell<Option<PendingDrop>>,
        /// What the drag over the window offered when it arrived.
        pub(super) first_offer: RefCell<Option<FirstOffer>>,
        /// The drop menu, built when first needed.
        pub(super) drop_menu: OnceCell<MenuPopover>,
        /// The tab a file drag hovers over, and the timer that shows it.
        pub(super) tab_hover: RefCell<Option<(TabId, glib::SourceId)>>,
        /// The tab drag this window started, while it lasts.
        pub(super) outgoing_tab: RefCell<Option<OutgoingTabDrag>>,
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
            SearchInfoStrip::ensure_type();
            FolderPane::ensure_type();
            DetailsPane::ensure_type();
            TransferPanel::ensure_type();
            Toast::ensure_type();
            StatusBar::ensure_type();
            SettingsPage::ensure_type();
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
            self.obj().close_network();
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
            // A closed window's sign-ins end with it, even while something
            // still holds the window (SAFE-011, TAB-050).
            self.obj().close_network();
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
        window.start_network();
        window.install_actions();
        window.install_input();
        window.connect_signals();
        window.connect_settings_page();
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

    /// The Settings page.
    fn settings_page(&self) -> &SettingsPage {
        &self.imp().settings_page
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
}
