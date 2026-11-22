// SPDX-License-Identifier: AGPL-3.0-only
//! The opt-in desktop frame, applied only before a new window is shown.

use ox_core::settings::PreferencesUpdate;

use super::bindings::PreferenceBinding;
use super::row::{ControlName, SettingRow};
use super::search::RowText;
use super::SettingsPage;

const SYSTEM_WINDOW_DECORATIONS: RowText = RowText {
    title: "Use system window decorations",
    description: "Use your desktop's title bar and window buttons, with tabs below. Applies to new \
                  windows; close and reopen existing windows after file operations finish. On desktops \
                  without system decorations, GTK provides a standard title bar.",
    keywords: "appearance layout frame border titlebar title bar desktop native system decorations \
               KDE KWin GNOME Wayland X11 CSD SSD minimize maximize close",
};

/// The searchable, keyboard-accessible Appearance > Layout switch.
pub(super) fn row(page: &SettingsPage) -> SettingRow {
    let row = SettingRow::new(SYSTEM_WINDOW_DECORATIONS);
    let switch = page.preference_switch(PreferenceBinding {
        read: |preferences| preferences.system_window_decorations,
        write: |on| PreferencesUpdate {
            system_window_decorations: Some(on),
            ..PreferencesUpdate::default()
        },
    });
    row.add_control(&switch, ControlName::RowTitle);
    row
}
