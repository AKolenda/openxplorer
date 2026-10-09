// SPDX-License-Identifier: AGPL-3.0-only
//! ZIP & archives: whether archives open as folders (ARC-022), how a ZIP
//! opens (ARC-026), and whether other apps open ZIP files in
//! `OpenXplorer`, the ZIP route of the Default apps page (INT-011).

use gtk::glib;
use gtk::prelude::*;
use ox_core::settings::{PreferencesUpdate, ZipOpening};

use super::bindings::{Choice, PreferenceBinding};
use super::group::SettingsGroup;
use super::pages::Category;
use super::row::{ControlName, SettingRow};
use super::search::RowText;
use super::section::{PageKind, SettingsSection};
use super::SettingsPage;

const BROWSE_ARCHIVES: RowText = RowText {
    title: "Open archives as folders",
    description: "Browse ZIP and TAR archives (.tar, .tar.gz, .tar.bz2, .tar.xz, .tar.zst) inside \
                  OpenXplorer. Off, they open in their default application.",
    keywords: "zip tar gz archive compressed browse extract",
};

const ZIP_OPENING: RowText = RowText {
    title: "Double-clicking a ZIP",
    description: "Like a folder opens a ZIP in the tab, as Windows Explorer does: browse it with \
                  the address bar, Back and Up, copy or drag files out, and Extract all from the \
                  bar. In a pop-up window shows it over the tab.",
    keywords: "open zip files zip open folder window pop-up compressed explorer browse double click",
};

/// How a ZIP opens (ARC-026).
const ZIP_OPENINGS: [Choice<ZipOpening>; 2] = [
    Choice {
        value: ZipOpening::Folder,
        label: crate::i18n::message_id("Like a folder (Windows)"),
    },
    Choice {
        value: ZipOpening::Window,
        label: crate::i18n::message_id("In a pop-up window (default)"),
    },
];

/// The ZIP & archives page. `zip_files` is the Default apps page's group
/// with the app that opens ZIP files, which this page shows.
pub(super) fn build(page: &SettingsPage, zip_files: &SettingsGroup) -> SettingsSection {
    let category = Category::Archives;
    let archives = SettingsSection::new(category.title(), category.lead(), PageKind::Category);
    archives.append_group(&opening_group(page));
    archives.append_group(zip_files);
    archives
}

/// Whether archives open as folders (ARC-022), as Dolphin's Navigation
/// setting "Open archives as folder".
fn opening_group(page: &SettingsPage) -> SettingsGroup {
    let group = SettingsGroup::new(&ox_core::i18n::gettext("Opening archives"));
    let row = SettingRow::new(BROWSE_ARCHIVES);
    let binding = PreferenceBinding {
        read: |preferences| preferences.browse_archives,
        write: |browse| PreferencesUpdate {
            browse_archives: Some(browse),
            ..PreferencesUpdate::default()
        },
    };
    row.add_control(&page.preference_switch(binding), ControlName::RowTitle);
    group.add_row(&row);
    let opening = SettingRow::new(ZIP_OPENING);
    let binding = PreferenceBinding {
        read: |preferences| preferences.zip_opening,
        write: |opening| PreferencesUpdate {
            zip_opening: Some(opening),
            ..PreferencesUpdate::default()
        },
    };
    let choice = page.preference_choice(&ZIP_OPENINGS, binding);
    // Only a ZIP that is browsed opens one way or the other.
    page.follow_preferences(glib::clone!(
        #[weak]
        choice,
        move |preferences| choice.set_sensitive(preferences.browse_archives)
    ));
    opening.add_control(&choice, ControlName::RowTitle);
    group.add_row(&opening);
    group
}
