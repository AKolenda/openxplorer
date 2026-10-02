// SPDX-License-Identifier: AGPL-3.0-only
//! Windows & tabs: the open windows, a new window, and moving tabs and
//! files between windows and apps.
//!
//! Ports the "Windows & tabs" section of `appendV07Settings` in
//! `v2.0.0:desktop/ui/app.js` (SET-009). "Open windows…" opens the menu of the
//! title bar's windows button (`windowsMenu`), and "New window" runs
//! `app.new-window` (Ctrl+N). Dolphin's options for folders opened from
//! other apps and for the address bar join them. The Python
//! section's paragraph about dragging tabs and files becomes three rows
//! and a note with the rest.

use gtk::prelude::*;
use ox_core::settings::PreferencesUpdate;

use super::bindings::{Choice, PreferenceBinding};
use super::group::SettingsGroup;
use super::pages::Category;
use super::parts;
use super::row::{ControlName, SettingRow};
use super::search::RowText;
use super::section::{PageKind, SettingsSection};
use super::startup::startup_group;
use super::SettingsPage;
use crate::application::AppAction;
use crate::icons::Icon;
use crate::window::list_open_windows_on_click;

const NEW_TAB_POSITION: RowText = RowText {
    title: "Open new tabs",
    description: "Where a folder opened in a new tab goes. Ctrl+T always adds a tab at the end.",
    keywords: "new tab position after current end tab bar order middle click",
};

/// The choices of "Open new tabs" (Dolphin's `OpenNewTabAfterLastTab`).
const NEW_TAB_POSITIONS: [Choice<bool>; 2] = [
    Choice {
        value: false,
        label: crate::i18n::message_id("After the current tab"),
    },
    Choice {
        value: true,
        label: crate::i18n::message_id("At the end of the tab bar"),
    },
];

const BEGIN_SPLIT: RowText = RowText {
    title: "Open new windows in split view",
    description: "New windows show two folders side by side. F3 splits or unsplits a tab.",
    keywords: "split view dual pane two panes side by side f3 commander",
};

const TAB_SWITCHES_PANES: RowText = RowText {
    title: "Switch between split panes with Tab",
    description: "Off: Tab moves keyboard focus through the window as usual.",
    keywords: "split view tab key switch pane focus keyboard",
};

const OPEN_WINDOWS: RowText = RowText {
    title: "Open windows",
    description: "Every OpenXplorer window by title; choose one to bring it to the front.",
    keywords: "taskbar panel list switch focus quit",
};

const NEW_WINDOW: RowText = RowText {
    title: "New window",
    description: "Opens another window (Ctrl+N).",
    keywords: "separate window",
};

const EXTERNAL_FOLDERS: RowText = RowText {
    title: "Open folders from other apps in a new window",
    description: "Off: a folder opened from another app or the command line opens in a new tab, \
                  and the tab you are using stays where it is.",
    keywords: "xdg-open command line external new tab window",
};

const FULL_PATH: RowText = RowText {
    title: "Show full path in the address bar",
    description: "Off: inside your home folder the address starts at Home, as in Home / Documents.",
    keywords: "breadcrumbs crumbs location bar path root",
};

const EDITABLE_ADDRESS: RowText = RowText {
    title: "Make the address bar editable in new windows",
    description: "New windows show the address as text you can type in instead of breadcrumbs. \
                  Right-click the address bar to switch one window.",
    keywords: "breadcrumbs location bar type text",
};

const TITLE_PATH: RowText = RowText {
    title: "Show full path in the title bar",
    description: "Off: the title is the folder's name, as in File Explorer.",
    keywords: "window title taskbar caption path",
};

const CONFIRM_TRASH: RowText = RowText {
    title: "Ask before moving items to the Recycle Bin",
    description: "Off: Delete moves the selection to the Recycle Bin at once; Undo brings it back.",
    keywords: "confirm confirmation trash delete recycle bin question warning",
};

const CONFIRM_DELETE: RowText = RowText {
    title: "Ask before deleting permanently",
    description: "Shift+Delete, and items on drives without a Recycle Bin.",
    keywords: "confirm confirmation permanent delete shift question warning",
};

const CONFIRM_EMPTY: RowText = RowText {
    title: "Ask before emptying the Recycle Bin",
    description: "Everything in it is deleted permanently.",
    keywords: "confirm confirmation empty trash recycle bin question warning",
};

const CONFIRM_CLOSE_TABS: RowText = RowText {
    title: "Ask before closing a window with several tabs",
    description: "Closing the window closes all of its tabs.",
    keywords: "confirm confirmation close window tabs quit question warning",
};

const ASK_TO_RUN: RowText = RowText {
    title: "Ask whether to run programs and scripts",
    description: "Off: opening one shows it in its viewer or editor, and nothing runs.",
    keywords: "confirm execute run program script executable launcher open",
};

const MOVE_TABS: RowText = RowText {
    title: "Move tabs between windows",
    description: "Drag a tab onto another OpenXplorer window's tab strip to merge it, or outside \
                  a window to detach it.",
    keywords: "detach drag separate window merge move tab to window",
};

const DRAG_TO_APPS: RowText = RowText {
    title: "Drag files into other apps",
    description: "Drag selected files or folders into another app to open or attach them.",
    keywords: "drag drop attach",
};

const DROP_ON_FOLDERS: RowText = RowText {
    title: "Drop files on folders",
    description: "Drop files on a writable folder to copy them, or drop folders in Quick access \
                  to pin them. Hold Shift to move, Ctrl+Shift to create links or Alt to choose; \
                  drop files on a program to open them with it.",
    keywords: "drag drop copy move link pin program run",
};

/// The rest of the Python section's paragraph.
const DRAGGING_NOTE: &str = crate::i18n::message_id(
    "Right-click a tab → Move tab to window… lets you pick an existing \
                             window without dragging. The original is kept until the destination \
                             accepts it. Close this tab's dialogs and finish file operations first. \
                             File drops never remove the source. ZIP contents must be extracted \
                             first; some apps need a mounted network path.",
);

const BROWSE_ARCHIVES: RowText = RowText {
    title: "Open archives as folders",
    description: "Browse ZIP and TAR archives (.tar, .tar.gz, .tar.bz2, .tar.xz, .tar.zst) inside \
                  OpenXplorer. Off, they open in their default application.",
    keywords: "zip tar gz archive compressed browse extract",
};

/// The Windows & tabs page.
pub(super) fn build(page: &SettingsPage) -> SettingsSection {
    let category = Category::WindowsAndTabs;
    let windows = SettingsSection::new(category.title(), category.lead(), PageKind::Category);
    windows.append_group(&windows_group(page));
    windows.append_group(&startup_group(page));
    windows.append_group(&split_view_group(page));
    windows.append_group(&address_group(page));
    windows.append_group(&archives_group(page));
    windows.append_group(&confirmations_group(page));
    windows.append_group(&dragging_group());
    windows.append_text(&parts::note(
        Icon::Info,
        ox_core::i18n::gettext_static(DRAGGING_NOTE),
    ));
    windows
}

/// "Open windows…" with the windows button's glyph, opening the title
/// bar's windows menu.
fn open_windows_button() -> gtk::MenuButton {
    let button = parts::menu_button_with_glyph(ox_core::i18n::gettext_static("Open windows…"), Icon::Desktop);
    list_open_windows_on_click(&button);
    button
}

fn windows_group(page: &SettingsPage) -> SettingsGroup {
    let group = SettingsGroup::new(&ox_core::i18n::gettext("Windows"));
    let listing = SettingRow::new(OPEN_WINDOWS);
    listing.add_control(&open_windows_button(), ControlName::OwnLabel);
    group.add_row(&listing);
    let new_window = SettingRow::new(NEW_WINDOW);
    let button = parts::button_with_glyph(&ox_core::i18n::gettext("New window"), Icon::WindowNew);
    button.set_action_name(Some(&AppAction::NewWindow.detailed_name()));
    new_window.add_control(&button, ControlName::OwnLabel);
    group.add_row(&new_window);
    let external = SettingRow::new(EXTERNAL_FOLDERS);
    let in_new_window = PreferenceBinding {
        read: |preferences| preferences.external_folders_in_new_window,
        write: |on| PreferencesUpdate {
            external_folders_in_new_window: Some(on),
            ..PreferencesUpdate::default()
        },
    };
    external.add_control(&page.preference_switch(in_new_window), ControlName::RowTitle);
    group.add_row(&external);
    let title_path = SettingRow::new(TITLE_PATH);
    let full_path_in_title = PreferenceBinding {
        read: |preferences| preferences.full_path_in_title,
        write: |on| PreferencesUpdate {
            full_path_in_title: Some(on),
            ..PreferencesUpdate::default()
        },
    };
    title_path.add_control(&page.preference_switch(full_path_in_title), ControlName::RowTitle);
    group.add_row(&title_path);
    let new_tabs = SettingRow::new(NEW_TAB_POSITION);
    let at_end = PreferenceBinding {
        read: |preferences| preferences.open_tabs_at_end,
        write: |at_end| PreferencesUpdate {
            open_tabs_at_end: Some(at_end),
            ..PreferencesUpdate::default()
        },
    };
    new_tabs.add_control(
        &page.preference_choice(&NEW_TAB_POSITIONS, at_end),
        ControlName::RowTitle,
    );
    group.add_row(&new_tabs);
    group
}

/// Split view's options (VIEW-059, Dolphin's "Begin in split view mode"
/// and "Switch between split views with tab key").
fn split_view_group(page: &SettingsPage) -> SettingsGroup {
    let group = SettingsGroup::new(&ox_core::i18n::gettext("Split view"));
    let bindings = [
        (
            BEGIN_SPLIT,
            PreferenceBinding {
                read: |preferences| preferences.begin_in_split_view,
                write: |on| PreferencesUpdate {
                    begin_in_split_view: Some(on),
                    ..PreferencesUpdate::default()
                },
            },
        ),
        (
            TAB_SWITCHES_PANES,
            PreferenceBinding {
                read: |preferences| preferences.tab_switches_split_panes,
                write: |on| PreferencesUpdate {
                    tab_switches_split_panes: Some(on),
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

/// The questions asked before items are deleted, before a program that is
/// opened runs (OPEN-008) and before a window with several tabs closes
/// (Dolphin's Confirmations page, SET-010).
fn confirmations_group(page: &SettingsPage) -> SettingsGroup {
    let group = SettingsGroup::new(&ox_core::i18n::gettext("Confirmations"));
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
            ASK_TO_RUN,
            PreferenceBinding {
                read: |preferences| preferences.ask_to_run_programs,
                write: |on| PreferencesUpdate {
                    ask_to_run_programs: Some(on),
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
    ];
    for (text, binding) in bindings {
        let row = SettingRow::new(text);
        row.add_control(&page.preference_switch(binding), ControlName::RowTitle);
        group.add_row(&row);
    }
    group
}

/// The address bar's options (NAV-024, NAV-029).
fn address_group(page: &SettingsPage) -> SettingsGroup {
    let group = SettingsGroup::new(&ox_core::i18n::gettext("Address bar"));
    let full_path = SettingRow::new(FULL_PATH);
    let show_full_path = PreferenceBinding {
        read: |preferences| preferences.show_full_path,
        write: |on| PreferencesUpdate {
            show_full_path: Some(on),
            ..PreferencesUpdate::default()
        },
    };
    full_path.add_control(&page.preference_switch(show_full_path), ControlName::RowTitle);
    group.add_row(&full_path);
    let editable = SettingRow::new(EDITABLE_ADDRESS);
    let editable_location = PreferenceBinding {
        read: |preferences| preferences.editable_location,
        write: |on| PreferencesUpdate {
            editable_location: Some(on),
            ..PreferencesUpdate::default()
        },
    };
    editable.add_control(&page.preference_switch(editable_location), ControlName::RowTitle);
    group.add_row(&editable);
    group
}

/// Whether archives open as folders (ARC-022), as Dolphin's Navigation
/// setting "Open archives as folder".
fn archives_group(page: &SettingsPage) -> SettingsGroup {
    let group = SettingsGroup::new(&ox_core::i18n::gettext("Archives"));
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
    group
}

/// Dragging tabs and files.
fn dragging_group() -> SettingsGroup {
    let group = SettingsGroup::new(&ox_core::i18n::gettext("Tabs and files"));
    for text in [MOVE_TABS, DRAG_TO_APPS, DROP_ON_FOLDERS] {
        group.add_row(&SettingRow::new(text));
    }
    group
}
