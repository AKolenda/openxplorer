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
//! ([`context_menu`], [`tab_menu`]), searching ([`folder_search`],
//! [`cache_folder`]), Properties and previous versions ([`item_dialogs`],
//! [`snapshot_tabs`], [`version_restore`]), folder sizes
//! ([`folder_size_scan`]), moving a relocated standard folder's files
//! ([`relocated_files`]), ZIP archives ([`archive_actions`]), requests
//! from other applications and the command line ([`external_requests`]),
//! Open with, Open in Terminal and updates ([`integration_actions`]),
//! closing while files are written ([`closing`]), and what the window
//! connects and lets go of ([`connections`]).
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
mod address_completion;
mod address_menu;
mod address_options;
mod address_protocols;
mod appearance;
mod archive_actions;
mod breakpoints;
mod button_style;
mod cache_folder;
mod caption_buttons;
mod card_grid;
mod closing;
mod command_bar;
mod compress_dialog;
mod connections;
mod context_menu;
mod copy_path;
mod crumb_drop;
mod crumb_menus;
mod details_hover;
mod details_pane;
mod dialog;
mod disabled_reasons;
mod disk_tools;
mod empty_page;
mod environment;
mod external_requests;
mod file_drag;
mod file_drop;
mod file_ops;
mod folder_pane;
mod folder_search;
mod folder_size_scan;
mod free_space;
mod gestures;
mod history_menu;
mod grid_keys;
mod imp;
mod input;
mod integration_actions;
mod item_dialogs;
mod landing;
mod listing_state;
mod live_search;
mod loading;
mod loading_line;
mod location_view;
mod menu_popover;
mod mount_first;
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
mod quick_look;
mod relocated_files;
mod result_location;
mod saved_search;
mod search_box;
mod select_matching;
mod selection;
mod selection_keys;
mod session;
mod settings_tab;
mod sidebar;
mod sidebar_resizer;
mod snapshot_tabs;
mod status_bar;
mod tab_commands;
mod tab_layout;
mod tab_menu;
mod tab_moves;
mod tab_strip;
mod title_bar;
mod toast;
mod transfer_panel;
mod type_to_select;
mod unported;
mod version_restore;
mod view_zoom;
mod widget_tree;
mod window_action;
mod window_keys;
mod window_size;

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

pub(crate) use actions::follow_text_size_keys;
pub(crate) use actions::install_accelerators;
pub(crate) use button_style::ButtonStyle;
pub(crate) use closing::QUIT_WHILE_WRITING;
pub(crate) use disk_tools::is_installed as is_disk_tool_installed;
pub(crate) use folder_pane::FolderView;
pub(crate) use search_box::{show_bundled_clear_icon, show_bundled_magnifier};
pub(crate) use title_bar::list_open_windows_on_click;
pub(crate) use unported::Milestone;
pub(crate) use widget_tree::children;
pub(crate) use window_action::WindowAction;

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
        window.install_size_scans();
        window.install_item_dialogs();
        window.install_archive_actions();
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

    /// The tab in front as a tab action's target, for tests.
    #[cfg(test)]
    pub(crate) fn active_tab_target(&self) -> Option<glib::Variant> {
        let active = self.imp().session.borrow().active_id();
        active.map(session::TabId::to_variant)
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
