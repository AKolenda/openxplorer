// SPDX-License-Identifier: AGPL-3.0-only
//! General > "Back up settings": Export settings… saves the app's
//! settings to one file, and Import settings… replaces them with a file's,
//! to carry them to another computer.
//!
//! What the file holds, and what it leaves on this computer, is decided in
//! ox-core ([`ox_core::settings::backup`]). Both buttons choose the file in
//! the desktop's file dialog (the portal's, where there is one); nothing is
//! read or written until a file is chosen, and an import asks first.

use gtk::prelude::*;
use gtk::{gio, glib};
use ox_core::settings::backup;

use super::group::SettingsGroup;
use super::parts;
use super::row::{ControlName, SettingRow};
use super::search::RowText;
use super::SettingsPage;
use crate::dialog::Dialog;
use crate::window::ButtonStyle;

const EXPORT: RowText = RowText {
    title: "Export settings",
    description: "Saves your OpenXplorer settings in one file you can take to another computer: \
                  views, layout, confirmations, icons, Quick access pins and network folders. \
                  Passwords, history and window sizes stay here.",
    keywords: "export save backup back up transfer settings preferences json new computer migrate",
};

const IMPORT: RowText = RowText {
    title: "Import settings",
    description: "Replaces your OpenXplorer settings with the ones in a file saved with Export \
                  settings.",
    keywords: "import load backup back up transfer settings preferences json new computer migrate",
};

/// The most folders and files a message names after an import; the rest
/// are counted.
const MAX_NAMED: usize = 3;

/// "Back up settings": Export settings… and Import settings….
pub(super) fn group(page: &SettingsPage) -> SettingsGroup {
    let group = SettingsGroup::new(&ox_core::i18n::gettext("Back up settings"));
    let export = SettingRow::new(EXPORT);
    let button = parts::button(&ox_core::i18n::gettext("Export settings…"), ButtonStyle::Bordered);
    button.connect_clicked(glib::clone!(
        #[weak]
        page,
        move |_| choose_export_file(&page)
    ));
    export.add_control(&button, ControlName::OwnLabel);
    group.add_row(&export);
    let import = SettingRow::new(IMPORT);
    let button = parts::button(&ox_core::i18n::gettext("Import settings…"), ButtonStyle::Bordered);
    button.connect_clicked(glib::clone!(
        #[weak]
        page,
        move |_| choose_import_file(&page)
    ));
    import.add_control(&button, ControlName::OwnLabel);
    group.add_row(&import);
    group
}

/// The file dialog's filter: JSON files.
fn json_filters() -> (gio::ListStore, gtk::FileFilter) {
    let filter = gtk::FileFilter::new();
    filter.set_name(Some(&ox_core::i18n::gettext("Settings files (JSON)")));
    filter.add_mime_type("application/json");
    filter.add_pattern("*.json");
    let filters = gio::ListStore::new::<gtk::FileFilter>();
    filters.append(&filter);
    (filters, filter)
}

/// The name of `file` as a message shows it.
fn shown_name(file: &gio::File) -> String {
    file.basename()
        .map_or_else(|| file.uri().to_string(), |name| name.display().to_string())
}

/// Asks where to save the settings, then saves them there.
fn choose_export_file(page: &SettingsPage) {
    let (filters, filter) = json_filters();
    let picker = gtk::FileDialog::builder()
        .title(ox_core::i18n::gettext("Export settings"))
        .modal(true)
        .filters(&filters)
        .default_filter(&filter)
        .initial_name(backup::SUGGESTED_FILE_NAME)
        .build();
    let window = page.root().and_downcast::<gtk::Window>();
    picker.save(
        window.as_ref(),
        gio::Cancellable::NONE,
        glib::clone!(
            #[weak]
            page,
            move |chosen| {
                if let Ok(file) = chosen {
                    export_to(&page, file);
                }
            }
        ),
    );
}

/// Saves the settings as they are now into `file`, replacing it.
pub(super) fn export_to(page: &SettingsPage, file: gio::File) {
    let text = backup::export_text(&page.context().settings_data());
    let page = page.downgrade();
    glib::spawn_future_local(async move {
        let written = file
            .replace_contents_future(text.into_bytes(), None, false, gio::FileCreateFlags::PRIVATE)
            .await;
        let Some(page) = page.upgrade() else {
            return;
        };
        let name = shown_name(&file);
        let message = match written {
            Ok(_) => ox_core::i18n::format_message("Settings exported to {name}.", &[("name", &name)]),
            Err((_, error)) => ox_core::i18n::format_message(
                "Could not export settings to {name}: {error}",
                &[("name", &name), ("error", &error.to_string())],
            ),
        };
        page.report(&message);
    });
}

/// Asks for a settings file, then whether to replace the settings with
/// it.
fn choose_import_file(page: &SettingsPage) {
    let (filters, filter) = json_filters();
    let picker = gtk::FileDialog::builder()
        .title(ox_core::i18n::gettext("Import settings"))
        .modal(true)
        .filters(&filters)
        .default_filter(&filter)
        .build();
    let window = page.root().and_downcast::<gtk::Window>();
    picker.open(
        window.as_ref(),
        gio::Cancellable::NONE,
        glib::clone!(
            #[weak]
            page,
            move |chosen| {
                if let Ok(file) = chosen {
                    import_from(&page, file);
                }
            }
        ),
    );
}

/// Reads `file`, asks whether to replace the settings with the ones in
/// it, and on Replace replaces them. A file too large, unreadable or not
/// a settings file is reported without asking, and nothing changes.
pub(super) fn import_from(page: &SettingsPage, file: gio::File) {
    let page = page.downgrade();
    glib::spawn_future_local(async move {
        let text = read_settings_file(&file).await;
        let Some(page) = page.upgrade() else {
            return;
        };
        let name = shown_name(&file);
        // A file that cannot be imported says so before anything is asked.
        let checked = text.and_then(|text| {
            backup::check(&text).map_err(|error| error.to_string())?;
            Ok(text)
        });
        match checked {
            Ok(text) => ask_to_replace(&page, &name, text),
            Err(error) => page.report(&ox_core::i18n::format_message(
                "Could not import settings from {name}: {error}",
                &[("name", &name), ("error", &error)],
            )),
        }
    });
}

/// The text of `file`, which may be no larger than a settings file.
async fn read_settings_file(file: &gio::File) -> Result<String, String> {
    let info = file
        .query_info_future(
            gio::FILE_ATTRIBUTE_STANDARD_SIZE,
            gio::FileQueryInfoFlags::NONE,
            glib::Priority::DEFAULT,
        )
        .await
        .map_err(|error| error.to_string())?;
    let too_large = || ox_core::i18n::gettext("This file is too large to be a settings file.");
    if u64::try_from(info.size()).map_or(true, |size| size > backup::SIZE_LIMIT) {
        return Err(too_large());
    }
    let (bytes, _) = file
        .load_contents_future()
        .await
        .map_err(|error| error.to_string())?;
    if bytes.len() as u64 > backup::SIZE_LIMIT {
        return Err(too_large());
    }
    String::from_utf8(bytes.to_vec())
        .map_err(|_| ox_core::i18n::gettext("This is not an OpenXplorer settings file. Nothing was changed."))
}

/// Asks whether to replace the settings with those of `text`, read from
/// the file `name`; Replace replaces them.
fn ask_to_replace(page: &SettingsPage, name: &str, text: String) {
    let Some(window) = page.root().and_downcast::<gtk::Window>() else {
        return;
    };
    let question = ox_core::i18n::format_message(
        "Replace your OpenXplorer settings with the ones in {name}?",
        &[("name", name)],
    );
    let dialog = Dialog::new(&window, &ox_core::i18n::gettext("Import settings"), &question);
    let replace = dialog.add_button(&ox_core::i18n::gettext("Replace"), ButtonStyle::Accent);
    dialog.add_cancel_button();
    dialog.open();
    let page = page.downgrade();
    let name = name.to_owned();
    glib::spawn_future_local(async move {
        let answer = dialog.next_response().await;
        dialog.finish();
        if answer != Some(replace) {
            return;
        }
        if let Some(page) = page.upgrade() {
            replace_settings(&page, &name, text);
        }
    });
}

/// Replaces the settings with those of `text`, from the file `name`, and
/// says how it went.
fn replace_settings(page: &SettingsPage, name: &str, text: String) {
    let weak = page.downgrade();
    let name = name.to_owned();
    page.context().import_settings(text, move |result| {
        let Some(page) = weak.upgrade() else {
            return;
        };
        page.show_current_preferences();
        let message = match result {
            Ok(missing) => imported_message(&name, &missing),
            Err(error) => ox_core::i18n::format_message(
                "Could not import settings from {name}: {error}",
                &[("name", &name), ("error", &error.to_string())],
            ),
        };
        page.report(&message);
    });
}

/// What the message line says once the settings of `name` are in place,
/// naming the folders and files they name that are not on this computer.
fn imported_message(name: &str, missing: &[String]) -> String {
    if missing.is_empty() {
        return ox_core::i18n::format_message("Settings imported from {name}.", &[("name", name)]);
    }
    let mut shown: Vec<&str> = missing.iter().take(MAX_NAMED).map(String::as_str).collect();
    let more;
    if missing.len() > MAX_NAMED {
        more = ox_core::i18n::format_message(
            "{count} more",
            &[("count", &(missing.len() - MAX_NAMED).to_string())],
        );
        shown.push(&more);
    }
    ox_core::i18n::format_message(
        "Settings imported from {name}. Not on this computer, kept as they are: {paths}.",
        &[("name", name), ("paths", &shown.join(", "))],
    )
}

#[cfg(test)]
mod tests {
    use super::imported_message;

    /// The message names up to three missing folders and counts the rest.
    #[test]
    fn the_message_names_what_is_not_on_this_computer() {
        assert_eq!(imported_message("a.json", &[]), "Settings imported from a.json.");
        let missing: Vec<String> = ["/a", "/b", "/c", "/d", "/e"].map(str::to_owned).into();
        assert_eq!(
            imported_message("a.json", &missing),
            "Settings imported from a.json. Not on this computer, kept as they are: /a, /b, /c, 2 more."
        );
    }
}
