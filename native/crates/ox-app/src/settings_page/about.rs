// SPDX-License-Identifier: AGPL-3.0-only
//! About: what this build is, its updates and its licence.
//!
//! Ports the "`OpenXplorer` · License & source" section of
//! `renderSettingsPage` in `v2.0.0:desktop/ui/app.js` (SET-009, UPD-016) and the
//! status bar's "Check for updates" (UPD-001). "About this build" runs the
//! window's `win.about`; "Check for updates" runs
//! `win.check-updates`, the Software updates dialog, and its row says what
//! the last check in any window found. "Check for updates automatically"
//! turns the daily check at launch on or off. "Install updates without
//! asking for a password", on by default and shown only in the stable
//! Debian package, has the update helper remove or add the polkit rule
//! that lets an administrator at the machine update without a password;
//! turning it on asks for an administrator's password (turning it off
//! does not, for an administrator at the machine, whom the rule already
//! lets through), and it shows whether the rule is there. "Show the Update
//! button in the command bar", on by default, hides the command bar's
//! Update when turned off; the status bar's Update available stays. "Read license & source
//! information" runs `win.license`, the licence dialog. "Keyboard
//! shortcuts" and "Help" run `win.keyboard-shortcuts` (Ctrl+?) and
//! `win.help` (F1): they, the licence and the build left the command
//! bar's "…" menu for this page (CMD-007), as Windows 11 File Explorer's
//! "…" has none of them.

use std::rc::Rc;

use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;
use ox_core::settings::PreferencesUpdate;
use ox_core::update::PasswordlessUpdates;

use super::bindings::PreferenceBinding;
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
use crate::update::{PasswordlessChange, Updates};
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
    heading: crate::i18n::message_id("Updates"),
    text: RowText {
        title: "Check for updates",
        description: "Look for a newer OpenXplorer release.",
        keywords: "update version release upgrade software updates install restart flatpak",
    },
    button: crate::i18n::message_id("Check for updates"),
    action: WindowAction::CheckUpdates,
};

const AUTOMATIC_CHECKS: RowText = RowText {
    title: "Check for updates automatically",
    description: "Once a day, when OpenXplorer starts, ask GitHub whether a newer release exists. Nothing \
                  installs without you.",
    keywords: "automatic update check daily launch startup notify notification network github privacy",
};

const PASSWORDLESS_UPDATES: RowText = RowText {
    title: "Install updates without asking for a password",
    description: "Administrators at this computer install OpenXplorer updates without typing their \
                  password. Turning it on asks for an administrator's password.",
    keywords: "password prompt administrator admin sudo polkit pkexec authentication install update \
               without password passwordless",
};

const UPDATE_BUTTON: RowText = RowText {
    title: "Show the Update button in the command bar",
    description: "While a newer release is available, show Update beside the Settings gear, as well as \
                  Update available in the status bar.",
    keywords: "update button command bar toolbar accent restart show hide notify",
};

const LICENSE_AND_SOURCE: ActionGroup = ActionGroup {
    heading: crate::i18n::message_id("License & source"),
    text: RowText {
        title: "OpenXplorer · License & source",
        description: "Copyright (c) 2026 OpenXplorer contributors. AGPL-3.0-only. No warranty.",
        keywords: "license licence agpl source code copyright mit notice. Original component notices are \
                   preserved.",
    },
    button: crate::i18n::message_id("Read license & source information"),
    action: WindowAction::License,
};

const KEYBOARD_SHORTCUTS: ActionRow = ActionRow {
    text: RowText {
        title: "Keyboard shortcuts",
        description: "Every key OpenXplorer has, grouped by task. Ctrl+? opens them too.",
        keywords: "keys keyboard shortcuts hotkeys accelerators bindings list ctrl+? cheat sheet",
    },
    button: crate::i18n::message_id("Show keyboard shortcuts"),
    action: WindowAction::KeyboardShortcuts,
};

const HELP: ActionRow = ActionRow {
    text: RowText {
        title: "Help",
        description: "The OpenXplorer manual, which needs no network connection. F1 opens it too.",
        keywords: "help manual guide documentation f1 how to user manual offline",
    },
    button: crate::i18n::message_id("Open the manual"),
    action: WindowAction::Help,
};

/// A row whose button runs a window action.
struct ActionRow {
    /// The row's text.
    text: RowText,
    /// The button's label.
    button: &'static str,
    /// What the button runs.
    action: WindowAction,
}

/// The About page.
pub(super) fn build(page: &SettingsPage) -> SettingsSection {
    let category = Category::About;
    let about = SettingsSection::new(category.title(), category.lead(), PageKind::Category);
    about.append_card(&build_card());
    let (updates, updates_row) = action_group(&UPDATES);
    updates.add_row(&automatic_checks_row(page));
    if let Some(setting) = page.context().passwordless_updates() {
        updates.add_row(&passwordless_row(page, setting.clone()));
    }
    updates.add_row(&update_button_row(page));
    about.append_group(&updates);
    follow_updates(page, &updates_row);
    about.append_group(&help_group());
    about.append_group(&action_group(&LICENSE_AND_SOURCE).0);
    about
}

/// "Help": the keyboard shortcuts and the manual.
fn help_group() -> SettingsGroup {
    let group = SettingsGroup::new(&ox_core::i18n::gettext("Help"));
    for spec in [&KEYBOARD_SHORTCUTS, &HELP] {
        group.add_row(&action_row(spec.text, spec.button, spec.action));
    }
    group
}

/// The build's name and description, with "About this build".
fn build_card() -> StatusCard {
    let about_build = parts::button(&ox_core::i18n::gettext("About this build"), ButtonStyle::Bordered);
    WindowAction::About.assign_to(&about_build);
    let text = ox_core::i18n::gettext(
        "An independent Windows 11–inspired file manager for Linux. Desktop: GTK 4 + GIO/GVfs.",
    );
    let status = StatusText {
        glyph: Icon::Info,
        title: BUILD_NAME,
        text: &text,
        notice: None,
    };
    StatusCard::new(status, &[about_build.upcast()])
}

/// "Check for updates automatically": the daily check at launch.
fn automatic_checks_row(page: &SettingsPage) -> SettingRow {
    let row = SettingRow::new(AUTOMATIC_CHECKS);
    let binding = PreferenceBinding {
        read: |preferences| preferences.update_checks,
        write: |on| PreferencesUpdate {
            update_checks: Some(on),
            ..PreferencesUpdate::default()
        },
    };
    row.add_control(&page.preference_switch(binding), ControlName::RowTitle);
    row
}

/// "Show the Update button in the command bar": on by default.
fn update_button_row(page: &SettingsPage) -> SettingRow {
    let row = SettingRow::new(UPDATE_BUTTON);
    let binding = PreferenceBinding {
        read: |preferences| preferences.show_update_button,
        write: |on| PreferencesUpdate {
            show_update_button: Some(on),
            ..PreferencesUpdate::default()
        },
    };
    row.add_control(&page.preference_switch(binding), ControlName::RowTitle);
    row
}

/// "Install updates without asking for a password": its switch is on
/// only while the polkit rule is there. Flipping it runs the update helper
/// through `pkexec`, which asks for an administrator's password; while it
/// runs the switch waits, and afterwards it shows the rule as it is, so a
/// cancelled prompt or a failure puts it back.
fn passwordless_row(page: &SettingsPage, setting: PasswordlessUpdates) -> SettingRow {
    let row = SettingRow::new(PASSWORDLESS_UPDATES);
    let switch = parts::switch();
    let setting = Rc::new(setting);
    show_passwordless(page, &switch, &setting);
    page.when_opened(glib::clone!(
        #[weak]
        page,
        #[weak]
        switch,
        #[strong]
        setting,
        move || {
            if switch.is_sensitive() {
                show_passwordless(&page, &switch, &setting);
            }
        }
    ));
    switch.connect_active_notify(glib::clone!(
        #[weak]
        page,
        move |switch| {
            if page.is_user_change() && switch.is_sensitive() {
                change_passwordless_from(&page, switch, &setting);
            }
        }
    ));
    row.add_control(&switch, ControlName::RowTitle);
    row
}

/// Shows on `switch` whether the rule is there now.
fn show_passwordless(page: &SettingsPage, switch: &gtk::Switch, setting: &PasswordlessUpdates) {
    let is_on = setting.is_on();
    page.while_showing_preferences(|| switch.set_active(is_on));
}

/// Turns the setting as `switch` now shows, then shows what happened.
fn change_passwordless_from(page: &SettingsPage, switch: &gtk::Switch, setting: &Rc<PasswordlessUpdates>) {
    let on = switch.is_active();
    let had_focus = switch.has_focus();
    switch.set_sensitive(false);
    let page = page.downgrade();
    let switch = switch.downgrade();
    let setting = Rc::clone(setting);
    glib::spawn_future_local(async move {
        let change = crate::update::change_passwordless(&setting, on).await;
        let (Some(page), Some(switch)) = (page.upgrade(), switch.upgrade()) else {
            return;
        };
        switch.set_sensitive(true);
        show_passwordless(&page, &switch, &setting);
        if had_focus {
            switch.grab_focus();
        }
        if change == PasswordlessChange::Failed {
            page.report(&ox_core::i18n::gettext(
                "Install updates without asking for a password could not be changed.",
            ));
        }
    });
}

/// The group `spec` describes, and its row.
fn action_group(spec: &ActionGroup) -> (SettingsGroup, SettingRow) {
    let group = SettingsGroup::new(&ox_core::i18n::gettext(spec.heading));
    let row = action_row(spec.text, spec.button, spec.action);
    group.add_row(&row);
    (group, row)
}

/// A row saying `text` whose button, labelled `button`, runs `action`.
fn action_row(text: RowText, button: &str, action: WindowAction) -> SettingRow {
    let row = SettingRow::new(text);
    let button = parts::button(&ox_core::i18n::gettext(button), ButtonStyle::Bordered);
    action.assign_to(&button);
    row.add_control(&button, ControlName::OwnLabel);
    row
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
