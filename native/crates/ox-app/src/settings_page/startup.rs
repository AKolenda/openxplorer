// SPDX-License-Identifier: AGPL-3.0-only
//! Windows & tabs ▸ Startup: where new windows open (TAB-055) and whether
//! a start reopens the last window's tabs (TAB-053).
//!
//! Ports Dolphin's Startup page ("Show on startup" with "Use Current
//! Location" and "Use Default Location", and "Folders, tabs, and window
//! state from last time") in Explorer's words ("Open File Explorer to",
//! "Restore previous folder windows").

use gtk::prelude::*;
use gtk::{gio, glib};
use ox_core::location::normalise_navigation;
use ox_core::settings::PreferencesUpdate;

use super::bindings::PreferenceBinding;
use super::group::SettingsGroup;
use super::parts;
use super::row::{ControlName, RowLayout, SettingRow};
use super::search::RowText;
use super::SettingsPage;
use crate::locations::{self, Page};
use crate::window::ButtonStyle;

const STARTUP_FOLDER: RowText = RowText {
    title: "Open new windows at",
    description: "The folder a new window shows first. Leave it empty for Home.",
    keywords: "startup start home folder location default show on startup open explorer to",
};

const RESTORE_SESSION: RowText = RowText {
    title: "Restore previous tabs at startup",
    description: "Starting OpenXplorer without a folder reopens the tabs and split panes the last \
                  window had.",
    keywords: "session remember reopen restore tabs last time startup logon",
};

/// The startup folder `typed` asks for, relative to `base`: its canonical
/// location, a landing page such as This PC, or an empty text for Home
/// when nothing is typed (Dolphin's "Use Default Location").
///
/// # Errors
///
/// What the message line says about a location the app cannot open, or a
/// local folder that does not exist.
pub(super) fn startup_folder_for(typed: &str, base: Option<&str>) -> Result<String, String> {
    let typed = typed.trim();
    if typed.is_empty() || locations::is_home_alias(typed) || typed.eq_ignore_ascii_case("home") {
        return Ok(String::new());
    }
    if let Some(page) = Page::from_title(typed) {
        return Ok(page.uri().to_owned());
    }
    let uri = normalise_navigation(typed, base, &glib::home_dir()).map_err(|error| error.to_string())?;
    if Page::from_uri(&uri) == Some(Page::Settings) {
        return Err("Settings cannot be the startup folder.".to_owned());
    }
    // A share is checked when it is listed, which may ask for a sign-in.
    let is_missing = gio::File::for_uri(&uri).path().is_some_and(|path| !path.is_dir());
    if is_missing {
        return Err(ox_core::i18n::format_message(
            "“{typed}” is not a folder that exists.",
            &[("typed", &(typed).to_string())],
        ));
    }
    Ok(uri)
}

/// The startup folder as the field shows it: a path, a page's title or
/// the location of a share; empty for Home.
fn shown_folder(uri: Option<&str>) -> String {
    let Some(uri) = uri else {
        return String::new();
    };
    match Page::from_uri(uri) {
        Some(page) => page.title().to_owned(),
        None => gio::File::for_uri(uri).parse_name().to_string(),
    }
}

/// The Startup group.
pub(super) fn startup_group(page: &SettingsPage) -> SettingsGroup {
    let group = SettingsGroup::new(&ox_core::i18n::gettext("Startup"));
    group.add_row(&startup_folder_row(page));
    let restore = SettingRow::new(RESTORE_SESSION);
    let binding = PreferenceBinding {
        read: |preferences| preferences.restore_session,
        write: |on| PreferencesUpdate {
            restore_session: Some(on),
            ..PreferencesUpdate::default()
        },
    };
    restore.add_control(&page.preference_switch(binding), ControlName::RowTitle);
    group.add_row(&restore);
    group
}

/// "Open new windows at": the folder's field, "Use current location" and
/// "Use Home".
fn startup_folder_row(page: &SettingsPage) -> SettingRow {
    let row = SettingRow::new(STARTUP_FOLDER);
    let field = gtk::Entry::builder()
        .placeholder_text(ox_core::i18n::gettext("Home"))
        .hexpand(true)
        .width_chars(36)
        .build();
    field.update_property(&[gtk::accessible::Property::Label(STARTUP_FOLDER.title)]);
    page.follow_preferences(glib::clone!(
        #[weak]
        field,
        move |preferences| field.set_text(&shown_folder(preferences.startup_folder.as_deref()))
    ));
    field.connect_activate(glib::clone!(
        #[weak]
        page,
        move |field| page.choose_startup_folder(&field.text())
    ));
    row.add_control(&field, ControlName::RowTitle);
    let current = parts::button(
        &ox_core::i18n::gettext("Use current location"),
        ButtonStyle::Bordered,
    );
    current.connect_clicked(glib::clone!(
        #[weak]
        page,
        move |_| {
            if let Some(origin) = page.index_origin() {
                page.choose_startup_folder(&origin);
            }
        }
    ));
    row.add_control(&current, ControlName::OwnLabel);
    let home = parts::button(&ox_core::i18n::gettext("Use Home"), ButtonStyle::Bordered);
    home.connect_clicked(glib::clone!(
        #[weak]
        page,
        move |_| page.choose_startup_folder("")
    ));
    row.add_control(&home, ControlName::OwnLabel);
    row.set_roomy_layout(RowLayout::ControlsBelow);
    row
}

impl SettingsPage {
    /// Saves `typed` as the startup folder, or says why it cannot be one
    /// and shows the saved folder again.
    pub(super) fn choose_startup_folder(&self, typed: &str) {
        match startup_folder_for(typed, self.index_origin().as_deref()) {
            Ok(folder) => self.save_preferences(PreferencesUpdate {
                startup_folder: Some(folder),
                ..PreferencesUpdate::default()
            }),
            Err(message) => self.report(&message),
        }
        // The field shows what is saved, also after a refusal.
        self.show_current_preferences();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A folder that exists is kept as its location, nothing typed is
    /// Home, and a missing folder is refused with a message.
    ///
    /// parity: TAB-055
    #[test]
    fn only_a_folder_that_exists_becomes_the_startup_folder() {
        let folder = tempfile::tempdir().expect("a temporary folder");
        let path = folder.path().to_str().expect("a UTF-8 path");

        let chosen = startup_folder_for(path, None).expect("an existing folder");
        let missing = startup_folder_for(&format!("{path}/missing"), None);

        assert_eq!(gio::File::for_uri(&chosen).path().as_deref(), Some(folder.path()));
        assert_eq!(startup_folder_for("  ", None), Ok(String::new()));
        let message = missing.expect_err("a missing folder is refused");
        assert!(message.ends_with("is not a folder that exists."), "{message}");
    }
}
