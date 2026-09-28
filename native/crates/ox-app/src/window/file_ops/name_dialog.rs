// SPDX-License-Identifier: AGPL-3.0-only
//! The dialog that asks for a name: New folder and Rename (OPS-001,
//! OPS-006, OPS-009).
//!
//! Ports `nameDialog` in `desktop/ui/app.js`: the title, "Names must not
//! contain slashes.", a Name field and Cancel and Save. Save checks the
//! name ([`check_typed_name`]), then tries the operation; a refusal, such
//! as a taken name, appears inside the dialog, which stays open for
//! another try. For a file, Rename selects the name without its extension,
//! as Windows and Dolphin do (OPS-010); the Python app selected all of it.

use std::future::Future;

use gtk::prelude::*;

use super::names::check_typed_name;
use crate::window::dialog::{ButtonStyle, Dialog};
use crate::window::BrowserWindow;

/// The line under the title (`nameDialog`).
const NAME_HINT: &str = "Names must not contain slashes.";

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
}

/// The number of characters of `name` before its extension: up to its last
/// dot, unless the name starts with its only dot (`.bashrc`) or has none.
pub(super) fn stem_length(name: &str) -> usize {
    match name.rfind('.') {
        Some(dot) if dot > 0 => name[..dot].chars().count(),
        _ => name.chars().count(),
    }
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
    Attempt: FnMut(String) -> Outcome,
    Outcome: Future<Output = Result<T, String>>,
{
    let dialog = Dialog::new(window, request.title, NAME_HINT);
    let field = dialog.add_text_field("Name", request.initial_name);
    dialog.add_cancel_button();
    dialog.add_button("Save", ButtonStyle::Primary);
    dialog.open();
    if request.selection == NameSelection::Stem {
        let stem = i32::try_from(stem_length(request.initial_name)).unwrap_or(-1);
        field.select_region(0, stem);
    }
    loop {
        dialog.next_response().await?;
        let typed = field.text().to_string();
        let name = match check_typed_name(&typed) {
            Ok(name) => name.to_owned(),
            Err(invalid) => {
                dialog.show_error(&invalid.to_string());
                continue;
            }
        };
        dialog.set_busy(true);
        let outcome = attempt(name).await;
        dialog.set_busy(false);
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
