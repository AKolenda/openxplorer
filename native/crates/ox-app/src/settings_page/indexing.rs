// SPDX-License-Identifier: AGPL-3.0-only
//! Search & indexing: the folders to index, how the index keeps current,
//! and folder sizes.
//!
//! Ports the "Search cache" and "Folder sizes" sections of
//! `renderSettingsPage` in `desktop/ui/app.js` (SET-006, SET-008,
//! SET-009), with the options of the settings mockup, including "Index
//! pinned folders automatically" (the owner's decision of 2026-09-28 that
//! anything pinned is indexed by default). The folder list opens as a page
//! of its own ([`super::indexed_folders`]), with the limits of indexing,
//! and so do the details of folder sizes. The page itself keeps one note,
//! on privacy, as the mockup does. Cached search is not in the native
//! preview yet: its actions wait for it, while the two options the Python
//! app already reads (`autoIndex`, `networkInterval`) are saved for it
//! now.

use gtk::glib;
use gtk::prelude::*;
use ox_core::settings::PreferencesUpdate;

use super::bindings::{Choice, PreferenceBinding};
use super::group::SettingsGroup;
use super::pages::{Category, SettingsView, Subpage};
use super::parts;
use super::row::{Availability, ControlName, SettingRow};
use super::search::RowText;
use super::section::{PageKind, SettingsSection};
use super::status_card::{StatusCard, StatusText};
use super::SettingsPage;
use crate::icons::Icon;
use crate::window::Milestone;

const FOLDERS_TO_INDEX: RowText = RowText {
    title: "Folders to index",
    description: "Check a folder to index the names and paths of its files and subfolders. SMB \
                  folders work too.",
    keywords: "search cache add custom folder directory path local disk smb nas entire drive 2tb \
               indexing refresh clear",
};

const PINNED_FOLDERS: RowText = RowText {
    title: "Index pinned folders automatically",
    description: "Pinning a folder adds it to instant search. Unpinning removes it unless you \
                  added it yourself.",
    keywords: "quick access pin search cache",
};

const WATCH_FOLDERS: RowText = RowText {
    title: "Watch folders for live changes",
    description: "Watch enabled local folders for changes while OpenXplorer is open.",
    keywords: "inotify automatic file updates index",
};

const NETWORK_CHECKS: RowText = RowText {
    title: "Network / fallback checks",
    description: "SMB and unwatched folders are checked for changes this often.",
    keywords: "network refresh interval smb polling nas seconds minute. SMB and unwatched folders \
               use incremental directory checks, not push notifications. Large trees take longer \
               than one interval.",
};

const FOLDER_SIZES: RowText = RowText {
    title: "Calculate folder sizes",
    description: "Right-click a folder → Calculate folder size. Results last for this session.",
    keywords: "zfs logical bytes snapshots disk usage. Results are kept only for this window \
               session.",
};

const HOW_SIZES_ARE_COUNTED: RowText = RowText {
    title: "How sizes are counted",
    description: "Logical file bytes, what a scan skips, and its limits.",
    keywords: "folder sizes scans run on demand outside the browsing worker pool compressed zfs \
               space snapshot usage hidden files links nested filesystem mounts snapshot \
               collections 1 million entries 5 minutes partial total cancel recalculate",
};

/// How often network and unwatched folders are checked, as the Python
/// select offers it.
const NETWORK_INTERVALS: [Choice<u32>; 3] = [
    Choice {
        value: 30,
        label: "30 seconds",
    },
    Choice {
        value: 60,
        label: "1 minute",
    },
    Choice {
        value: 300,
        label: "5 minutes",
    },
];

/// The privacy note of the Python section.
const PRIVACY_NOTE: &str = "Cached paths are stored locally and can be searched while a share is \
                            offline. Uncheck a root to stop caching and remove that root's names. \
                            Other overlapping roots may still contain them. Hidden folders and \
                            symbolic links are skipped.";

/// The two paragraphs of the Python "Folder sizes" section.
const FOLDER_SIZES_HELP: [&str; 2] = [
    "Right-click a folder → Calculate folder size. Scans run on demand, outside the browsing worker \
     pool. Results are logical file bytes, not compressed ZFS space or snapshot usage. They are \
     kept only for this window session. Recalculate to pick up later changes.",
    "Scans include hidden files, but skip links, nested filesystem mounts and snapshot \
     collections. Each folder is limited to 1 million entries or 5 minutes between I/O calls. \
     Inaccessible or excluded entries produce a partial total. Cancel stops the active scan and \
     any queued folders.",
];

/// The Search & indexing page.
pub(super) fn build(page: &SettingsPage) -> SettingsSection {
    let category = Category::SearchAndIndexing;
    let indexing = SettingsSection::new(category.title(), category.lead(), PageKind::Category);
    indexing.append_card(&status_card());
    indexing.append_group(&folders_group(page));
    indexing.append_group(&options_group(page));
    indexing.append_text(&parts::note(Icon::ShieldLock, PRIVACY_NOTE));
    indexing.append_group(&folder_sizes_group(page));
    indexing
}

/// Where instant search stands, with "Refresh all" waiting for it.
fn status_card() -> StatusCard {
    let refresh = parts::button_with_glyph("Refresh all", Icon::ArrowClockwise);
    let pending = Milestone::SearchAndMetadata.notice();
    refresh.set_sensitive(false);
    refresh.set_tooltip_text(Some(&pending));
    let status = StatusText {
        glyph: Icon::Search,
        title: "Instant search",
        text: "Search file names in the folders you choose, even while a share is offline.",
        notice: Some(&pending),
    };
    StatusCard::new(status, &[refresh.upcast()])
}

/// "Folders to index", which opens the Indexed folders page.
fn folders_group(page: &SettingsPage) -> SettingsGroup {
    let group = SettingsGroup::new("Indexed folders");
    let row = SettingRow::new(FOLDERS_TO_INDEX);
    let manage = parts::page_button("Manage…");
    manage.connect_clicked(glib::clone!(
        #[weak]
        page,
        move |_| page.show_view(SettingsView::Subpage(Subpage::IndexedFolders))
    ));
    row.add_control(&manage, ControlName::OwnLabel);
    group.add_row(&row);
    group
}

fn options_group(page: &SettingsPage) -> SettingsGroup {
    let group = SettingsGroup::new("Options");
    group.add_row(&pinned_folders_row());
    let saved_for_search = Availability::SavedForLater(Milestone::SearchAndMetadata);
    let watch = SettingRow::new(WATCH_FOLDERS);
    let auto_index = PreferenceBinding {
        read: |preferences| preferences.auto_index,
        write: |watch| PreferencesUpdate {
            auto_index: Some(watch),
            ..PreferencesUpdate::default()
        },
    };
    watch.add_control(&page.preference_switch(auto_index), ControlName::RowTitle);
    watch.set_availability(saved_for_search);
    group.add_row(&watch);
    let network = SettingRow::new(NETWORK_CHECKS);
    let interval = PreferenceBinding {
        read: |preferences| preferences.network_interval,
        write: |seconds| PreferencesUpdate {
            network_interval: Some(seconds),
            ..PreferencesUpdate::default()
        },
    };
    let choice = page.preference_choice(&NETWORK_INTERVALS, interval);
    network.add_control(&choice, ControlName::RowTitle);
    network.set_availability(saved_for_search);
    group.add_row(&network);
    group
}

/// "Index pinned folders automatically", waiting for cached search. The
/// owner decided that it starts on (2026-09-28), but neither app indexes
/// pinned folders yet, so the switch shows off until cached search brings
/// the behaviour and a settings key for it.
fn pinned_folders_row() -> SettingRow {
    let row = SettingRow::new(PINNED_FOLDERS);
    row.add_control(&parts::switch(), ControlName::RowTitle);
    row.set_availability(Availability::Unported(Milestone::SearchAndMetadata));
    row
}

/// "Calculate folder sizes", a command of the folder's context menu, and
/// the row that opens how sizes are counted.
fn folder_sizes_group(page: &SettingsPage) -> SettingsGroup {
    let group = SettingsGroup::new("Folder sizes");
    let command = SettingRow::new(FOLDER_SIZES);
    command.set_availability(Availability::Unported(Milestone::SearchAndMetadata));
    group.add_row(&command);
    let details = SettingRow::new(HOW_SIZES_ARE_COUNTED);
    let open = parts::chevron_button(HOW_SIZES_ARE_COUNTED.title);
    open.connect_clicked(glib::clone!(
        #[weak]
        page,
        move |_| page.show_view(SettingsView::Subpage(Subpage::FolderSizes))
    ));
    details.add_control(&open, ControlName::OwnLabel);
    group.add_row(&details);
    group
}

/// The page of how folder sizes are counted: the Python section's help.
pub(super) fn build_folder_sizes() -> SettingsSection {
    let sizes = SettingsSection::new(
        "Folder sizes",
        "How Calculate folder size counts a folder, and what it leaves out.",
        PageKind::Subpage,
    );
    for paragraph in FOLDER_SIZES_HELP {
        sizes.append_text(&parts::paragraph(paragraph));
    }
    sizes
}

#[cfg(test)]
mod tests {
    use ox_core::settings::NETWORK_INTERVALS as ACCEPTED_INTERVALS;

    use super::*;

    /// parity: SET-016
    #[test]
    fn the_network_intervals_offered_are_the_ones_settings_accept() {
        let offered = NETWORK_INTERVALS.map(|choice| choice.value);
        assert_eq!(offered, ACCEPTED_INTERVALS);
    }
}
