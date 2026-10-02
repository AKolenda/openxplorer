// SPDX-License-Identifier: AGPL-3.0-only
//! About: what this build is, its updates and its licence.
//!
//! Ports the "`OpenXplorer` · License & source" section of
//! `renderSettingsPage` in `v2.0.0:desktop/ui/app.js` (SET-009, UPD-016) and the
//! status bar's "Check for updates" (UPD-001). "About this build" runs the
//! window's `win.about`, as the More menu does; "Check for updates" runs
//! `win.check-updates`, the Software updates dialog, and its row says what
//! the last check in any window found. "Read license & source
//! information" runs `win.license`, the licence dialog.

use gtk::prelude::*;
use gtk::subclass::prelude::*;

use super::group::SettingsGroup;
use super::pages::Category;
use super::parts;
use super::row::{ControlName, SettingRow};
use super::search::RowText;
use super::section::{PageKind, SettingsSection};
use super::status_card::{StatusCard, StatusText};
use super::{SettingsPage, SharedHandler};
use crate::config::BUILD_NAME;
use crate::icons::Icon;
use crate::update::Updates;
use crate::window::{ButtonStyle, WindowAction};

/// A group of one row whose button runs a window action.
struct ActionGroup {
    /// The group's heading.
    heading: &'static str,
    /// The row's text.
    text: RowText,
    /// The button's label.
    button: &'static str,
    /// What the button runs.
    action: WindowAction,
}

const UPDATES: ActionGroup = ActionGroup {
    heading: "Updates",
    text: RowText {
        title: "Check for updates",
        description: "Look for a newer OpenXplorer release.",
        keywords: "update version release upgrade software updates install restart flatpak",
    },
    button: "Check for updates",
    action: WindowAction::CheckUpdates,
};

const LICENSE_AND_SOURCE: ActionGroup = ActionGroup {
    heading: "License & source",
    text: RowText {
        title: "OpenXplorer · License & source",
        description: "Copyright (c) 2026 OpenXplorer contributors. AGPL-3.0-only. No warranty.",
        keywords: "licence agpl source code copyright mit notice. Original component notices are \
                   preserved.",
    },
    button: "Read license & source information",
    action: WindowAction::License,
};

/// The About page.
pub(super) fn build(page: &SettingsPage) -> SettingsSection {
    let category = Category::About;
    let about = SettingsSection::new(category.title(), category.lead(), PageKind::Category);
    about.append_card(&build_card());
    let (updates, updates_row) = action_group(&UPDATES);
    about.append_group(&updates);
    follow_updates(page, &updates_row);
    about.append_group(&action_group(&LICENSE_AND_SOURCE).0);
    about
}

/// The build's name and description, with "About this build".
fn build_card() -> StatusCard {
    let about_build = parts::button(&ox_core::i18n::gettext("About this build"), ButtonStyle::Bordered);
    WindowAction::About.assign_to(&about_build);
    let status = StatusText {
        glyph: Icon::Info,
        title: BUILD_NAME,
        text: "An independent Windows 11–inspired file manager for Zorin. Desktop: GTK 4 + \
               GIO/GVfs.",
        notice: None,
    };
    StatusCard::new(status, &[about_build.upcast()])
}

/// The group `spec` describes, and its row.
fn action_group(spec: &ActionGroup) -> (SettingsGroup, SettingRow) {
    let group = SettingsGroup::new(spec.heading);
    let row = SettingRow::new(spec.text);
    let button = parts::button(spec.button, ButtonStyle::Bordered);
    spec.action.assign_to(&button);
    row.add_control(&button, ControlName::OwnLabel);
    group.add_row(&row);
    (group, row)
}

/// Keeps the Check for updates row saying what the application knows
/// about updates: the installed version and what the last check found.
fn follow_updates(page: &SettingsPage, row: &SettingRow) {
    let updates = page.context().updates();
    let show = |row: &SettingRow, updates: &Updates| {
        row.set_description(&updates.state().about_summary(Updates::running_version()));
    };
    show(row, updates);
    let weak_row = row.downgrade();
    let id = updates.connect_state_changed(move |updates| {
        if let Some(row) = weak_row.upgrade() {
            show(&row, updates);
        }
    });
    page.imp().handlers.borrow_mut().push(SharedHandler {
        object: updates.clone().upcast(),
        id,
    });
}
