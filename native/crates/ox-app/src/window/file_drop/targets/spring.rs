// SPDX-License-Identifier: AGPL-3.0-only
//! Folders that open while a drag hovers over them (DND-021).
//!
//! As in Dolphin with "Open folders during drag operations", a drag that
//! stays over a folder in the folder view for [`FOLDER_HOVER_DELAY`] opens
//! that folder in the tab, so the drop can go into a deeper folder. Moving
//! to another spot starts the wait again; leaving or dropping stops it.
//! A place in the sidebar never opens under a drag, as in Windows
//! Explorer's navigation pane, which only expands a collapsed node
//! (`Sidebar::expand_after_drag_hover`, DND-027).

use std::time::Duration;

use gtk::glib;
use gtk::subclass::prelude::*;

use super::spot::DropSpot;
use crate::window::file_drop::DropDestination;
use crate::window::BrowserWindow;

/// How long a drag must stay over a folder before it opens (Dolphin's
/// 750 ms).
const FOLDER_HOVER_DELAY: Duration = Duration::from_millis(750);

/// The folder that opens when a drag stays over `spot`: a folder item
/// under the pointer; `None` for the folder shown, a program, the
/// sidebar, a crumb or a tab.
pub(super) fn folder_to_open(spot: &DropSpot) -> Option<&str> {
    match spot {
        DropSpot::FolderView {
            destination: DropDestination::Folder(folder),
            row: Some(_),
        } => Some(folder),
        _ => None,
    }
}

impl BrowserWindow {
    /// Opens `folder` once a drag has stayed over it for
    /// [`FOLDER_HOVER_DELAY`]; another folder, or `None`, stops the wait.
    pub(super) fn open_folder_after_hover(&self, folder: Option<&str>) {
        let folder = folder.filter(|folder| self.current_uri().as_deref() != Some(*folder));
        let waiting = self
            .imp()
            .folder_hover
            .borrow()
            .as_ref()
            .map(|(waiting, _)| waiting.clone());
        if waiting.as_deref() == folder {
            return;
        }
        if let Some((_, timer)) = self.imp().folder_hover.take() {
            timer.remove();
        }
        let Some(folder) = folder.map(str::to_owned) else {
            return;
        };
        let timer = glib::timeout_add_local_once(
            FOLDER_HOVER_DELAY,
            glib::clone!(
                #[weak(rename_to = window)]
                self,
                #[strong]
                folder,
                move || {
                    window.imp().folder_hover.replace(None);
                    window.navigate_or_report(&folder);
                }
            ),
        );
        self.imp().folder_hover.replace(Some((folder, timer)));
    }
}
