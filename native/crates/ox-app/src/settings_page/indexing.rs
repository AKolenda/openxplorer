// SPDX-License-Identifier: AGPL-3.0-only
//! Search & indexing: the folders to index, how the index keeps current,
//! and folder sizes.
//!
//! Ports the "Search cache" and "Folder sizes" sections of
//! `renderSettingsPage` in `desktop/ui/app.js` (SET-006, SET-008,
//! SET-009), with the options of the settings mockup, including
//! SRCH-040's "Index pinned folders automatically". The folder list opens
//! as a page of its own ([`super::indexed_folders`]). Cached search is not
//! in the native preview yet: its actions wait for it, while the two
//! options the Python app already reads (`autoIndex`, `networkInterval`)
//! are saved for it now.

use gtk::glib;
use gtk::prelude::*;
use ox_core::settings::PreferencesUpdate;

use super::bindings::{Choice, PreferenceBinding};
use super::group::SettingsGroup;
use super::pages::{Category, SettingsView, Subpage};
use super::parts::{self, StatusText};
use super::row::{Availability, ControlName, SettingRow};
use super::search::RowText;
use super::section::{PageKind, SettingsSection};
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
    description: "SMB and unwatched folders use incremental directory checks, not push \
                  notifications. Large trees take longer than one interval.",
    keywords: "network refresh interval smb polling nas seconds minute",
};

const FOLDER_SIZES: RowText = RowText {
    title: "Calculate folder sizes",
    description: "Right-click a folder → Calculate folder size. Results are kept only for this \
                  window session.",
    keywords: "zfs logical bytes snapshots disk usage",
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

/// The limits of indexing, from the Python section's help.
const LIMITS_NOTE: &str = "Local changes update the index after a short debounce. Watching uses up \
                           to 8,192 directories; timed checks cover any remaining ones. Disk roots \
                           skip system/temporary folders, nested mounts and symlinks. Select each \
                           mounted volume separately. Initial scans are limited to 1 million \
                           entries.";

/// How folder sizes are counted, from the Python "Folder sizes" section.
const FOLDER_SIZES_NOTE: &str = "Scans run on demand, outside the browsing worker pool. Results are \
                                 logical file bytes, not compressed ZFS space or snapshot usage. \
                                 Scans include hidden files, but skip links, nested filesystem \
                                 mounts and snapshot collections. Each folder is limited to 1 \
                                 million entries or 5 minutes between I/O calls. Inaccessible or \
                                 excluded entries produce a partial total. Cancel stops the active \
                                 scan and any queued folders. Recalculate to pick up later changes.";

/// The Search & indexing page.
pub(super) fn build(page: &SettingsPage) -> SettingsSection {
    let category = Category::SearchAndIndexing;
    let indexing = SettingsSection::new(category.title(), category.lead(), PageKind::Category);
    indexing.append_extra(&status_card());
    indexing.append_group(&folders_group(page));
    indexing.append_group(&options_group(page));
    indexing.append_extra(&parts::note(Icon::ShieldLock, PRIVACY_NOTE));
    indexing.append_extra(&parts::note(Icon::Info, LIMITS_NOTE));
    indexing.append_group(&folder_sizes_group());
    indexing.append_extra(&parts::note(Icon::Info, FOLDER_SIZES_NOTE));
    indexing
}

/// Where instant search stands, with "Refresh all" waiting for it.
fn status_card() -> gtk::Box {
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
    parts::status_card(status, &[refresh.upcast()])
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

/// SRCH-040's switch, on as the owner decided, waiting for cached search:
/// no settings key exists for it yet.
fn pinned_folders_row() -> SettingRow {
    let row = SettingRow::new(PINNED_FOLDERS);
    let switch = parts::switch();
    switch.set_active(true);
    row.add_control(&switch, ControlName::RowTitle);
    row.set_availability(Availability::Unported(Milestone::SearchAndMetadata));
    row
}

/// "Calculate folder sizes", a command of the folder's context menu.
fn folder_sizes_group() -> SettingsGroup {
    let group = SettingsGroup::new("Folder sizes");
    let row = SettingRow::new(FOLDER_SIZES);
    row.set_availability(Availability::Unported(Milestone::SearchAndMetadata));
    group.add_row(&row);
    group
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
