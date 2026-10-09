// SPDX-License-Identifier: AGPL-3.0-only
//! General: where new windows open, the windows and tabs, the address
//! bar, and moving tabs and files between windows and apps.
//!
//! Ports the "Windows & tabs" section of `appendV07Settings` in
//! `v2.0.0:desktop/ui/app.js` (SET-009). "Open windows…" opens the menu of the
//! title bar's windows button (`windowsMenu`), and "New window" runs
//! `app.new-window` (Ctrl+N). Dolphin's options for folders opened from
//! other apps and for the address bar join them. The Python
//! section's paragraph about dragging tabs and files becomes three rows,
//! folded away, with the rest in the first row's ⓘ.

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
    title: "New tabs open",
    description: "Where a folder opened in a new tab goes. Ctrl+T always adds a tab at the end.",
    keywords: "open new tabs new tab position after current end tab bar order middle click",
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
    title: "Tab key switches between split panes",
    description: "Off: Tab moves keyboard focus through the window as usual.",
    keywords: "switch between split panes with tab split view tab key switch pane focus keyboard",
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
    title: "Folders from other apps open in a new window",
    description: "Off: a folder opened from another app or the command line opens in a new tab, \
                  and the tab you are using stays where it is.",
    keywords: "open folders from other apps in a new window xdg-open command line external new tab \
               window",
};

const FULL_PATH: RowText = RowText {
    title: "Show the full path",
    description: "Off: inside your home folder the address starts at Home, as in Home / Documents.",
    keywords: "show full path in the address bar breadcrumbs crumbs location bar path root",
};

const EDITABLE_ADDRESS: RowText = RowText {
    title: "Type addresses instead of breadcrumbs in new windows",
    description: "New windows show the address as text you can type in instead of breadcrumbs. \
                  Right-click the address bar to switch one window.",
    keywords: "make the address bar editable in new windows breadcrumbs location bar type text",
};

const TITLE_PATH: RowText = RowText {
    title: "Show the full path in the title bar",
    description: "Off: the title is the folder's name, as in File Explorer.",
    keywords: "show full path in the title bar window title taskbar caption path",
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
                             File drops never remove the source. Files can be dragged out of a \
                             ZIP opened like a folder; from the pop-up window, extract them first. \
                             Some apps need a mounted network path.",
);

/// The General page.
pub(super) fn build(page: &SettingsPage) -> SettingsSection {
    let category = Category::General;
    let general = SettingsSection::new(category.title(), category.lead(), PageKind::Category);
    general.append_group(&startup_group(page));
    general.append_group(&windows_group(page));
    general.append_group(&address_group(page));
    general.append_group(&dragging_group());
    general
}

/// "Open windows…" with the windows button's glyph, opening the title
/// bar's windows menu.
fn open_windows_button() -> gtk::MenuButton {
    let button = parts::menu_button_with_glyph(ox_core::i18n::gettext_static("Open windows…"), Icon::Desktop);
    list_open_windows_on_click(&button);
    button
}

fn windows_group(page: &SettingsPage) -> SettingsGroup {
    let group = SettingsGroup::new(&ox_core::i18n::gettext("Windows and tabs"));
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
    add_split_view_rows(page, &group);
    let listing = SettingRow::new(OPEN_WINDOWS);
    listing.add_control(&open_windows_button(), ControlName::OwnLabel);
    group.add_row(&listing);
    let new_window = SettingRow::new(NEW_WINDOW);
    let button = parts::button_with_glyph(&ox_core::i18n::gettext("New window"), Icon::WindowNew);
    button.set_action_name(Some(&AppAction::NewWindow.detailed_name()));
    new_window.add_control(&button, ControlName::OwnLabel);
    group.add_row(&new_window);
    group
}

/// Split view's options (VIEW-059, Dolphin's "Begin in split view mode"
/// and "Switch between split views with tab key").
fn add_split_view_rows(page: &SettingsPage, group: &SettingsGroup) {
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

/// Dragging tabs and files, which always works: folded away, with the
/// rest of the Python paragraph in the first row's ⓘ.
fn dragging_group() -> SettingsGroup {
    let group = SettingsGroup::new_folded(&ox_core::i18n::gettext("Dragging tabs and files"));
    for text in [MOVE_TABS, DRAG_TO_APPS, DROP_ON_FOLDERS] {
        let row = SettingRow::new(text);
        if text == MOVE_TABS {
            row.add_details(ox_core::i18n::gettext_static(DRAGGING_NOTE));
        }
        group.add_row(&row);
    }
    group
}
