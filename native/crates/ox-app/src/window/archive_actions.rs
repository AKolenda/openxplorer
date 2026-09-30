// SPDX-License-Identifier: AGPL-3.0-only
//! ZIP archives in the window: the archive browser, Extract all…, Extract
//! here and Compress to ZIP file.
//!
//! Ports `archiveDialog`, the `extractDialog` flow after its dialog and
//! the `isZipEntry` item of `entryMenu` in `desktop/ui/app.js`: opening a
//! ZIP browses it (ARC-002), Extract all… asks where (ARC-009) and runs
//! the extraction with the operation panel and Cancel (ARC-011), then
//! shows the result in the tab that asked, or a new tab. Extract here
//! (ARC-025) and Compress to ZIP file (ARC-023) come from the Dolphin
//! baseline. One archive operation runs at a time per window, and every
//! write goes through the previous-versions write guard (ARC-020).

use std::cell::OnceCell;
use std::rc::Rc;
use std::sync::Arc;

use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;
use ox_core::archive::{
    default_preview_root, ArchiveBrowser, ArchiveError, CompressionRequest, ExtractionRequest,
    GioArchiveOpener, GioExtractionOutput, ZipCompressor, ZipExtractor,
};
use ox_core::entry::Entry;
use ox_core::gio_node::GioNode;
use ox_core::integration::Activation;
use ox_core::location::{is_smb_server, parent_location};
use ox_core::ops::OperationSummary;
use ox_core::transfer::{Cancellation, Node, NodeFactory, Progress};
use ox_core::versions::snapshot_location;

use crate::archive_view::{
    archive_dialog, compressed_file_name, compression_failure_text, compression_success_text, extract_dialog,
    extraction_failure_text, extraction_success_text, unique_folder_names, ArchiveDialogActions,
    ArchiveTarget, ExtractDialogSetup, ExtractionChoice, OperationPanel, COMPRESSION_STOPPED,
    EXTRACTION_STOPPED,
};
use crate::locations::Page;

use super::actions::plain_action;
use super::session::TabId;
use super::window_action::WindowAction;
use super::BrowserWindow;

/// Shown when an archive operation is asked for while one runs.
const OPERATION_RUNNING: &str = "Finish the current file operation before extracting.";
/// The panel's first label of an extraction.
const PREPARING: &str = "Preparing extraction…";
/// The panel's first label of a compression.
const PREPARING_COMPRESSION: &str = "Preparing compression…";

/// The window's archive operation panel.
#[derive(Debug, Default)]
pub(super) struct ArchiveOperations {
    /// Floats over the folder pane; set by `install_archive_actions`.
    panel: OnceCell<OperationPanel>,
}

impl BrowserWindow {
    /// The panel of the running extraction, compression or restored copy.
    pub(super) fn operation_panel(&self) -> &OperationPanel {
        self.imp()
            .archive_operations
            .panel
            .get()
            .expect("BrowserWindow::new installs the operation panel")
    }

    /// Floats the operation panel over the folder pane and adds the
    /// archive actions.
    pub(super) fn install_archive_actions(&self) {
        let panel = OperationPanel::default();
        if let Some(overlay) = self.imp().toast.parent().and_downcast::<gtk::Overlay>() {
            overlay.add_overlay(&panel);
        }
        self.imp()
            .archive_operations
            .panel
            .set(panel)
            .expect("installed once");
        self.add_action_entries([
            plain_action(WindowAction::ExtractAll, |window| {
                if let Some(archive) = window.selected_archive() {
                    window.open_extract_dialog(&archive);
                }
            }),
            plain_action(WindowAction::ExtractHere, BrowserWindow::extract_here),
            plain_action(WindowAction::CompressToZip, BrowserWindow::compress_selection),
        ]);
    }

    /// Enables the archive commands for what is selected and where.
    pub(super) fn update_archive_actions(&self) {
        let is_idle = !self.is_writing_files();
        let has_archive = self.selected_archive().is_some();
        let folder_is_writable = self
            .current_uri()
            .is_some_and(|uri| self.is_writable_folder(&uri));
        let selected = self.folder_pane().model().summary().count;
        self.set_action_enabled(WindowAction::ExtractAll, is_idle && has_archive);
        self.set_action_enabled(
            WindowAction::ExtractHere,
            is_idle && has_archive && folder_is_writable,
        );
        let can_compress = is_idle && selected > 0 && folder_is_writable;
        self.set_action_enabled(WindowAction::CompressToZip, can_compress);
    }

    /// The one selected item, when it is a ZIP archive.
    fn selected_archive(&self) -> Option<ArchiveTarget> {
        let selected = self.folder_pane().model().selected_items();
        let [item] = selected.as_slice() else {
            return None;
        };
        archive_target(item.entry())
    }

    /// True for a folder the user may write into: not a page, a server
    /// listing or a previous version (`writableLocation`).
    fn is_writable_folder(&self, uri: &str) -> bool {
        is_writable_location(uri, &self.imp().locations.borrow().snapshot_roots)
    }

    /// Opens the ZIP `entry` in the archive browser (ARC-002, ARC-003).
    pub(super) fn open_archive(&self, entry: &Entry) {
        let Some(archive) = archive_target(entry) else {
            return;
        };
        let shown_path = self.imp().locations.borrow().display_location(&archive.uri);
        let browser = ArchiveBrowser::new(Arc::new(GioArchiveOpener), default_preview_root());
        let actions = ArchiveDialogActions {
            extract_all: Box::new(glib::clone!(
                #[weak(rename_to = window)]
                self,
                #[strong]
                archive,
                move || window.open_extract_dialog(&archive)
            )),
            open_externally: Box::new(glib::clone!(
                #[weak(rename_to = window)]
                self,
                #[strong(rename_to = uri)]
                archive.uri,
                move || window.open_externally(&uri)
            )),
            open_copy: Rc::new(glib::clone!(
                #[weak(rename_to = window)]
                self,
                move |uri: String| window.open_externally(&uri)
            )),
        };
        let frame = archive_dialog(&archive, browser, &shown_path, actions);
        self.present_window_dialog(&frame);
    }

    /// Opens `uri` in the desktop's application for its type: an archive
    /// in the archive manager, a member's private copy in its viewer.
    fn open_externally(&self, uri: &str) {
        let on_error = glib::clone!(
            #[weak(rename_to = window)]
            self,
            move |error: glib::Error| window.show_message(&error.to_string())
        );
        self.context().open_uri(uri, self.upcast_ref(), on_error);
    }

    /// True when no write runs in this window; otherwise says so, as one
    /// operation runs at a time (OPS-024).
    fn may_start_archive_operation(&self) -> bool {
        if self.is_writing_files() {
            self.show_message(OPERATION_RUNNING);
            return false;
        }
        true
    }

    /// Asks where to extract `archive` (Extract all…).
    fn open_extract_dialog(&self, archive: &ArchiveTarget) {
        if !self.may_start_archive_operation() {
            return;
        }
        let default_destination = self.default_extraction_folder(&archive.uri);
        let shown_destination = self
            .imp()
            .locations
            .borrow()
            .display_location(&default_destination);
        let roots = self.imp().locations.borrow().snapshot_roots.clone();
        let setup = ExtractDialogSetup {
            default_destination,
            shown_destination,
            inspector: self.zip_extractor(),
            is_writable: Box::new(move |uri| is_writable_location(uri, &roots)),
        };
        let origin = self.imp().session.borrow().active_id();
        let external_uri = archive.uri.clone();
        let frame = extract_dialog(
            archive,
            setup,
            glib::clone!(
                #[weak(rename_to = window)]
                self,
                #[strong]
                archive,
                move |choice| window.extract_archive(&archive, choice, origin)
            ),
            glib::clone!(
                #[weak(rename_to = window)]
                self,
                move || window.open_externally(&external_uri)
            ),
        );
        self.present_window_dialog(&frame);
    }

    /// The folder Extract all… suggests: the archive's own folder when it
    /// is writable, else Downloads, else the home folder.
    fn default_extraction_folder(&self, archive_uri: &str) -> String {
        let parent = parent_location(archive_uri).filter(|parent| self.is_writable_folder(parent));
        if let Some(parent) = parent {
            return parent;
        }
        let downloads = self
            .context()
            .known_folders()
            .into_iter()
            .find(|place| place.known_folder == Some(ox_core::places::KnownFolder::Downloads));
        downloads.map_or_else(|| self.imp().locations.borrow().home_uri(), |place| place.uri)
    }

    /// An extractor over GIO that the write guard protects (ARC-020).
    fn zip_extractor(&self) -> ZipExtractor {
        let factory: NodeFactory = Arc::new(|uri: &str| Ok(Box::new(GioNode::new(uri)) as Box<dyn Node>));
        ZipExtractor::new(Arc::new(GioArchiveOpener), factory, Arc::new(GioExtractionOutput))
            .with_write_guard(self.context().previous_versions().write_guard())
    }

    /// Extracts `archive` as the user chose, with the operation panel, then
    /// shows the result in `origin` if it is still in front, else in a new
    /// tab, or lists the destination again (ARC-011).
    fn extract_archive(&self, archive: &ArchiveTarget, choice: ExtractionChoice, origin: Option<TabId>) {
        let cancel = Cancellation::new();
        self.operation_panel().start(PREPARING, cancel.clone());
        self.update_archive_actions();
        let request = ExtractionRequest {
            archive_uri: archive.uri.clone(),
            destination_uri: choice.destination_uri.clone(),
            folder_name: choice.folder_name.clone(),
        };
        let extractor = self
            .zip_extractor()
            .with_progress(self.operation_progress_sender());
        glib::spawn_future_local(glib::clone!(
            #[weak(rename_to = window)]
            self,
            async move {
                let extracted = extractor.extract_in_background(request, cancel).await;
                window.finish_archive_operation();
                match extracted {
                    Ok(folder) => {
                        // Listing a folder hides the toast, so it comes last.
                        window.show_extracted(&folder.uri, &choice, origin);
                        let text = extraction_success_text(&folder);
                        window.notify_if_in_background(&OperationSummary::Toast(text.clone()));
                        window.show_message(&text);
                    }
                    Err(error) => {
                        let text = extraction_failure_text(&error);
                        window.notify_if_in_background(&OperationSummary::Report(text.clone()));
                        window.show_result_dialog(EXTRACTION_STOPPED, &text);
                    }
                }
            }
        ));
    }

    /// Shows the extracted folder, or lists its destination again.
    fn show_extracted(&self, folder_uri: &str, choice: &ExtractionChoice, origin: Option<TabId>) {
        if !choice.show_result {
            self.reload_tabs_showing(&choice.destination_uri);
            return;
        }
        let active = self.imp().session.borrow().active_id();
        let opened = if origin.is_some() && origin == active {
            self.navigate(folder_uri)
        } else {
            self.add_tab(folder_uri)
        };
        if let Err(error) = opened {
            self.show_message(&error.to_string());
        }
    }

    /// Extract here: into a new folder beside the archive, named after it,
    /// or `<name> (2)` and so on while a name is taken (ARC-025).
    fn extract_here(&self) {
        let (Some(archive), Some(folder)) = (self.selected_archive(), self.current_uri()) else {
            return;
        };
        if !self.may_start_archive_operation() {
            return;
        }
        let cancel = Cancellation::new();
        self.operation_panel().start(PREPARING, cancel.clone());
        self.update_archive_actions();
        let base_name = ox_core::archive::suggested_folder_name(&archive.name).unwrap_or_default();
        glib::spawn_future_local(glib::clone!(
            #[weak(rename_to = window)]
            self,
            async move {
                let extracted = window
                    .extract_into_free_name(&archive, &folder, &base_name, &cancel)
                    .await;
                window.finish_archive_operation();
                let outcome = extracted.map_err(|error| extraction_failure_text(&error));
                window.report_in_folder(&folder, outcome, EXTRACTION_STOPPED);
            }
        ));
    }

    /// Extracts `archive` into `folder` under the first free name based on
    /// `base_name`; the extractor refuses a taken name without writing.
    async fn extract_into_free_name(
        &self,
        archive: &ArchiveTarget,
        folder: &str,
        base_name: &str,
        cancel: &Cancellation,
    ) -> Result<String, ArchiveError> {
        for name in unique_folder_names(base_name) {
            let request = ExtractionRequest {
                archive_uri: archive.uri.clone(),
                destination_uri: folder.to_owned(),
                folder_name: name,
            };
            let extractor = self
                .zip_extractor()
                .with_progress(self.operation_progress_sender());
            match extractor.extract_in_background(request, cancel.clone()).await {
                Err(ArchiveError::DestinationExists) => {}
                Ok(extracted) => return Ok(extraction_success_text(&extracted)),
                Err(error) => return Err(error),
            }
        }
        Err(ArchiveError::DestinationExists)
    }

    /// Compress to ZIP file: the selection into a new ZIP in the folder,
    /// named after the first item (ARC-023).
    fn compress_selection(&self) {
        let selected = self.folder_pane().model().selected_items();
        let (Some(first), Some(folder)) = (selected.first(), self.current_uri()) else {
            return;
        };
        if !self.may_start_archive_operation() {
            return;
        }
        let uris: Vec<String> = selected.iter().map(|item| item.entry().uri.clone()).collect();
        let first_name = first.entry().name.clone();
        let cancel = Cancellation::new();
        self.operation_panel()
            .start(PREPARING_COMPRESSION, cancel.clone());
        self.update_archive_actions();
        glib::spawn_future_local(glib::clone!(
            #[weak(rename_to = window)]
            self,
            async move {
                let created = window
                    .compress_into_free_name(uris, &folder, &first_name, &cancel)
                    .await;
                window.finish_archive_operation();
                let outcome = created.map_err(|error| compression_failure_text(&error));
                window.report_in_folder(&folder, outcome, COMPRESSION_STOPPED);
            }
        ));
    }

    /// Compresses `uris` into `folder` under the first free ZIP name for
    /// `first_name`; the compressor refuses a taken name without writing.
    async fn compress_into_free_name(
        &self,
        uris: Vec<String>,
        folder: &str,
        first_name: &str,
        cancel: &Cancellation,
    ) -> Result<String, ArchiveError> {
        for archive_name in compressed_file_name(first_name) {
            let request = CompressionRequest {
                uris: uris.clone(),
                destination_uri: folder.to_owned(),
                archive_name,
            };
            let compressor = ZipCompressor::new()
                .with_write_guard(self.context().previous_versions().write_guard())
                .with_progress(self.operation_progress_sender());
            match compressor.compress_in_background(request, cancel.clone()).await {
                Err(ArchiveError::ArchiveExists) => {}
                Ok(created) => return Ok(compression_success_text(&created)),
                Err(error) => return Err(error),
            }
        }
        Err(ArchiveError::ArchiveExists)
    }

    /// Where the worker thread sends progress: to this window's operation
    /// panel, on the main thread.
    pub(super) fn operation_progress_sender(&self) -> impl FnMut(Progress) + Send + 'static {
        let window = glib::SendWeakRef::from(self.downgrade());
        move |progress: Progress| {
            let window = window.clone();
            glib::MainContext::default().invoke(move || {
                if let Some(window) = window.upgrade() {
                    window
                        .operation_panel()
                        .show_progress(&progress.label, progress.fraction);
                }
            });
        }
    }

    /// Lists `folder` again, then says how an operation ended: `outcome`'s
    /// message as a toast, or its failure in the dialog titled
    /// `stopped_title`. Listing a folder hides the toast, so it comes last.
    fn report_in_folder(&self, folder: &str, outcome: Result<String, String>, stopped_title: &str) {
        self.reload_tabs_showing(folder);
        let summary = match &outcome {
            Ok(message) => OperationSummary::Toast(message.clone()),
            Err(failure) => OperationSummary::Report(failure.clone()),
        };
        self.notify_if_in_background(&summary);
        match outcome {
            Ok(message) => self.show_message(&message),
            Err(failure) => self.show_result_dialog(stopped_title, &failure),
        }
    }

    /// Hides the panel and enables the archive commands again.
    pub(super) fn finish_archive_operation(&self) {
        self.operation_panel().finish();
        self.update_archive_actions();
    }
}

/// `entry` as an archive to act on, when it is a ZIP (`isZipEntry`).
fn archive_target(entry: &Entry) -> Option<ArchiveTarget> {
    let is_archive = Activation::for_entry(entry) == Ok(Activation::BrowseArchive);
    is_archive.then(|| ArchiveTarget {
        uri: entry.uri.clone(),
        name: entry.name.clone(),
    })
}

/// True for a location the user may extract into (`writableLocation`):
/// not a page, a server listing or a previous version.
fn is_writable_location(uri: &str, snapshot_roots: &[String]) -> bool {
    let is_page = Page::from_uri(uri).is_some();
    let is_previous_version = snapshot_location(uri, snapshot_roots).is_some();
    !is_page && !is_smb_server(uri) && !is_previous_version
}
