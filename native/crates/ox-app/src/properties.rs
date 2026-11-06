// SPDX-License-Identifier: AGPL-3.0-only
//! The Properties dialog, previous versions and on-demand folder sizes.
//!
//! Ports `propertiesDialog`, `renderVersionsPanel`, `renderSnapshotSource`,
//! `restoreVersion`, `renderSnapshotBanner` and the folder-size functions
//! (`scanFolderSizes`, `folderSizeText`, `updateSizeLabels`) of
//! `v2.0.0:desktop/ui/app.js`, over the ox-core services that port
//! `v2.0.0:desktop/file_services.py`, `v2.0.0:desktop/previous_versions.py` and
//! `v2.0.0:desktop/folder_sizes.py`.
//!
//! Each Properties is a top-level window of its own, as in Windows 11
//! ([`window`]): it never blocks the browser window or another Properties
//! window, and has OK, Cancel and Apply. What it asks of the browser window
//! (open a snapshot in a tab, restore a copy, Open with, rename) it asks
//! through that window's actions and methods, which stay reachable while
//! the browser window is open.
//!
//! | Module | Responsibility |
//! |---|---|
//! | `window` | [`PropertiesWindow`]: the window, its tabs and OK, Cancel and Apply |
//! | `item_window` | The Properties window of one item |
//! | `selection_view` | Properties of several items |
//! | `selection_attributes` | Read-only and Hidden of several items |
//! | `metadata` | Reading an item's properties off the main thread |
//! | `general_tab` | The General tab: name, type, sizes, dates and attributes |
//! | `drive_tab` | A drive's General and Hardware tabs |
//! | `drive_facts` | A drive's label, type and file system |
//! | `space_ring` | [`SpaceRing`](space_ring::SpaceRing): the ring of a drive's used and free space |
//! | `fixed_width` | [`FixedWidth`](fixed_width::FixedWidth): the window's content at Windows' width |
//! | `folder_count` | A folder's size and content, counting up |
//! | `security_tab` | The Security tab and its Edit and Advanced dialogs |
//! | `security_model` | Windows' permission names over Linux bits |
//! | `details_tab` | The Details tab of a file: its list, keys and reader |
//! | `details_rows` | The Details tab's headings, rows and value wording |
//! | `permissions_editor` | Changing several items' permissions together |
//! | `custom_icon` | The Customize tab: Change Icon... and Restore Default |
//! | `change_icon` | The Change Icon dialog: the theme's folder icons and Browse... |
//! | `location_panel` | [`LocationPanel`](location_panel::LocationPanel): the Location tab of a standard folder |
//! | `mount_assistant` | The network mount assistant of the Location tab |
//! | `sharing_panel` | The Sharing tab: Samba user shares of a local folder |
//! | `versions_panel` | [`VersionsPanel`]: the Previous Versions tab |
//! | `version_row` | One row of the versions list and its date heading |
//! | `snapshot_source` | The Snapshot source form |
//! | `restore` | The Restore a copy dialog |
//! | `snapshot_banner` | [`SnapshotBanner`]: the banner of a tab inside a snapshot |
//! | `folder_sizes` | [`FolderSizeState`] and [`FolderSizes`]: measured sizes and their text |
//! | `size_scan_strip` | [`SizeScanStrip`]: the bar of a running folder-size scan |

mod change_icon;
mod custom_icon;
mod details_rows;
mod details_tab;
mod drive_facts;
mod drive_tab;
mod fixed_width;
mod folder_count;
mod folder_sizes;
mod general_tab;
mod item_window;
mod location_panel;
mod metadata;
mod mount_assistant;
mod permissions_editor;
mod restore;
mod security_model;
mod security_tab;
mod selection_attributes;
mod selection_view;
mod sharing_panel;
mod size_scan_strip;
mod snapshot_banner;
mod snapshot_source;
mod space_ring;
mod version_row;
mod versions_panel;
mod window;

use ox_core::location::ItemKind;
use ox_core::places::KnownFolder;

/// Shown while the properties are read.
const READING: &str = crate::i18n::message_id("Reading file properties…");

#[cfg(test)]
pub(crate) use custom_icon::set_custom_icon;
pub(crate) use details_tab::focus_detail_row;
pub(crate) use folder_sizes::{size_key, FolderSizeState, FolderSizes, NOT_SCANNED};
pub(crate) use item_window::{open_item_properties, PropertiesContext};
#[cfg(test)]
pub(crate) use location_panel::LocationPanel;
pub(crate) use restore::RestoreRequest;
pub(crate) use selection_view::open_selection_properties;
pub(crate) use sharing_panel::system_usershares;
pub(crate) use size_scan_strip::{progress_text, RunEnd, RunPosition, SizeScanStrip};
pub(crate) use snapshot_banner::SnapshotBanner;
#[cfg(test)]
pub(crate) use space_ring::SpaceRing;
pub(crate) use version_row::SnapshotTarget;
#[cfg(test)]
pub(crate) use versions_panel::VersionsPanel;
pub(crate) use window::PropertiesWindow;

/// The item a Properties dialog describes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PropertiesTarget {
    /// The item's location: a share's or shortcut's target, else its own
    /// URI (`entry.targetUri || entry.uri`).
    pub uri: String,
    /// The title's name: a standard folder's label ("Downloads"), else
    /// the item's name.
    pub title: String,
    /// Whether the item is a folder, which decides the Size row, the
    /// folder-size button and where versions are looked for.
    pub kind: ItemKind,
    /// The standard folder the item is, which adds the Location tab.
    pub known_folder: Option<KnownFolder>,
}

impl PropertiesTarget {
    /// The dialog's title: `<name> Properties`.
    pub(crate) fn dialog_title(&self) -> String {
        ox_core::i18n::format_message("{title} Properties", &[("title", &self.title)])
    }
}

/// A tab of the Properties window, as the window opens on it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PropertiesTab {
    /// Name, type, location, sizes, dates and attributes.
    General,
    /// Where a drive is mounted and what is mounted there (drives only).
    Hardware,
    /// Sharing a local folder on the network (local folders only).
    Sharing,
    /// Owner, group and others, and what each may do.
    Security,
    /// Where a standard folder is stored (standard folders only).
    Location,
    /// What the file says about itself: its tags, picture, video or audio
    /// (files only).
    Details,
    /// Snapshots and backups of the item.
    PreviousVersions,
    /// The folder's icon (local folders only).
    Customize,
}

impl PropertiesTab {
    /// The tab's label, as Windows 11 names it.
    pub(crate) fn label(self) -> &'static str {
        match self {
            PropertiesTab::General => ox_core::i18n::gettext_static("General"),
            PropertiesTab::Hardware => ox_core::i18n::gettext_static("Hardware"),
            PropertiesTab::Sharing => ox_core::i18n::gettext_static("Sharing"),
            PropertiesTab::Security => ox_core::i18n::gettext_static("Security"),
            PropertiesTab::Location => ox_core::i18n::gettext_static("Location"),
            PropertiesTab::Details => ox_core::i18n::gettext_static("Details"),
            PropertiesTab::PreviousVersions => ox_core::i18n::gettext_static("Previous Versions"),
            PropertiesTab::Customize => ox_core::i18n::gettext_static("Customize"),
        }
    }

    /// The name of the tab's page in the window.
    pub(crate) const fn page_name(self) -> &'static str {
        match self {
            PropertiesTab::General => "general",
            PropertiesTab::Hardware => "hardware",
            PropertiesTab::Sharing => "sharing",
            PropertiesTab::Security => "security",
            PropertiesTab::Location => "location",
            PropertiesTab::Details => "details",
            PropertiesTab::PreviousVersions => "versions",
            PropertiesTab::Customize => "customize",
        }
    }
}
