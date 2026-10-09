// SPDX-License-Identifier: AGPL-3.0-only
//! Confirmations: the questions asked before items are deleted, before a
//! program that is opened runs (OPEN-008) and before a window with several
//! tabs closes, as Dolphin's Confirmations page (SET-010).

use ox_core::settings::PreferencesUpdate;

use super::bindings::PreferenceBinding;
use super::group::SettingsGroup;
use super::pages::Category;
use super::row::{ControlName, SettingRow};
use super::search::RowText;
use super::section::{PageKind, SettingsSection};
use super::SettingsPage;

const CONFIRM_TRASH: RowText = RowText {
    title: "Moving items to the Recycle Bin",
    description: "Off: Delete moves the selection to the Recycle Bin at once; Undo brings it back.",
    keywords: "ask before moving items to the recycle bin confirm confirmation trash delete recycle \
               bin question warning",
};

const CONFIRM_DELETE: RowText = RowText {
    title: "Deleting permanently",
    description: "Shift+Delete, and items on drives without a Recycle Bin.",
    keywords: "ask before deleting permanently confirm confirmation permanent delete shift question \
               warning",
};

const CONFIRM_EMPTY: RowText = RowText {
    title: "Emptying the Recycle Bin",
    description: "Everything in it is deleted permanently.",
    keywords: "ask before emptying the recycle bin confirm confirmation empty trash recycle bin \
               question warning",
};

const CONFIRM_CLOSE_TABS: RowText = RowText {
    title: "Closing a window with several tabs",
    description: "Closing the window closes all of its tabs.",
    keywords: "ask before closing a window with several tabs confirm confirmation close window tabs \
               quit question warning",
};

const ASK_TO_RUN: RowText = RowText {
    title: "Running programs and scripts",
    description: "Off: opening one shows it in its viewer or editor, and nothing runs.",
    keywords: "ask whether to run programs and scripts confirm execute run program script \
               executable launcher open",
};

/// The Confirmations page.
pub(super) fn build(page: &SettingsPage) -> SettingsSection {
    let category = Category::Confirmations;
    let confirmations = SettingsSection::new(category.title(), category.lead(), PageKind::Category);
    confirmations.append_group(&ask_before_group(page));
    confirmations
}

/// The questions asked before items are deleted, before a program that is
/// opened runs (OPEN-008) and before a window with several tabs closes
/// (Dolphin's Confirmations page, SET-010).
fn ask_before_group(page: &SettingsPage) -> SettingsGroup {
    let group = SettingsGroup::new(&ox_core::i18n::gettext("Ask before…"));
    let bindings = [
        (
            CONFIRM_TRASH,
            PreferenceBinding {
                read: |preferences| preferences.confirm_trash,
                write: |on| PreferencesUpdate {
                    confirm_trash: Some(on),
                    ..PreferencesUpdate::default()
                },
            },
        ),
        (
            CONFIRM_DELETE,
            PreferenceBinding {
                read: |preferences| preferences.confirm_delete,
                write: |on| PreferencesUpdate {
                    confirm_delete: Some(on),
                    ..PreferencesUpdate::default()
                },
            },
        ),
        (
            CONFIRM_EMPTY,
            PreferenceBinding {
                read: |preferences| preferences.confirm_empty_trash,
                write: |on| PreferencesUpdate {
                    confirm_empty_trash: Some(on),
                    ..PreferencesUpdate::default()
                },
            },
        ),
        (
            CONFIRM_CLOSE_TABS,
            PreferenceBinding {
                read: |preferences| preferences.confirm_close_tabs,
                write: |on| PreferencesUpdate {
                    confirm_close_tabs: Some(on),
                    ..PreferencesUpdate::default()
                },
            },
        ),
        (
            ASK_TO_RUN,
            PreferenceBinding {
                read: |preferences| preferences.ask_to_run_programs,
                write: |on| PreferencesUpdate {
                    ask_to_run_programs: Some(on),
                    ..PreferencesUpdate::default()
                },
            },
        ),
    ];
    for (text, binding) in bindings {
        let row = SettingRow::new(text);
        row.add_control(&page.preference_switch(binding), ControlName::RowTitle);
        group.add_row(&row);
    }
    group
}
