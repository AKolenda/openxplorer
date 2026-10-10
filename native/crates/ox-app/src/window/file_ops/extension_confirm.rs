// SPDX-License-Identifier: AGPL-3.0-only
//! Asking before a rename changes a file's extension (OPS-049).
//!
//! Windows Explorer asks "If you change a file name extension, the file
//! might become unusable. Are you sure you want to change it?" when a
//! rename adds, removes or changes the part after a file's last dot, since
//! apps and the desktop pick how to open a file by it. Folders have no
//! extension, so renaming one never asks, and changing only the case of an
//! extension (`.JPG` to `.jpg`) does not either.

use crate::dialog::Dialog;
use crate::window::BrowserWindow;
use crate::window::ButtonStyle;

/// The question's title.
const TITLE: &str = crate::i18n::message_id("Change the extension?");

/// The question, in Windows Explorer's words.
const MESSAGE: &str = crate::i18n::message_id(
    "If you change a file name extension, the file might become unusable. Are you sure you \
                                 want to change it?",
);

/// The extension of the file name `name`, case-folded: what follows its
/// last dot, unless that dot is a leading one (`.bashrc` has none) or
/// nothing follows it. Folding upper case then lower case matches letters
/// that differ only in case as Unicode case folding does, such as `ſ` and
/// `S` or the Kelvin sign and `k`, which lower case alone keeps apart.
fn extension(name: &str) -> Option<String> {
    let dot = name.rfind('.')?;
    let is_leading = name[..dot].chars().all(|character| character == '.');
    let extension = &name[dot + 1..];
    (!is_leading && !extension.is_empty()).then(|| extension.to_uppercase().to_lowercase())
}

/// True when renaming the file `old_name` to `new_name` adds, removes or
/// changes its extension; never for a folder.
fn changes_extension(old_name: &str, new_name: &str, is_folder: bool) -> bool {
    !is_folder && extension(old_name) != extension(new_name)
}

impl BrowserWindow {
    /// True when the rename of `old_name` to `new_name` may go ahead: it
    /// keeps the file's extension, or the user confirmed the change.
    pub(crate) async fn confirm_extension_change(
        &self,
        old_name: &str,
        new_name: &str,
        is_folder: bool,
    ) -> bool {
        if !changes_extension(old_name, new_name, is_folder) {
            return true;
        }
        let dialog = Dialog::new(
            self,
            ox_core::i18n::gettext_static(TITLE),
            ox_core::i18n::gettext_static(MESSAGE),
        );
        dialog.add_cancel_button();
        dialog.add_button(
            ox_core::i18n::gettext_static("Change extension"),
            ButtonStyle::Accent,
        );
        dialog.open();
        let answer = dialog.next_response().await;
        dialog.finish();
        answer.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// parity: OPS-049
    #[test]
    fn adding_removing_or_changing_a_files_extension_asks() {
        assert!(changes_extension("report.txt", "report.pdf", false));
        assert!(changes_extension("report", "report.txt", false), "added");
        assert!(changes_extension("report.txt", "report", false), "removed");
        assert!(changes_extension("archive.tar.gz", "archive.tar", false));
        assert!(changes_extension(".bashrc", ".bashrc.old", false));
    }

    /// parity: OPS-049
    #[test]
    fn keeping_the_extension_or_renaming_a_folder_does_not_ask() {
        assert!(!changes_extension("report.txt", "summary.txt", false));
        assert!(
            !changes_extension("photo.JPG", "photo.jpg", false),
            "only its case"
        );
        assert!(!changes_extension("Makefile", "GNUmakefile", false));
        assert!(
            !changes_extension(".bashrc", ".profile", false),
            "a leading dot is no extension"
        );
        assert!(
            !changes_extension("notes", "notes.", false),
            "nothing after the dot"
        );
        assert!(
            !changes_extension("Photos", "Photos.2024", true),
            "folders have no extension"
        );
    }

    /// parity: OPS-049
    #[test]
    fn a_case_change_that_only_case_folding_sees_does_not_ask() {
        assert!(
            !changes_extension("report.\u{17f}", "report.S", false),
            "long s and S"
        );
        assert!(
            !changes_extension("photo.\u{212a}", "photo.k", false),
            "Kelvin sign and k"
        );
        assert!(changes_extension("report.s", "report.t", false));
    }
}
