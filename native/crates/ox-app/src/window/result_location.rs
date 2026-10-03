// SPDX-License-Identifier: AGPL-3.0-only
//! "Open file location" on a search result: its folder opens in this tab,
//! a new tab or a new window, with the result selected (SRCH-015,
//! SRCH-016).

use gtk::prelude::*;
use gtk::subclass::prelude::*;

use ox_core::location::{parent_location, LocationError};

use super::session::{TabId, TabPlacement};
use super::BrowserWindow;

/// Where "Open file location" opens a search result's folder.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum LocationTarget {
    /// In the tab that shows the search, which leaves the search.
    ThisTab,
    /// In a new tab behind it (SRCH-016).
    NewTab,
    /// In a new window (SRCH-016).
    NewWindow,
}

impl BrowserWindow {
    /// "Open file location" on a search result: opens the folder it is in
    /// where `target` says, with the result selected and scrolled into
    /// view (SRCH-015, SRCH-016). A new tab opens behind, so the search
    /// stays in front, as Dolphin opens it.
    pub(super) fn open_result_location(&self, target: LocationTarget) {
        let items = self.folder_pane().model().selected_items();
        let [item] = items.as_slice() else {
            return;
        };
        self.open_item_location(item.entry().uri.clone(), target);
    }

    /// Opens the folder that holds `uri` where `target` says, with `uri`
    /// selected and scrolled into view: a search result's location, or
    /// a link's target (CMD-030).
    pub(super) fn open_item_location(&self, uri: String, target: LocationTarget) {
        let Some(folder) = parent_location(&uri) else {
            return;
        };
        let opened = match target {
            LocationTarget::ThisTab => self.navigate(&folder).map(|()| self.locate_in_active_tab(uri)),
            LocationTarget::NewTab => self.open_located_tab(&folder, uri),
            LocationTarget::NewWindow => self.open_located_window(&folder, uri),
        };
        if let Err(error) = opened {
            self.show_message(&error.to_string());
        }
    }

    /// Selects `uri` in the active tab once it has listed its folder.
    fn locate_in_active_tab(&self, uri: String) {
        if let Some(tab) = self.imp().session.borrow_mut().active_mut() {
            tab.selected = vec![uri.clone()];
            tab.revealed_item = Some(uri);
        }
    }

    /// Opens `folder` in a tab behind the current one, where `uri` is
    /// selected when the tab is first shown.
    fn open_located_tab(&self, folder: &str, uri: String) -> Result<(), LocationError> {
        let folder = self.resolve_address(folder)?;
        self.save_tab_view();
        let position = self.opened_tab_position();
        let mut session = self.imp().session.borrow_mut();
        let id = session.add_at(&folder, TabPlacement::Background, position);
        if let Some(tab) = session.tab_mut(id) {
            tab.selected = vec![uri.clone()];
            tab.revealed_item = Some(uri);
        }
        drop(session);
        self.render_tabs();
        Ok(())
    }

    /// Opens `folder` in a new window, with `uri` selected there.
    fn open_located_window(&self, folder: &str, uri: String) -> Result<(), LocationError> {
        let Some(app) = self.application() else {
            return Ok(());
        };
        let window = BrowserWindow::new(&app, self.context());
        if let Err(error) = window.add_tab(folder) {
            window.destroy();
            return Err(error);
        }
        window.locate_in_active_tab(uri);
        window.present();
        Ok(())
    }

    /// Scrolls tab `id`'s item that "Open file location" asked for into
    /// view, once the tab has listed its folder.
    pub(super) fn reveal_located_item(&self, id: TabId) {
        let revealed = {
            let mut session = self.imp().session.borrow_mut();
            session.tab_mut(id).and_then(|tab| tab.revealed_item.take())
        };
        let Some(uri) = revealed else {
            return;
        };
        let model = self.folder_pane().model();
        let position = (0..model.n_items()).find(|position| {
            let item = model.item(*position);
            item.is_some_and(|item| item.entry().uri == uri)
        });
        if let Some(position) = position {
            self.folder_pane().reveal(position);
        }
    }
}
