// SPDX-License-Identifier: AGPL-3.0-only
//! The Properties dialog, previous versions and on-demand folder sizes.
//!
//! Ports `propertiesDialog`, `renderVersionsPanel`, `renderSnapshotSource`,
//! `restoreVersion`, `renderSnapshotBanner` and the folder-size functions
//! (`scanFolderSizes`, `folderSizeText`, `updateSizeLabels`) of
//! `desktop/ui/app.js`, over the ox-core services that port
//! `desktop/file_services.py`, `desktop/previous_versions.py` and
//! `desktop/folder_sizes.py`.
//!
//! The dialog belongs to the tab that opened it (PROP-008): the window
//! shows it on an in-window [`DialogLayer`](crate::dialog_layer::DialogLayer),
//! withdraws it when another tab comes to the front and shows it again,
//! as it was, when its tab returns. What the dialog asks of the window
//! (open a snapshot in a tab, restore a copy, measure a folder) it asks
//! through window actions, so it never reaches into the window.
//!
//! | Module | Responsibility |
//! |---|---|
//! | `view` | [`PropertiesView`]: the tabs and their panels |
//! | `selection_view` | [`SelectionProperties`]: Properties of several items |
//! | `metadata` | Reading an item's properties off the main thread |
//! | `general_panel` | The General and Permissions tabs |
//! | `permissions_editor` | Changing permissions on the Permissions tab |
//! | `checksums_panel` | The Checksums tab of a file |
//! | `location_panel` | [`LocationPanel`](location_panel::LocationPanel): the Location tab of a standard folder |
//! | `mount_assistant` | The network mount assistant of the Location tab |
//! | `versions_panel` | [`VersionsPanel`]: the Previous versions tab |
//! | `version_row` | One row of the versions list |
//! | `snapshot_source` | The Snapshot source form |
//! | `restore` | The Restore a copy dialog |
//! | `snapshot_banner` | [`SnapshotBanner`]: the banner of a tab inside a snapshot |
//! | `folder_sizes` | [`FolderSizeState`] and [`FolderSizes`]: measured sizes and their text |
//! | `size_scan_strip` | [`SizeScanStrip`]: the bar of a running folder-size scan |

mod checksums_panel;
mod folder_sizes;
mod general_panel;
mod location_panel;
mod metadata;
mod mount_assistant;
mod permissions_editor;
mod restore;
mod selection_view;
mod size_scan_strip;
mod snapshot_banner;
mod snapshot_source;
mod version_row;
mod versions_panel;
mod view;

use ox_core::location::ItemKind;
use ox_core::places::KnownFolder;

pub(crate) use folder_sizes::{size_key, FolderSizeState, FolderSizes, NOT_SCANNED};
#[cfg(test)]
pub(crate) use location_panel::LocationPanel;
pub(crate) use restore::RestoreRequest;
pub(crate) use selection_view::SelectionProperties;
pub(crate) use size_scan_strip::{progress_text, RunEnd, RunPosition, SizeScanStrip};
pub(crate) use snapshot_banner::SnapshotBanner;
pub(crate) use version_row::SnapshotTarget;
pub(crate) use view::{PropertiesContext, PropertiesView};

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
        format!("{} Properties", self.title)
    }
}

/// A tab of the Properties dialog, as the dialog opens on it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PropertiesTab {
    /// Type, location, size, dates and the default application.
    General,
    /// Where a standard folder is stored (standard folders only).
    Location,
    /// Owner, group, mode and access.
    Permissions,
    /// MD5, SHA1, SHA256 and SHA512 of a file (files only).
    Checksums,
    /// Snapshots and backups of the item.
    PreviousVersions,
}

impl PropertiesTab {
    /// The tab's label.
    pub(crate) const fn label(self) -> &'static str {
        match self {
            PropertiesTab::General => "General",
            PropertiesTab::Location => "Location",
            PropertiesTab::Permissions => "Permissions",
            PropertiesTab::Checksums => "Checksums",
            PropertiesTab::PreviousVersions => "Previous versions",
        }
    }

    /// The name of the tab's page in the dialog's stack.
    pub(crate) const fn page_name(self) -> &'static str {
        match self {
            PropertiesTab::General => "general",
            PropertiesTab::Location => "location",
            PropertiesTab::Permissions => "permissions",
            PropertiesTab::Checksums => "checksums",
            PropertiesTab::PreviousVersions => "versions",
        }
    }

    /// The tab whose page is named `name`.
    pub(crate) fn from_page_name(name: &str) -> Option<Self> {
        [
            PropertiesTab::General,
            PropertiesTab::Location,
            PropertiesTab::Permissions,
            PropertiesTab::Checksums,
            PropertiesTab::PreviousVersions,
        ]
        .into_iter()
        .find(|tab| tab.page_name() == name)
    }
}
