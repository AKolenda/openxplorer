// SPDX-License-Identifier: AGPL-3.0-only
//! A drive copied and pasted, or dragged, into a folder becomes a shortcut
//! to it, never a copy of everything on it.
//!
//! Windows Explorer lets a drive be copied (Ctrl+C, its menu's Copy, the
//! command bar's Copy with a drive selected in This PC) but not cut, and a
//! paste of it into a folder makes a shortcut named after the drive as
//! This PC shows it, "Local Disk (C:) - Shortcut", then "Local Disk (C:) -
//! Shortcut (2)". Dragging a drive into a folder makes one too: a drive's
//! drop effect is a link ("Create link in Documents"). This app shows no
//! drive letters, so Local Disk's shortcut is "Local Disk - Shortcut".
//!
//! Safety rule "a drive is never copied whole": a paste or a drop checks
//! every item against the drives This PC lists (Local Disk and each
//! mounted drive or device), wherever the item came from, and makes a
//! shortcut to each drive it finds, through the app's own shortcuts
//! ([`ox_core::ops::create_named_shortcuts`], symbolic links named as
//! Windows names them). Only the other items are copied or moved.

use ox_core::location::same_location;
use ox_core::ops::{NamedShortcuts, ShortcutTarget};

use crate::locations::Page;
use crate::volumes::VolumeState;

use super::BrowserWindow;

/// Local Disk's location: the root of the file system.
const LOCAL_DISK: &str = "file:///";

/// Items taken apart into drives and the rest: each drive
/// with its name, and the other items, both in their order.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(super) struct SplitItems {
    /// The drives, which get shortcuts.
    pub(super) drives: Vec<ShortcutTarget>,
    /// Everything else, copied or moved as usual.
    pub(super) others: Vec<String>,
}

/// `uris` taken apart into drives and other items: an item is a drive
/// when `drive_name` names it.
pub(super) fn split_items(uris: &[String], drive_name: impl Fn(&str) -> Option<String>) -> SplitItems {
    let mut split = SplitItems::default();
    for uri in uris {
        match drive_name(uri) {
            Some(name) => split.drives.push(ShortcutTarget {
                uri: uri.clone(),
                name: Some(name),
            }),
            None => split.others.push(uri.clone()),
        }
    }
    split
}

impl BrowserWindow {
    /// `uris` taken apart into the drives This PC lists, each with the
    /// name This PC shows (Local Disk, or a mounted drive's or device's
    /// name), and other items, folders inside a drive included.
    pub(super) fn split_drives(&self, uris: &[String]) -> SplitItems {
        // The drives are read once, not once per item.
        let places = self.places();
        let local_disk = ox_core::i18n::gettext("Local Disk");
        split_items(uris, |uri| {
            if same_location(uri, LOCAL_DISK) {
                return Some(local_disk.clone());
            }
            places.drives.iter().find_map(|row| match &row.state {
                VolumeState::Mounted { uri: root, .. } if same_location(root, uri) => Some(row.label.clone()),
                _ => None,
            })
        })
    }

    /// True when every item of `uris` is a drive This PC lists, so a drop
    /// of them makes shortcuts ("Create link in …").
    pub(super) fn are_all_drives(&self, uris: &[String]) -> bool {
        !uris.is_empty() && self.split_drives(uris).others.is_empty()
    }

    /// The drive selected on This PC, when the tab shows This PC and a
    /// drive's tile is selected: what Copy copies there.
    pub(super) fn selected_drive_tile(&self) -> Option<String> {
        let uri = self.current_uri()?;
        if Page::from_uri(&uri) != Some(Page::ThisPc) {
            return None;
        }
        super::landing::selected_drive(self.folder_pane().landing())
    }

    /// Makes a shortcut in `folder` to each of `drives`, named as Windows
    /// names a drive's shortcut, and selects the new shortcuts; Undo moves
    /// them to the Trash. Does nothing while another operation runs.
    pub(super) async fn make_drive_shortcuts(&self, drives: Vec<ShortcutTarget>, folder: String) {
        if drives.is_empty() {
            return;
        }
        let request = NamedShortcuts {
            targets: drives,
            destination_folder: folder,
        };
        self.run_named_shortcuts(&request).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::harness::{Fixture, TestWindow};

    fn names(uri: &str) -> Option<String> {
        match uri {
            "file:///" => Some("Local Disk".to_owned()),
            "file:///media/alex/Holiday%20Stick" => Some("Holiday Stick".to_owned()),
            _ => None,
        }
    }

    /// Drives are taken apart from the other items, each named, and the
    /// order of both is kept; a folder inside a drive is no drive.
    #[test]
    fn drives_are_taken_apart_from_other_items() {
        let uris = [
            "file:///home/alex/Projects",
            "file:///",
            "file:///media/alex/Holiday%20Stick/Photos",
            "file:///media/alex/Holiday%20Stick",
        ]
        .map(ToOwned::to_owned);

        let split = split_items(&uris, names);

        assert_eq!(
            split.drives,
            [
                ShortcutTarget {
                    uri: "file:///".to_owned(),
                    name: Some("Local Disk".to_owned()),
                },
                ShortcutTarget {
                    uri: "file:///media/alex/Holiday%20Stick".to_owned(),
                    name: Some("Holiday Stick".to_owned()),
                },
            ]
        );
        assert_eq!(
            split.others,
            [
                "file:///home/alex/Projects",
                "file:///media/alex/Holiday%20Stick/Photos"
            ]
        );
    }

    /// Local Disk is a drive wherever its address comes from; a folder
    /// is not.
    #[gtk::test]
    fn local_disk_is_a_drive_and_a_folder_is_not() {
        let fixture = Fixture::standard();
        let test = TestWindow::open(&fixture.uri());

        let split = test.window.split_drives(&["file:///".to_owned(), fixture.uri()]);
        assert_eq!(
            split.drives,
            [ShortcutTarget {
                uri: "file:///".to_owned(),
                name: Some("Local Disk".to_owned()),
            }]
        );
        assert_eq!(split.others, [fixture.uri()]);
        assert!(test.window.are_all_drives(&["file:///".to_owned()]));
        assert!(!test
            .window
            .are_all_drives(&["file:///".to_owned(), fixture.uri()]));
        assert!(!test.window.are_all_drives(&[]));
    }
}
