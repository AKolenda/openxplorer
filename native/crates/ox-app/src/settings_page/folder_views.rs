// SPDX-License-Identifier: AGPL-3.0-only
//! Appearance › Files and folders: how the folder views show their items.
//!
//! Dolphin's View and Behavior pages: relative dates (`UseShortRelativeDates`,
//! VIEW-004), one display style for all folders or one per folder
//! (`GlobalViewProps`, VIEW-020), the hover selection marker
//! (`ShowSelectionToggle`, SEL-014) and folders that expand in the details
//! view (`ExpandableFolders`, VIEW-035). Every window follows a change at
//! once.

use ox_core::settings::PreferencesUpdate;

use super::bindings::PreferenceBinding;
use super::group::SettingsGroup;
use super::row::{ControlName, SettingRow};
use super::search::RowText;
use super::SettingsPage;

const RELATIVE_DATES: RowText = RowText {
    title: "Relative dates",
    description: "Show “Today” and “Yesterday” with the time; off, every date is shown in full.",
    keywords: "date modified time today yesterday absolute short format column",
};

const FOLDER_STYLES: RowText = RowText {
    title: "Remember each folder's view",
    description: "Each folder keeps its own layout, sorting and grouping; off, every folder shares one.",
    keywords: "view properties per folder display style layout sort group remember global",
};

const SELECTION_MARKER: RowText = RowText {
    title: "Selection marker",
    description: "Hovering an item shows a button that adds it to the selection or takes it out.",
    keywords: "check box checkbox select toggle hover marker plus minus item",
};

const EXPANDABLE_FOLDERS: RowText = RowText {
    title: "Expandable folders",
    description: "In the details view, a folder's arrow lists its contents beneath it.",
    keywords: "tree expand collapse arrow chevron details subfolders nested",
};

/// One row per option, each a switch saved for every window.
pub(super) fn group(page: &SettingsPage) -> SettingsGroup {
    let group = SettingsGroup::new("Files and folders");
    let rows = [
        (
            RELATIVE_DATES,
            PreferenceBinding {
                read: |preferences| !preferences.absolute_dates,
                write: |on| PreferencesUpdate {
                    absolute_dates: Some(!on),
                    ..PreferencesUpdate::default()
                },
            },
        ),
        (
            FOLDER_STYLES,
            PreferenceBinding {
                read: |preferences| preferences.per_folder_views,
                write: |on| PreferencesUpdate {
                    per_folder_views: Some(on),
                    ..PreferencesUpdate::default()
                },
            },
        ),
        (
            SELECTION_MARKER,
            PreferenceBinding {
                read: |preferences| preferences.selection_marker,
                write: |on| PreferencesUpdate {
                    selection_marker: Some(on),
                    ..PreferencesUpdate::default()
                },
            },
        ),
        (
            EXPANDABLE_FOLDERS,
            PreferenceBinding {
                read: |preferences| preferences.expandable_folders,
                write: |on| PreferencesUpdate {
                    expandable_folders: Some(on),
                    ..PreferencesUpdate::default()
                },
            },
        ),
    ];
    for (text, binding) in rows {
        let row = SettingRow::new(text);
        row.add_control(&page.preference_switch(binding), ControlName::RowTitle);
        group.add_row(&row);
    }
    group
}
