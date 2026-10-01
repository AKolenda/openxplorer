// SPDX-License-Identifier: AGPL-3.0-only
//! The "Apps for this type" dialog of a file's Properties (OPEN-026), as
//! Dolphin's file-type options button opens: every application associated
//! with the file's type, which can be made the default or, when the user
//! added it, removed; any other application can be added, and the type
//! reset to the system's associations. The changes are the user's own
//! ([`crate::integration::change_type`]).

use gtk::{gio, glib};

use super::dialog::{ButtonStyle, Dialog};
use super::BrowserWindow;
use crate::integration::{change_type, other_applications, type_applications, TypeApplication, TypeChange};

/// Why Remove did nothing.
const NOT_ADDED: &str = "Only applications you added to this type can be removed.";

/// How an associated application is listed.
fn application_label(application: &TypeApplication) -> String {
    match (application.is_default, application.is_added) {
        (true, _) => format!("{} (default)", application.name),
        (false, true) => format!("{} (added by you)", application.name),
        (false, false) => application.name.clone(),
    }
}

/// The item of `dropdown` chosen now, as an index into what it lists.
fn chosen(dropdown: &gtk::DropDown) -> Option<usize> {
    usize::try_from(dropdown.selected())
        .ok()
        .filter(|_| dropdown.selected() != gtk::INVALID_LIST_POSITION)
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
        let message =
            format!("What opens {description} files ({content_type}). Changes apply to your account only.");
        let dialog = Dialog::new(self, "Apps for this type", &message);
        let associated = gtk::DropDown::from_strings(&[]);
        dialog.add_labelled("Associated apps", &associated);
        let others = gtk::DropDown::from_strings(&[]);
        dialog.add_labelled("Another app", &others);
        let reset = dialog.add_button("Reset", ButtonStyle::Standard);
        let remove = dialog.add_button("Remove", ButtonStyle::Standard);
        let add = dialog.add_button("Add", ButtonStyle::Standard);
        let make_default = dialog.add_button("Set as default", ButtonStyle::Primary);
        let close = dialog.add_button("Close", ButtonStyle::Standard);
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
                dialog.open_on_first_button();
                is_open = true;
            }
            let Some(answer) = dialog.next_response().await else {
                return;
            };
            let selected = chosen(&associated).and_then(|index| shown.get(index));
            let change = if answer == close {
                break;
            } else if answer == reset {
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
                    _ => Err(NOT_ADDED.to_owned()),
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
