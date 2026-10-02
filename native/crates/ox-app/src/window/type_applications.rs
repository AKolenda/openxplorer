// SPDX-License-Identifier: AGPL-3.0-only
//! The "Apps for this type" dialog of a file's Properties (OPEN-026), as
//! Dolphin's file-type options button opens: every application associated
//! with the file's type, which can be made the default or, when the user
//! added it, removed; any other application can be added, and the type
//! reset to the system's associations. The changes are the user's own
//! ([`crate::integration::change_type`]).

use gtk::{gio, glib};

use super::BrowserWindow;
use super::ButtonStyle;
use crate::dialog::Dialog;
use crate::integration::{change_type, other_applications, type_applications, TypeApplication, TypeChange};

/// Why Remove did nothing.
const NOT_ADDED: &str = crate::i18n::message_id("Only applications you added to this type can be removed.");

/// How an associated application is listed.
fn application_label(application: &TypeApplication) -> String {
    match (application.is_default, application.is_added) {
        (true, _) => {
            ox_core::i18n::format_message("{name} (default)", &[("name", &(application.name).to_string())])
        }
        (false, true) => ox_core::i18n::format_message(
            "{name} (added by you)",
            &[("name", &(application.name).to_string())],
        ),
        (false, false) => application.name.clone(),
    }
}

/// The item of `dropdown` chosen now, as an index into what it lists.
fn chosen(dropdown: &gtk::DropDown) -> Option<usize> {
    usize::try_from(dropdown.selected())
        .ok()
        .filter(|_| dropdown.selected() != gtk::INVALID_LIST_POSITION)
}

/// Asks over `parent` before the applications of a type described as
/// `description` go back to the system's; true when the user agreed.
async fn confirm_reset(parent: &Dialog, description: &str) -> bool {
    let message = ox_core::i18n::format_message("Reset the apps for {description} files to the system defaults? Your default app and the apps you added are forgotten.", &[("description", &(description).to_string())]);
    let question = Dialog::new(parent, &ox_core::i18n::gettext("Reset apps"), &message);
    question.add_cancel_button();
    let reset = question.add_button(&ox_core::i18n::gettext("Reset"), ButtonStyle::Accent);
    question.open();
    let answer = question.next_response().await;
    question.finish();
    answer == Some(reset)
}

impl BrowserWindow {
    /// Shows the applications of `content_type` and applies each change
    /// the user makes until the dialog is closed.
    pub(super) fn manage_type_applications(&self, content_type: &str) {
        let content_type = content_type.to_owned();
        glib::spawn_future_local(glib::clone!(
            #[weak(rename_to = window)]
            self,
            async move { window.run_type_applications(&content_type).await }
        ));
    }

    async fn run_type_applications(&self, content_type: &str) {
        let description = gio::content_type_get_description(content_type);
        let message = ox_core::i18n::format_message(
            "What opens {description} files ({content_type}). Changes apply to your account only.",
            &[
                ("description", &(description).to_string()),
                ("content_type", &(content_type).to_string()),
            ],
        );
        let dialog = Dialog::new(self, &ox_core::i18n::gettext("Apps for this type"), &message);
        let associated = gtk::DropDown::from_strings(&[]);
        dialog.add_labelled("Associated apps", &associated);
        let others = gtk::DropDown::from_strings(&[]);
        dialog.add_labelled("Another app", &others);
        let reset = dialog.add_button(&ox_core::i18n::gettext("Reset"), ButtonStyle::Bordered);
        let remove = dialog.add_button(&ox_core::i18n::gettext("Remove"), ButtonStyle::Bordered);
        let add = dialog.add_button(&ox_core::i18n::gettext("Add"), ButtonStyle::Bordered);
        let make_default = dialog.add_button(&ox_core::i18n::gettext("Set as default"), ButtonStyle::Accent);
        let close = dialog.add_button(&ox_core::i18n::gettext("Close"), ButtonStyle::Bordered);
        let mut is_open = false;
        loop {
            let shown = type_applications(content_type);
            let addable = other_applications(content_type);
            let labels: Vec<String> = shown.iter().map(application_label).collect();
            let names: Vec<&str> = addable.iter().map(|(_, name)| name.as_str()).collect();
            associated.set_model(Some(&gtk::StringList::new(
                &labels.iter().map(String::as_str).collect::<Vec<_>>(),
            )));
            others.set_model(Some(&gtk::StringList::new(&names)));
            if !is_open {
                // Not Reset, the first button: Enter must never reset.
                dialog.open_focusing(&associated);
                is_open = true;
            }
            let Some(answer) = dialog.next_response().await else {
                return;
            };
            let selected = chosen(&associated).and_then(|index| shown.get(index));
            let change = if answer == close {
                break;
            } else if answer == reset {
                if !confirm_reset(&dialog, &description).await {
                    continue;
                }
                Ok(TypeChange::Reset)
            } else if answer == make_default {
                selected
                    .map(|application| TypeChange::SetDefault(application.id.clone()))
                    .ok_or_else(String::new)
            } else if answer == remove {
                match selected {
                    Some(application) if application.is_added => {
                        Ok(TypeChange::Remove(application.id.clone()))
                    }
                    _ => Err(ox_core::i18n::gettext_static(NOT_ADDED).to_owned()),
                }
            } else if answer == add {
                let other = chosen(&others).and_then(|index| addable.get(index));
                other
                    .map(|(id, _)| TypeChange::Add(id.clone()))
                    .ok_or_else(String::new)
            } else {
                continue;
            };
            match change.and_then(|change| change_type(content_type, &change)) {
                Ok(()) => dialog.hide_error(),
                Err(message) if message.is_empty() => {}
                Err(message) => dialog.show_error(&message),
            }
        }
        dialog.finish();
    }
}
