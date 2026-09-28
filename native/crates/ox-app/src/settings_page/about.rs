// SPDX-License-Identifier: AGPL-3.0-only
//! About: what this build is, its updates and its licence.
//!
//! Ports the "`OpenXplorer` · License & source" section of
//! `renderSettingsPage` in `desktop/ui/app.js` (SET-009, UPD-016) and the
//! status bar's "Check for updates". "About this build" runs the window's
//! `win.about`, as the More menu does; the licence dialog and the update
//! check run `win.license` and `win.check-updates`, which wait for the
//! packaging and updates milestone.

use gtk::prelude::*;

use super::group::SettingsGroup;
use super::pages::Category;
use super::parts;
use super::row::{Availability, ControlName, SettingRow};
use super::search::RowText;
use super::section::{PageKind, SettingsSection};
use super::status_card::{StatusCard, StatusText};
use crate::config::BUILD_NAME;
use crate::icons::Icon;
use crate::window::{ButtonStyle, Milestone, WindowAction};

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
        keywords: "update version release upgrade",
    },
    button: "Check for updates",
    action: WindowAction::CheckUpdates,
};

const LICENSE_AND_SOURCE: ActionGroup = ActionGroup {
    heading: "License & source",
    text: RowText {
        title: "OpenXplorer · License & source",
        description: "Copyright (c) 2026 OpenXplorer contributors. AGPL-3.0-only. No warranty. \
                      Original component notices are preserved.",
        keywords: "licence agpl source code copyright mit notice",
    },
    button: "Read license & source information",
    action: WindowAction::License,
};

/// The About page.
pub(super) fn build() -> SettingsSection {
    let category = Category::About;
    let about = SettingsSection::new(category.title(), category.lead(), PageKind::Category);
    about.append_card(&build_card());
    let pending = Availability::Unported(Milestone::Distribution);
    about.append_group(&action_group(&UPDATES, pending));
    about.append_group(&action_group(&LICENSE_AND_SOURCE, pending));
    about
}

/// The build's name and description, with "About this build".
fn build_card() -> StatusCard {
    let about_build = parts::button("About this build", ButtonStyle::Bordered);
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

/// The group `spec` describes, waiting for `pending`.
fn action_group(spec: &ActionGroup, pending: Availability) -> SettingsGroup {
    let group = SettingsGroup::new(spec.heading);
    let row = SettingRow::new(spec.text);
    let button = parts::button(spec.button, ButtonStyle::Bordered);
    spec.action.assign_to(&button);
    row.add_control(&button, ControlName::OwnLabel);
    row.set_availability(pending);
    group.add_row(&row);
    group
}
