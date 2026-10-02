// SPDX-License-Identifier: AGPL-3.0-only
//! The dialog that asks for a name: New folder and Rename (OPS-001,
//! OPS-006, OPS-009).
//!
//! Ports `nameDialog` in `v2.0.0:desktop/ui/app.js`: the title, "Names must not
//! contain slashes.", a Name field and Cancel and Save. Save checks the
//! name ([`check_typed_name`]), then tries the operation; a refusal, such
//! as a taken name, appears inside the dialog, which stays open for
//! another try. For a file, Rename selects the name without its extension,
//! as Windows and Dolphin do (OPS-010); the Python app selected all of it.
//! While the user types, a line under the field warns, as Dolphin's New
//! folder dialog does, about a taken name, a leading dot that hides the
//! item, and a leading space or tilde (OPS-007). New folder takes slashes,
//! as Dolphin's does: `Photos/2026` makes a folder inside a folder, and
//! the line names the folders it will make before Save.

use std::future::Future;

use gtk::prelude::*;
use gtk::{gio, glib};
use ox_core::transfer::Cancellation;

use super::names::{check_folder_path, check_typed_name, folder_path_preview, name_warning};
use crate::dialog::Dialog;
use crate::window::BrowserWindow;
use crate::window::ButtonStyle;

/// The line under the title (`nameDialog`).
const NAME_HINT: &str = crate::i18n::message_id("Names must not contain slashes.");

/// The line under New folder's title, which takes slashes (OPS-007).
const FOLDER_PATH_HINT: &str = crate::i18n::message_id("A slash makes a folder inside the one before it.");

/// How much of the name the field selects when the dialog opens.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum NameSelection {
    /// All of it: a new folder's name, or a folder being renamed.
    Whole,
    /// The name without its extension: a file being renamed.
    Stem,
}

/// What the dialog shows when it opens.
#[derive(Debug, Clone, Copy)]
pub(super) struct NameRequest<'a> {
    /// The dialog's title, such as "Rename".
    pub(super) title: &'a str,
    /// The name the field starts with.
    pub(super) initial_name: &'a str,
    /// How much of it is selected.
    pub(super) selection: NameSelection,
    /// The folder the name is for, which the dialog checks each typed
    /// name against while the user types.
    pub(super) folder: &'a str,
    /// Whether slashes make folders inside folders (New folder).
    pub(super) takes_folder_path: bool,
}

/// Why a dialog that takes slashes refuses a name.
const INVALID_FOLDER_PATH: &str = crate::i18n::message_id(
    "Use names without backslashes or control characters, one slash between folders.",
);

/// Whether a name dialog takes `typed`, or why not: with
/// `takes_folder_path`, slashes make folders inside folders.
fn check_name(typed: &str, takes_folder_path: bool) -> Result<(), String> {
    if takes_folder_path {
        check_folder_path(typed)
            .map(drop)
            .map_err(|_| ox_core::i18n::gettext_static(INVALID_FOLDER_PATH).to_owned())
    } else {
        check_typed_name(typed)
            .map(drop)
            .map_err(|invalid| invalid.to_string())
    }
}

/// The number of characters of `name` before its extension: up to its last
/// dot, unless the name starts with its only dot (`.bashrc`) or has none.
pub(super) fn stem_length(name: &str) -> usize {
    match name.rfind('.') {
        Some(dot) if dot > 0 => name[..dot].chars().count(),
        _ => name.chars().count(),
    }
}

/// Shows under `field` what [`name_warning`] says about each name typed
/// there, checking whether it is taken in the request's folder; the
/// starting name, and a name refused anyway, are not warned about.
fn warn_while_typing(dialog: &Dialog, field: &gtk::Entry, request: &NameRequest<'_>) {
    let warning = dialog.add_hint("");
    warning.set_visible(false);
    let folder = gio::File::for_uri(request.folder);
    let initial_name = request.initial_name.to_owned();
    let takes_folder_path = request.takes_folder_path;
    field.connect_changed(move |field| {
        let typed = field.text().to_string();
        if typed == initial_name || check_name(&typed, takes_folder_path).is_err() {
            warning.set_visible(false);
            return;
        }
        let child = folder.resolve_relative_path(&typed);
        glib::spawn_future_local(glib::clone!(
            #[weak]
            field,
            #[weak]
            warning,
            async move {
                let taken = child
                    .query_info_future(
                        "standard::type",
                        gio::FileQueryInfoFlags::NOFOLLOW_SYMLINKS,
                        glib::Priority::DEFAULT,
                    )
                    .await
                    .is_ok();
                // A later change has its own check.
                if field.text() != typed {
                    return;
                }
                let path_preview = takes_folder_path && !taken;
                let preview = check_folder_path(&typed)
                    .ok()
                    .and_then(|names| folder_path_preview(&names));
                let text = preview
                    .filter(|_| path_preview)
                    .or_else(|| name_warning(&typed, taken));
                warning.set_text(text.as_deref().unwrap_or_default());
                warning.set_visible(text.is_some());
            }
        ));
    });
}

/// Asks for a name and tries `attempt` with each one the user saves,
/// until an attempt succeeds (its value is returned) or the user cancels
/// (`None`). A failed attempt's message stays in the dialog.
pub(super) async fn ask_for_name<T, Attempt, Outcome>(
    window: &BrowserWindow,
    request: NameRequest<'_>,
    mut attempt: Attempt,
) -> Option<T>
where
    Attempt: FnMut(String, Cancellation) -> Outcome,
    Outcome: Future<Output = Result<T, String>>,
{
    let hint = if request.takes_folder_path {
        ox_core::i18n::gettext_static(FOLDER_PATH_HINT)
    } else {
        ox_core::i18n::gettext_static(NAME_HINT)
    };
    let dialog = Dialog::new(window, request.title, hint);
    let field = dialog.add_text_field(ox_core::i18n::gettext_static("Name"), request.initial_name);
    warn_while_typing(&dialog, &field, &request);
    dialog.add_cancel_button();
    dialog.add_button(ox_core::i18n::gettext_static("Save"), ButtonStyle::Accent);
    dialog.open();
    if request.selection == NameSelection::Stem {
        let stem = i32::try_from(stem_length(request.initial_name)).unwrap_or(-1);
        field.select_region(0, stem);
    }
    loop {
        dialog.next_response().await?;
        let typed = field.text().to_string();
        if let Err(message) = check_name(&typed, request.takes_folder_path) {
            dialog.show_error(&message);
            continue;
        }
        let name = typed;
        let running = Cancellation::new();
        dialog.set_busy(Some(&running));
        let outcome = attempt(name, running).await;
        dialog.set_busy(None);
        match outcome {
            Ok(value) => {
                dialog.finish();
                return Some(value);
            }
            Err(message) => dialog.show_error(&message),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One name and how many of its characters Rename selects.
    struct StemCase {
        name: &'static str,
        stem: usize,
    }

    /// parity: OPS-010
    #[test]
    fn rename_selects_the_name_before_its_last_extension() {
        let cases = [
            StemCase {
                name: "report.pdf",
                stem: 6,
            },
            StemCase {
                name: "archive.tar.gz",
                stem: 11,
            },
            StemCase {
                name: "Résumé.txt",
                stem: 6,
            },
            StemCase {
                name: ".bashrc",
                stem: 7,
            },
            StemCase {
                name: "Makefile",
                stem: 8,
            },
        ];
        for case in cases {
            assert_eq!(stem_length(case.name), case.stem, "{}", case.name);
        }
    }
}
