// SPDX-License-Identifier: AGPL-3.0-only
//! Where a drop at one point of a zone goes (DND-011, DND-014, DND-016,
//! TAB-018).
//!
//! A folder view takes drops on a writable folder, on a program, or on
//! blank space for the folder shown; the sidebar on a place's folder, and
//! in Quick access to pin; the breadcrumbs and the tabs on their folders.
//! The Recycle Bin takes drops too, to move them to the Trash.
//! Nothing takes drops while a file operation runs.

use gtk::subclass::prelude::*;
use ox_core::location::{same_location, TRASH_URI};

use super::DropZone;
use crate::window::file_drag::is_draggable_location;
use crate::window::file_drop::DropDestination;
use crate::window::session::TabId;
use crate::window::sidebar::SidebarDropSpot;
use crate::window::BrowserWindow;

/// Where a drop at one point of a zone goes, and what shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum DropSpot {
    /// In a folder view: a folder or program under the pointer, at
    /// position `row`, or the folder shown when `row` is `None`.
    FolderView {
        destination: DropDestination,
        row: Option<u32>,
    },
    /// A sidebar row, or a place in Quick access.
    Sidebar(SidebarDropSpot),
    /// A crumb's folder.
    Crumb(String),
    /// A tab's folder.
    Tab { id: TabId, folder: String },
}

impl DropSpot {
    /// Where the drop goes.
    pub(super) fn destination(&self) -> DropDestination {
        match self {
            DropSpot::FolderView { destination, .. } => destination.clone(),
            DropSpot::Sidebar(SidebarDropSpot::Folder { uri, .. }) => {
                DropDestination::for_folder(uri.clone())
            }
            DropSpot::Sidebar(SidebarDropSpot::Pin { before, .. }) => DropDestination::QuickAccess {
                before: before.clone(),
            },
            DropSpot::Crumb(folder) | DropSpot::Tab { folder, .. } => {
                DropDestination::for_folder(folder.clone())
            }
        }
    }
}

impl BrowserWindow {
    /// Where a drop at (`x`, `y`) of `widget`, the window's `zone`, goes;
    /// `None` where nothing takes it, and anywhere while a file operation
    /// runs.
    pub(super) fn drop_spot(&self, zone: DropZone, widget: &gtk::Widget, x: f64, y: f64) -> Option<DropSpot> {
        if self.check_idle().is_err() {
            return None;
        }
        match zone {
            DropZone::FolderView => {
                let position = self.folder_pane().owners().position_at(widget, x, y);
                self.folder_view_spot(position)
            }
            DropZone::Sidebar => self.sidebar_spot(y),
            DropZone::Breadcrumbs => {
                let folder = self.address_bar().crumb_location_at(x, y)?;
                self.takes_drops(&folder).then_some(DropSpot::Crumb(folder))
            }
            DropZone::Tabs => {
                let tab = self.tab_strip().tab_at(x, y)?;
                self.takes_drops(&tab.uri).then_some(DropSpot::Tab {
                    id: tab.id,
                    folder: tab.uri,
                })
            }
        }
    }

    /// Where a drop on the folder view at `position`, or on blank space,
    /// goes: into a writable folder, to a program, or into the folder
    /// shown.
    pub(super) fn folder_view_spot(&self, position: Option<u32>) -> Option<DropSpot> {
        let under_pointer = position.and_then(|position| self.item_destination(position));
        if let Some(destination) = under_pointer {
            return Some(DropSpot::FolderView {
                destination,
                row: position,
            });
        }
        let shown = self.shown_folder_for_drops()?;
        Some(DropSpot::FolderView {
            destination: DropDestination::for_folder(shown),
            row: None,
        })
    }

    /// Where a drop on the item at `position` goes: into it when it is a
    /// folder that takes drops, to it when it is a program; `None` for any
    /// other item, whose drop goes into the folder shown.
    pub(super) fn item_destination(&self, position: u32) -> Option<DropDestination> {
        let item = self.folder_pane().model().item(position)?;
        let entry = item.entry();
        if entry.is_dir && !entry.is_virtual {
            let folder = entry.navigation_uri().to_owned();
            return self
                .takes_drops(&folder)
                .then_some(DropDestination::Folder(folder));
        }
        self.program_under_drag(entry).map(DropDestination::Program)
    }

    /// Where a drop on the folder view at `position` goes, for tests that
    /// drop without a pointer.
    #[cfg(test)]
    pub(in crate::window::file_drop) fn folder_view_destination(
        &self,
        position: Option<u32>,
    ) -> Option<DropDestination> {
        self.folder_view_spot(position).map(|spot| spot.destination())
    }

    /// The folder shown, when a drop on blank space may go into it: a
    /// writable folder that is listed without error and not searched.
    fn shown_folder_for_drops(&self) -> Option<String> {
        let shown = self.current_uri()?;
        let is_listed_cleanly = {
            let session = self.imp().session.borrow();
            session
                .active()
                .is_some_and(|tab| tab.error.is_none() && !tab.listing_state.is_listing())
        };
        let takes_drops = is_listed_cleanly && !self.is_searching() && self.takes_drops(&shown);
        takes_drops.then_some(shown)
    }

    /// Where a drop on the sidebar at `y` goes: a writable place's folder,
    /// or Quick access.
    pub(super) fn sidebar_spot(&self, y: f64) -> Option<DropSpot> {
        let spot = self.sidebar().drop_spot_at(y)?;
        if let SidebarDropSpot::Folder { uri, .. } = &spot {
            if !self.takes_drops(uri) {
                return None;
            }
        }
        Some(DropSpot::Sidebar(spot))
    }

    /// True for a folder dropped items may go into: a writable local or
    /// SMB folder, not a page, a server or a previous version; or the
    /// Recycle Bin, which moves them to the Trash (OPS-045).
    pub(super) fn takes_drops(&self, folder: &str) -> bool {
        let is_writable_folder =
            is_draggable_location(folder) && self.imp().locations.borrow().is_writable_location(folder);
        is_writable_folder || same_location(folder, TRASH_URI)
    }
}
