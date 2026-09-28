// SPDX-License-Identifier: AGPL-3.0-only
//! Windows & tabs: the open windows, a new window, and moving tabs and
//! files between windows and apps.
//!
//! Ports the "Windows & tabs" section of `appendV07Settings` in
//! `desktop/ui/app.js` (SET-009). "Open windows…" opens the menu of the
//! title bar's windows button (`windowsMenu`), and "New window" runs
//! `app.new-window` (Ctrl+N). The Python
//! section's paragraph about dragging tabs and files becomes three rows
//! that wait for the file-operations milestone, which brings that
//! dragging, and a note with the rest.

use gtk::prelude::*;

use super::group::SettingsGroup;
use super::pages::Category;
use super::parts;
use super::row::{Availability, ControlName, SettingRow};
use super::search::RowText;
use super::section::{PageKind, SettingsSection};
use crate::application::AppAction;
use crate::icons::Icon;
use crate::window::{list_open_windows_on_click, Milestone};

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
                  to pin them.",
    keywords: "drag drop copy pin",
};

/// The rest of the Python section's paragraph.
const DRAGGING_NOTE: &str = "Right-click a tab → Move tab to window… lets you pick an existing \
                             window without dragging. The original is kept until the destination \
                             accepts it. Close this tab's dialogs and finish file operations first. \
                             File drops never remove the source. ZIP contents must be extracted \
                             first; some apps need a mounted network path.";

/// The Windows & tabs page.
pub(super) fn build() -> SettingsSection {
    let category = Category::WindowsAndTabs;
    let windows = SettingsSection::new(category.title(), category.lead(), PageKind::Category);
    windows.append_group(&windows_group());
    windows.append_group(&dragging_group());
    windows.append_text(&parts::note(Icon::Info, DRAGGING_NOTE));
    windows
}

/// "Open windows…" with the windows button's glyph, opening the title
/// bar's windows menu.
fn open_windows_button() -> gtk::MenuButton {
    let button = parts::menu_button_with_glyph("Open windows…", Icon::Desktop);
    list_open_windows_on_click(&button);
    button
}

fn windows_group() -> SettingsGroup {
    let group = SettingsGroup::new("Windows");
    let listing = SettingRow::new(OPEN_WINDOWS);
    listing.add_control(&open_windows_button(), ControlName::OwnLabel);
    group.add_row(&listing);
    let new_window = SettingRow::new(NEW_WINDOW);
    let button = parts::button_with_glyph("New window", Icon::WindowNew);
    button.set_action_name(Some(&AppAction::NewWindow.detailed_name()));
    new_window.add_control(&button, ControlName::OwnLabel);
    group.add_row(&new_window);
    group
}

/// Dragging tabs and files, which arrives with file operations.
fn dragging_group() -> SettingsGroup {
    let pending = Availability::Unported(Milestone::FileOperations);
    let group = SettingsGroup::pending("Tabs and files", pending);
    for text in [MOVE_TABS, DRAG_TO_APPS, DROP_ON_FOLDERS] {
        let row = SettingRow::new(text);
        row.set_availability(pending);
        group.add_row(&row);
    }
    group
}
