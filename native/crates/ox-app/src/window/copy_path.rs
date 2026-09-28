// SPDX-License-Identifier: AGPL-3.0-only
//! Copy path: puts the address of the selected item, or of the folder, on
//! the clipboard as text.
//!
//! Ports `copyPath` in `desktop/ui/app.js`. Only text is copied, so the
//! command changes no file and no sharing permission, and it needs none
//! of the file-operation workflows.

use gtk::prelude::*;
use gtk::subclass::prelude::*;

use crate::locations::Page;

use super::BrowserWindow;

/// What Copy path copies.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum CopiedPath {
    /// The display address of a folder or item.
    Address(String),
    /// A landing page has no path.
    NoFolder,
}

impl BrowserWindow {
    /// What Copy path would copy now: the single selected item, else the
    /// folder the tab shows.
    pub(super) fn path_to_copy(&self) -> CopiedPath {
        let selected = self.folder_pane().model().selected_items();
        let uri = match selected.as_slice() {
            [item] => Some(item.entry().uri.clone()),
            _ => self.current_uri(),
        };
        let Some(uri) = uri.filter(|uri| Page::from_uri(uri).is_none()) else {
            return CopiedPath::NoFolder;
        };
        let address = self.imp().locations.borrow().display_location(&uri);
        CopiedPath::Address(address)
    }

    /// Copies the path and says so, as app.js does.
    pub(super) fn copy_path(&self) {
        match self.path_to_copy() {
            CopiedPath::NoFolder => self.show_message("Open a folder first."),
            CopiedPath::Address(address) => {
                self.clipboard().set_text(&address);
                self.show_message("Path copied. Sharing permissions are unchanged.");
            }
        }
    }
}
