// SPDX-License-Identifier: AGPL-3.0-only
//! Moving a standard folder's files after the Location tab moved the
//! folder (PROP-017).
//!
//! The Python app never moved them. Windows 11 asks, after Apply, "Do you
//! want to move all of the files from the old location to the new
//! location?"; the native app asks the same once the new location is
//! applied, and moves the items ox-core allows
//! ([`FolderRelocation::contents_to_move`]) through the transfer engine,
//! as a paste of cut items would: the name-conflict dialog asks first,
//! items without an answer are skipped, and the transfer panel shows the
//! progress with Cancel. Safety rule "never lose files": nothing is
//! overwritten without a choice, and a skipped or failed item stays in
//! the old folder.
//!
//! [`FolderRelocation::contents_to_move`]: ox_core::folder_locations::FolderRelocation::contents_to_move

use std::path::{Path, PathBuf};

use gtk::subclass::prelude::*;
use ox_core::location::file_uri;
use ox_core::ops::TransferOutcome;
use ox_core::transfer::TransferMode;

use super::file_ops::IncomingItems;
use super::BrowserWindow;
use super::ButtonStyle;
use crate::dialog::Dialog;

/// The question's title.
const TITLE: &str = crate::i18n::message_id("Move files");
/// The question, in Windows 11's words.
const QUESTION: &str = crate::i18n::message_id(
    "Do you want to move all of the files from the old location to the new location?",
);
/// What happens to names that exist in both folders.
const CONFLICTS: &str = crate::i18n::message_id(
    "Items whose names already exist in the new location are asked about first. \
                         Nothing is replaced without your choice, and skipped items stay where they are.",
);
/// Said instead of asking while another file operation runs.
const BUSY: &str =
    crate::i18n::message_id("Finish the current file operation, then move the files from the old location.");

impl BrowserWindow {
    /// Asks whether to move `items` from `previous` into `destination`,
    /// then moves them. Returns the outcome of a move that ran; `None`
    /// when the user kept the files where they are or another operation
    /// runs.
    pub(crate) async fn offer_to_move_files(
        &self,
        previous: &Path,
        destination: &Path,
        items: Vec<PathBuf>,
    ) -> Option<TransferOutcome> {
        if items.is_empty() {
            return None;
        }
        if !self.imp().file_operations.borrow().is_idle() {
            self.show_message(ox_core::i18n::gettext_static(BUSY));
            return None;
        }
        if !self.asks_to_move(previous, destination).await {
            return None;
        }
        let incoming = IncomingItems {
            mode: TransferMode::Move,
            uris: items.iter().map(|item| file_uri(item)).collect(),
            destination_folder: file_uri(destination),
        };
        self.transfer_with_conflicts(incoming).await
    }

    /// Whether the user chose Move files; "Don't move", Escape and closing
    /// keep the files where they are.
    async fn asks_to_move(&self, previous: &Path, destination: &Path) -> bool {
        let dialog = Dialog::new(
            self,
            ox_core::i18n::gettext_static(TITLE),
            ox_core::i18n::gettext_static(QUESTION),
        );
        dialog.add_hint(&ox_core::i18n::format_message(
            "Old location: {display}",
            &[("display", &previous.display().to_string())],
        ));
        dialog.add_hint(&ox_core::i18n::format_message(
            "New location: {display}",
            &[("display", &destination.display().to_string())],
        ));
        dialog.add_note(ox_core::i18n::gettext_static(CONFLICTS));
        dialog.add_button(&ox_core::i18n::gettext("Don't move"), ButtonStyle::Bordered);
        let move_files = dialog.add_button(&ox_core::i18n::gettext("Move files"), ButtonStyle::Accent);
        dialog.open();
        let answer = dialog.next_response().await;
        dialog.finish();
        answer == Some(move_files)
    }
}
