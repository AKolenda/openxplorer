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
//! editing is off. "Back to files" closes the tab, as `closeTab` does; on
//! the window's only tab it opens that folder instead, so the window stays
//! open.

use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;

use crate::locations::Page;
use crate::places::Places;
use crate::settings_page::{index_candidates, CandidateSources, SettingsView};

use super::session::{TabId, TabPlacement};
use super::title_bar::list_open_windows_on_click;
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

impl BrowserWindow {
    /// Binds the Settings page to the window's shared state and wires its
    /// window-wide parts: the open-windows menu, "Back to files" and its
    /// messages.
    pub(super) fn connect_settings_page(&self) {
        let page = self.settings_page();
        page.bind(self.context());
        list_open_windows_on_click(&page.open_windows_button());
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

    /// Opens Settings, showing `view` when one is given and where it was
    /// left otherwise.
    pub(crate) fn open_settings(&self, view: Option<SettingsView>) {
        self.remember_settings_origin();
        match self.settings_tab() {
            Some(id) => self.switch_tab(id),
            None => {
                if let Err(error) = self.open_tab(Page::Settings.uri(), TabPlacement::Foreground) {
                    self.show_message(error.message());
                }
            }
        }
        let page = self.settings_page();
        if let Some(view) = view {
            page.show_view(view);
        }
        page.refresh();
        self.show_index_candidates(&self.places());
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
            self.imp().settings_origin.replace(origin);
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
        let imp = self.imp();
        imp.navigation_row.set_visible(browsing);
        self.command_bar().set_visible(browsing);
        self.status_bar().set_visible(browsing);
        imp.surfaces.set_visible_child_name(surface.name());
        // Address editing is inert on the Settings tab (SET-001).
        self.set_action_enabled(WindowAction::Location, browsing);
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
        let origin = self.imp().settings_origin.borrow().clone();
        let folder = origin.unwrap_or_else(|| self.imp().locations.borrow().home_uri());
        self.navigate_or_report(&folder);
    }

    /// Offers the search index the folder shown before Settings and the
    /// places in `places`.
    pub(super) fn show_index_candidates(&self, places: &Places) {
        let imp = self.imp();
        let origin = imp.settings_origin.borrow().clone();
        let shares = self.context().settings_data().shares;
        let volumes = imp.volumes.borrow();
        let locations = imp.locations.borrow();
        let sources = CandidateSources {
            origin: origin.as_deref(),
            quick_access: &places.quick_access,
            shares: &shares,
            volumes: &volumes,
            locations: &locations,
        };
        self.settings_page()
            .show_index_candidates(&index_candidates(&sources));
    }
}
