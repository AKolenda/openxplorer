// SPDX-License-Identifier: AGPL-3.0-only
//! Where a drop on the sidebar goes, and the highlight that shows it
//! (DND-009, DND-014).
//!
//! Ports the sidebar half of `publishFileDragLayout` and the pin half of
//! `showFileDropHint` in `desktop/ui/app.js`. Quick access always pins,
//! also over a pinned folder: before the pin whose vertical middle is
//! below the pointer, with an accent line above it, or after the last pin,
//! with the line under it. Any other row that opens a location is that
//! folder; the window decides whether the folder takes drops. A drive
//! that still has to be mounted takes none.

use gtk::prelude::*;
use gtk::subclass::prelude::*;
use ox_core::location::RECENT_URI;

use super::entries::{RowTarget, Section};
use super::Sidebar;

/// The CSS class of the row whose folder a drop would go into.
const FOLDER_DROP_CLASS: &str = "file-drop-active";
/// The CSS class of the pin a drop would go before.
const PIN_BEFORE_CLASS: &str = "drop-before";
/// The CSS class of the last pin, when a drop would go after it.
const PIN_END_CLASS: &str = "drop-end";

/// Where a drop on the sidebar goes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::window) enum SidebarDropSpot {
    /// Into the folder of the row at `index`.
    Folder {
        /// The row.
        index: i32,
        /// Its folder.
        uri: String,
    },
    /// Onto the drive of the row at `index`, which is mounted first
    /// (DEV-010).
    Volume {
        /// The row.
        index: i32,
        /// The volume to mount.
        id: String,
    },
    /// Pinned to Quick access before the pin at `before`, whose row is at
    /// `index`, or after the last pin, at `index`, when `before` is
    /// `None`.
    Pin {
        /// The row the accent line is drawn at.
        index: i32,
        /// The pin the new pins go before.
        before: Option<String>,
    },
}

impl SidebarDropSpot {
    /// The row the highlight is drawn on, and its class.
    fn highlight(&self) -> (i32, &'static str) {
        match self {
            SidebarDropSpot::Folder { index, .. } | SidebarDropSpot::Volume { index, .. } => {
                (*index, FOLDER_DROP_CLASS)
            }
            SidebarDropSpot::Pin {
                index,
                before: Some(_),
            } => (*index, PIN_BEFORE_CLASS),
            SidebarDropSpot::Pin { index, before: None } => (*index, PIN_END_CLASS),
        }
    }
}

impl Sidebar {
    /// Where a drop at `y` of the list goes; `None` between sections and
    /// on a drive that is not mounted.
    pub(in crate::window) fn drop_spot_at(&self, y: f64) -> Option<SidebarDropSpot> {
        #[expect(clippy::cast_possible_truncation, reason = "pointer positions are small")]
        let row = self.list().row_at_y(y as i32)?;
        let index = row.index();
        let entries = self.imp().entries.borrow();
        let entry = entries.get(usize::try_from(index).ok()?)?;
        let uri = match &entry.target {
            RowTarget::Location(uri) => uri,
            RowTarget::PinDropTail => return Some(SidebarDropSpot::Pin { index, before: None }),
            // A drive still to be mounted takes the drop and is mounted
            // first, as Dolphin's places panel does (DEV-010).
            RowTarget::MountVolume(id) if entry.section != Section::QuickAccess => {
                return Some(SidebarDropSpot::Volume {
                    index,
                    id: id.clone(),
                });
            }
            RowTarget::MountVolume(_) | RowTarget::SavedSearch(_) => return None,
        };
        // Recent files lists what the desktop recorded; nothing goes in.
        if uri == RECENT_URI {
            return None;
        }
        if entry.section != Section::QuickAccess {
            return Some(SidebarDropSpot::Folder {
                index,
                uri: uri.clone(),
            });
        }
        let bounds = row.compute_bounds(self.list())?;
        let middle = f64::from(bounds.y() + bounds.height() / 2.0);
        if y < middle {
            return Some(SidebarDropSpot::Pin {
                index,
                before: Some(uri.clone()),
            });
        }
        let next = self.quick_access_pin(index + 1);
        Some(match next {
            Some(before) => SidebarDropSpot::Pin {
                index: index + 1,
                before: Some(before),
            },
            None => SidebarDropSpot::Pin { index, before: None },
        })
    }

    /// The location of the Quick access row at `index`, if it is one.
    fn quick_access_pin(&self, index: i32) -> Option<String> {
        let entries = self.imp().entries.borrow();
        let entry = entries.get(usize::try_from(index).ok()?)?;
        match (&entry.target, entry.section) {
            (RowTarget::Location(uri), Section::QuickAccess) => Some(uri.clone()),
            _ => None,
        }
    }

    /// Highlights `spot`, or nothing.
    pub(in crate::window) fn show_drop_spot(&self, spot: Option<&SidebarDropSpot>) {
        let list = self.list();
        let mut index = 0;
        while let Some(row) = list.row_at_index(index) {
            for class in [FOLDER_DROP_CLASS, PIN_BEFORE_CLASS, PIN_END_CLASS] {
                row.remove_css_class(class);
            }
            index += 1;
        }
        let Some((index, class)) = spot.map(SidebarDropSpot::highlight) else {
            return;
        };
        if let Some(row) = list.row_at_index(index) {
            row.add_css_class(class);
        }
    }

    /// The drop highlight's class on the row labelled `label`, for tests.
    #[cfg(test)]
    pub(in crate::window) fn drop_highlight_of(&self, label: &str) -> Option<&'static str> {
        let index = self.labels().iter().position(|shown| shown == label)?;
        let row = self.list().row_at_index(i32::try_from(index).ok()?)?;
        [FOLDER_DROP_CLASS, PIN_BEFORE_CLASS, PIN_END_CLASS]
            .into_iter()
            .find(|class| row.has_css_class(class))
    }

    /// The vertical middle of the row labelled `label`, in the list's
    /// coordinates, for tests.
    #[cfg(test)]
    pub(in crate::window) fn middle_of(&self, label: &str) -> f64 {
        let index = self
            .labels()
            .iter()
            .position(|shown| shown == label)
            .unwrap_or_else(|| panic!("the sidebar shows {label}"));
        let row = i32::try_from(index)
            .ok()
            .and_then(|index| self.list().row_at_index(index))
            .expect("every entry has a row");
        let bounds = row.compute_bounds(self.list()).expect("a shown row has bounds");
        f64::from(bounds.y() + bounds.height() / 2.0)
    }
}
