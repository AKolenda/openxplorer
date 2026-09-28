// SPDX-License-Identifier: AGPL-3.0-only
//! The Settings tab: opening Settings, showing it in place of the
//! browsing area, and leaving it.
//!
//! Ports `settingsDialog` in `desktop/ui/app.js` and the `.settings-open`
//! rules of `desktop/ui/style.css` (SET-001). Ctrl+,, the gear, More >
//! Settings and More > Default file explorer… open Settings as a tab of
//! its own, titled Settings; an open Settings tab is shown again rather
//! than a second one opened. The folder shown before is remembered
//! (`state.settingsOrigin`), so the search index offers it first. While
//! the Settings tab is in front, the Settings page replaces the navigation
//! row, the command bar, the workspace and the status bar, and address
//! editing is off. The status bar follows the settings mockup, which has
//! none; the Python page kept it (`style.css:130` hides only the sidebar,
//! the resizer, the details pane, the command bar and the navigation row).
//! "Back to files" closes the tab, as `closeTab` does; on the window's
//! only tab it opens that folder instead, so the window stays open.

use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;
use ox_core::places::Place;

use crate::locations::Page;
use crate::settings_page::{index_candidates, CandidateSources, Category, SettingsView};

use super::actions::plain_action;
use super::session::{TabId, TabPlacement};
use super::window_action::WindowAction;
use super::BrowserWindow;

/// What the window shows under the title bar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Surface {
    /// The navigation row, the command bar, the panes and the status bar.
    Browsing,
    /// The Settings page.
    Settings,
}

impl Surface {
    /// The surface for the tab showing `uri`.
    fn for_location(uri: &str) -> Self {
        if Page::from_uri(uri) == Some(Page::Settings) {
            Surface::Settings
        } else {
            Surface::Browsing
        }
    }

    /// Its name in the window's `surfaces` stack (`window.ui`).
    const fn name(self) -> &'static str {
        match self {
            Surface::Browsing => "browsing",
            Surface::Settings => "settings",
        }
    }
}

/// What the Settings tab keeps from the browsing tabs, for the folders it
/// offers the search index.
#[derive(Debug, Default)]
pub(super) struct SettingsTabState {
    /// The folder shown before Settings opened, which the search index
    /// offers first (`state.settingsOrigin` in app.js).
    origin: Option<String>,
    /// Quick access as the sidebar last drew it; Settings offers these
    /// folders to the search index.
    quick_access: Vec<Place>,
}

impl BrowserWindow {
    /// Binds the Settings page to the window's shared state and wires its
    /// window-wide parts: "Back to files" and its messages.
    pub(super) fn connect_settings_page(&self) {
        let page = self.settings_page();
        page.bind(self.context());
        page.connect_back_to_files(glib::clone!(
            #[weak(rename_to = window)]
            self,
            move || window.leave_settings()
        ));
        page.connect_message(glib::clone!(
            #[weak(rename_to = window)]
            self,
            move |message| window.show_message(message)
        ));
    }

    /// Settings (Ctrl+,), the Default file explorer… shortcut to it, and
    /// the layout reset it offers.
    pub(super) fn install_settings_actions(&self) {
        self.add_action_entries([
            plain_action(WindowAction::Settings, |window| window.open_settings(None)),
            plain_action(WindowAction::DefaultFileExplorer, |window| {
                window.open_settings(Some(SettingsView::Category(Category::DefaultApps)));
            }),
            plain_action(WindowAction::ResetLayout, BrowserWindow::reset_layout),
        ]);
    }

    /// Opens Settings, showing `view` when one is given and where it was
    /// left otherwise, with keyboard focus on its chosen category.
    pub(crate) fn open_settings(&self, view: Option<SettingsView>) {
        self.remember_settings_origin();
        self.show_settings_tab();
        self.settings_page().open(view);
        self.show_index_candidates();
    }

    /// Brings the Settings tab to the front, opening it when there is none.
    fn show_settings_tab(&self) {
        if let Some(id) = self.settings_tab() {
            self.switch_tab(id);
            return;
        }
        if let Err(error) = self.open_tab(Page::Settings.uri(), TabPlacement::Foreground) {
            self.show_message(&error.to_string());
        }
    }

    /// Types `query` into the settings search, as the snapshot hook asks.
    pub(crate) fn search_settings(&self, query: &str) {
        self.settings_page().search(query);
    }

    /// Remembers the location shown before Settings, unless that is
    /// Settings itself.
    fn remember_settings_origin(&self) {
        let current = self.current_uri();
        let origin = current.filter(|uri| Surface::for_location(uri) == Surface::Browsing);
        if origin.is_some() {
            self.imp().settings_tab.borrow_mut().origin = origin;
        }
    }

    /// The Settings tab, if one is open.
    fn settings_tab(&self) -> Option<TabId> {
        let session = self.imp().session.borrow();
        let settings = session
            .tabs()
            .iter()
            .find(|tab| Surface::for_location(tab.uri()) == Surface::Settings);
        settings.map(|tab| tab.id)
    }

    /// Shows the Settings page while the tab at `uri` is the Settings tab,
    /// and the browsing area otherwise.
    pub(super) fn show_surface_for(&self, uri: &str) {
        let surface = Surface::for_location(uri);
        let browsing = surface == Surface::Browsing;
        let leaves_settings = browsing && self.shows_settings();
        let imp = self.imp();
        imp.navigation_row.set_visible(browsing);
        self.command_bar().set_visible(browsing);
        self.status_bar().set_visible(browsing);
        imp.surfaces.set_visible_child_name(surface.name());
        // Address editing is inert on the Settings tab (SET-001).
        self.set_action_enabled(WindowAction::Location, browsing);
        if leaves_settings {
            self.focus_file_list_when_listed();
        }
    }

    /// Settings had keyboard focus, which GTK hands to the first sidebar
    /// row as the page hides; the file list takes it back once it is
    /// listed, as in a new window. A tab draws its location before its
    /// content, so the list is looked for once the tab is shown.
    fn focus_file_list_when_listed(&self) {
        self.imp().file_list_awaits_focus.set(true);
        glib::idle_add_local_once(glib::clone!(
            #[weak(rename_to = window)]
            self,
            move || window.focus_new_file_list()
        ));
    }

    /// Whether the Settings page is shown.
    pub(super) fn shows_settings(&self) -> bool {
        self.imp().surfaces.visible_child_name().as_deref() == Some(Surface::Settings.name())
    }

    /// "Back to files": closes the Settings tab, or on the window's only
    /// tab opens the folder shown before Settings, else the home folder.
    fn leave_settings(&self) {
        let Some(id) = self.settings_tab() else {
            return;
        };
        if self.tab_count() > 1 {
            self.close_tab(id);
            return;
        }
        let origin = self.imp().settings_tab.borrow().origin.clone();
        let folder = origin.unwrap_or_else(|| self.imp().locations.borrow().home_uri());
        self.navigate_or_report(&folder);
    }

    /// Keeps `quick_access`, as the sidebar now shows it, for the folders
    /// Settings offers the search index, and offers them again.
    pub(super) fn update_index_candidates(&self, quick_access: &[Place]) {
        self.imp().settings_tab.borrow_mut().quick_access = quick_access.to_vec();
        self.show_index_candidates();
    }

    /// Offers the search index the folder shown before Settings, Quick
    /// access as the sidebar last drew it, the saved shares and the drives.
    fn show_index_candidates(&self) {
        let imp = self.imp();
        let state = imp.settings_tab.borrow();
        let shares = self.context().settings_data().shares;
        let volumes = imp.volumes.borrow();
        let locations = imp.locations.borrow();
        let sources = CandidateSources {
            origin: state.origin.as_deref(),
            quick_access: &state.quick_access,
            shares: &shares,
            volumes: &volumes,
            locations: &locations,
        };
        self.settings_page()
            .show_index_candidates(&index_candidates(&sources));
    }
}
