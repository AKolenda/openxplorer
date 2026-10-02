// SPDX-License-Identifier: AGPL-3.0-only
//! Extract all… into the folder the dialog names (ARC-009, ARC-011), as
//! Windows Explorer extracts:
//!
//! - A folder that does not exist yet is created, with any missing
//!   parents, and receives the archive's contents. An archive holding one
//!   top-level folder of the same name gives that folder, not
//!   `tidewater/tidewater`.
//! - An existing folder (`Downloads`) receives the contents directly. The
//!   archive is unpacked into a private hidden folder inside it first,
//!   with every safety check of the extractor; then its items are moved
//!   in through the usual name-conflict question (Replace, Skip, Keep
//!   both), so nothing is replaced without asking. The private folder is
//!   removed afterwards, with whatever the user chose to skip.
//!
//! Either way the extraction shows in the transfer panel, and the result
//! is shown in the tab it started from if that is still in front, else
//! in a new tab, or its folder is listed again.

use gtk::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{gio, glib};
use ox_core::archive::{
    lift_same_named_folder, private_extraction_name, ArchiveError, ExtractedFolder, ExtractionRequest,
};
use ox_core::gio_node::GioNode;
use ox_core::location::{parent_location, validate_name};
use ox_core::ops::OperationSummary;
use ox_core::transfer::{Cancellation, Node, TransferMode};

use crate::archive_view::{
    extraction_failure_text, extraction_success_text, ArchiveTarget, ExtractionChoice, EXTRACTION_STOPPED,
};

use super::background_notice::Destination;
use super::file_ops::IncomingItems;
use super::session::TabId;
use super::transfer_panel::TransferKind;
use super::BrowserWindow;

/// Shown in the transfer panel while the archive is checked.
const PREPARING: &str = "Preparing extraction…";

/// The folder field names a file.
const NOT_A_FOLDER: &str = "A file with that name is already there. Choose a folder to extract to.";

/// The user cancelled the name-conflict question.
const NOTHING_ADDED: &str = "Extraction cancelled. Nothing was added to the folder.";

/// What the dialog's folder is now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Target {
    /// Nothing has the name yet: it is created.
    Missing,
    /// An existing folder: the contents go into it.
    Folder,
    /// A file or something else: refused.
    Other,
}

/// Looks `uri` up, following a symlinked folder as a folder.
async fn target_of(uri: &str) -> Result<Target, String> {
    let queried = gio::File::for_uri(uri)
        .query_info_future(
            gio::FILE_ATTRIBUTE_STANDARD_TYPE,
            gio::FileQueryInfoFlags::NONE,
            glib::Priority::DEFAULT,
        )
        .await;
    match queried {
        Ok(info) if info.file_type() == gio::FileType::Directory => Ok(Target::Folder),
        Ok(_) => Ok(Target::Other),
        Err(error) if error.matches(gio::IOErrorEnum::NotFound) => Ok(Target::Missing),
        Err(error) => Err(error.to_string()),
    }
}

/// The URIs of the items directly inside `folder`. Blocking.
fn children_of(folder: &str) -> Result<Vec<String>, glib::Error> {
    let folder = gio::File::for_uri(folder);
    let children = folder.enumerate_children(
        gio::FILE_ATTRIBUTE_STANDARD_NAME,
        gio::FileQueryInfoFlags::NOFOLLOW_SYMLINKS,
        gio::Cancellable::NONE,
    )?;
    let mut uris = Vec::new();
    while let Some(info) = children.next_file(gio::Cancellable::NONE)? {
        uris.push(folder.child(info.name()).uri().to_string());
    }
    Ok(uris)
}

/// Creates `folder` and its missing parents; an existing one is fine.
/// Blocking.
fn make_folder(folder: &str) -> Result<(), glib::Error> {
    match gio::File::for_uri(folder).make_directory_with_parents(gio::Cancellable::NONE) {
        Err(error) if !error.matches(gio::IOErrorEnum::Exists) => Err(error),
        _ => Ok(()),
    }
}

/// The last part of `uri`, decoded, as a name.
fn last_name(uri: &str) -> Option<String> {
    gio::File::for_uri(uri)
        .basename()
        .and_then(|name| name.to_str().map(str::to_owned))
}

impl BrowserWindow {
    /// Extracts `archive` where the user chose (see the module).
    pub(super) fn extract_archive(
        &self,
        archive: &ArchiveTarget,
        choice: ExtractionChoice,
        origin: Option<TabId>,
    ) {
        // The dialog may have stayed open while another write or an
        // update began.
        if !self.may_start_archive_operation() {
            return;
        }
        let archive = archive.clone();
        glib::spawn_future_local(glib::clone!(
            #[weak(rename_to = window)]
            self,
            async move {
                let result = match target_of(&choice.target_uri).await {
                    Ok(Target::Missing) => window.extract_into_new_folder(&archive, &choice).await,
                    Ok(Target::Folder) => window.extract_into_existing_folder(&archive, &choice).await,
                    Ok(Target::Other) => Err(NOT_A_FOLDER.to_owned()),
                    Err(message) => Err(message),
                };
                match result {
                    Ok(Some((shown, text))) => window.conclude_extraction(&shown, &choice, &text, origin),
                    Ok(None) => window.show_message(NOTHING_ADDED),
                    Err(text) => {
                        window.notify_if_in_background(
                            &OperationSummary::Report(text.clone()),
                            Destination::default(),
                        );
                        window.show_result_dialog(EXTRACTION_STOPPED, &text);
                    }
                }
            }
        ));
    }

    /// Creates the chosen folder and extracts into it. Returns the folder
    /// to show and the message.
    async fn extract_into_new_folder(
        &self,
        archive: &ArchiveTarget,
        choice: &ExtractionChoice,
    ) -> Result<Option<(String, String)>, String> {
        let target = &choice.target_uri;
        let parent = parent_location(target).ok_or_else(|| NOT_A_FOLDER.to_owned())?;
        let name = last_name(target).unwrap_or_default();
        let name = validate_name(&name)
            .map_err(|error| error.to_string())?
            .to_owned();
        let missing_parent = parent.clone();
        gio::spawn_blocking(move || make_folder(&missing_parent))
            .await
            .unwrap_or_else(|panic| std::panic::resume_unwind(panic))
            .map_err(|error| error.to_string())?;
        let request = ExtractionRequest {
            archive_uri: archive.uri.clone(),
            destination_uri: parent,
            folder_name: name,
        };
        let extracted = self
            .run_extraction(request, None)
            .await
            .map_err(|error| extraction_failure_text(&error))?;
        let lifted = gio::spawn_blocking(move || lift_same_named_folder(extracted))
            .await
            .unwrap_or_else(|panic| std::panic::resume_unwind(panic));
        Ok(Some((lifted.uri.clone(), extraction_success_text(&lifted))))
    }

    /// Extracts into a private folder inside the chosen one, then moves
    /// the items in with the name-conflict question. Returns the folder
    /// to show and the message, or `None` when the user cancelled.
    async fn extract_into_existing_folder(
        &self,
        archive: &ArchiveTarget,
        choice: &ExtractionChoice,
    ) -> Result<Option<(String, String)>, String> {
        let target = choice.target_uri.clone();
        let private = private_extraction_name().map_err(|error| error.to_string())?;
        let request = ExtractionRequest {
            archive_uri: archive.uri.clone(),
            destination_uri: target.clone(),
            folder_name: private,
        };
        let extracted = self
            .run_extraction(request, None)
            .await
            .map_err(|error| extraction_failure_text(&error))?;
        let staging = extracted.uri.clone();
        let listed = staging.clone();
        let items = gio::spawn_blocking(move || children_of(&listed))
            .await
            .unwrap_or_else(|panic| std::panic::resume_unwind(panic));
        let moved = match items {
            Ok(uris) if uris.is_empty() => Some(()),
            Ok(uris) => self
                .transfer_without_undo(IncomingItems {
                    mode: TransferMode::Move,
                    uris,
                    destination_folder: target.clone(),
                })
                .await
                .map(|_| ()),
            Err(error) => {
                self.discard_private_folder(staging).await;
                return Err(error.to_string());
            }
        };
        self.discard_private_folder(staging).await;
        Ok(moved.map(|()| (target.clone(), existing_folder_text(&extracted, &target))))
    }

    /// Removes the private folder of an extraction into an existing
    /// folder, with anything the user chose to skip. Never follows links.
    async fn discard_private_folder(&self, uri: String) {
        let removed = gio::spawn_blocking(move || GioNode::new(&uri).delete_staging(None))
            .await
            .unwrap_or_else(|panic| std::panic::resume_unwind(panic));
        if let Err(error) = removed {
            self.show_message(&format!(
                "Could not remove the extraction's temporary folder: {error}"
            ));
        }
    }

    /// Runs `request` with the transfer panel and the window's progress;
    /// only the members `selection` names, when it names some.
    pub(super) async fn run_extraction(
        &self,
        request: ExtractionRequest,
        selection: Option<&[String]>,
    ) -> Result<ExtractedFolder, ArchiveError> {
        let cancel = Cancellation::new();
        self.transfer_panel()
            .start(TransferKind::Archive, PREPARING, cancel.clone());
        self.update_archive_actions();
        let mut extractor = self
            .zip_extractor()
            .with_progress(self.operation_progress_sender());
        if let Some(members) = selection {
            extractor = extractor.with_selection(members);
        }
        let extracted = extractor.extract_in_background(request, cancel).await;
        self.finish_archive_operation();
        extracted
    }

    /// Shows `shown` (the extracted folder), or lists the chosen folder's
    /// parent again, then the message (ARC-011).
    fn conclude_extraction(&self, shown: &str, choice: &ExtractionChoice, text: &str, origin: Option<TabId>) {
        if choice.show_result && self.current_uri().as_deref() == Some(shown) {
            // Already in front: listing it again shows the new items.
            self.reload_tabs_showing(shown);
        } else if choice.show_result {
            let active = self.imp().session.borrow().active_id();
            let opened = if origin.is_some() && origin == active {
                self.navigate(shown)
            } else {
                self.add_tab(shown)
            };
            if let Err(error) = opened {
                self.show_message(&error.to_string());
            }
        } else {
            let listed = parent_location(shown).unwrap_or_else(|| shown.to_owned());
            self.reload_tabs_showing(&listed);
            self.reload_tabs_showing(shown);
        }
        let destination = Destination::items(vec![shown.to_owned()]);
        self.notify_if_in_background(&OperationSummary::Toast(text.to_owned()), destination);
        // Listing a folder hides the toast, so it comes last.
        self.show_message(text);
    }
}

/// "Extracted 63 files into Downloads." for an extraction into the
/// existing folder `target`.
fn existing_folder_text(extracted: &ExtractedFolder, target: &str) -> String {
    let name = last_name(target).unwrap_or_else(|| target.to_owned());
    let folder = ExtractedFolder {
        name,
        ..extracted.clone()
    };
    extraction_success_text(&folder)
}
